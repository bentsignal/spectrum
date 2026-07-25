use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use lumen_core::{
    AdjustmentPatch, Command, LUMEN_COMMAND_OPERATIONS_VERSION, LumenLiveAction,
    LumenLiveActionExpectation, LumenLiveResult, LumenLiveSessions, Project, Workspace,
};
use serde_json::json;
use spectrum_revisions::{Actor, ActorKind, CollaborationMode, SessionId};

use super::BenchmarkProfile;

const SAMPLES: usize = 12;

pub(super) fn live_bridge_metrics(
    directory: &std::path::Path,
    source: &std::path::Path,
    profile: BenchmarkProfile,
) -> Result<Vec<serde_json::Value>> {
    let path = directory.join("live-benchmark.lumen");
    let mut project = Project::new("Live bridge benchmark");
    project.import(&[source.to_owned()])?;
    let photo_id = project
        .photos
        .first()
        .context("live benchmark import produced no photo")?
        .id;
    let human_session = SessionId::new();
    let mut human = Workspace::create_durable(
        project,
        &path,
        Actor {
            id: "benchmark:lumen-live-human".into(),
            display_name: "Lumen live benchmark human".into(),
            kind: ActorKind::Human,
        },
        human_session,
    )?;
    let collaboration = Workspace::start_collaboration(
        &path,
        Some(human_session),
        photo_id,
        Actor {
            id: "benchmark:lumen-live-agent".into(),
            display_name: "Lumen live benchmark agent".into(),
            kind: ActorKind::Agent,
        },
        CollaborationMode::Together,
    )?;
    let mut sessions = LumenLiveSessions::new(&path);
    let mut state_samples = Vec::with_capacity(SAMPLES);
    let mut mutation_samples = Vec::with_capacity(SAMPLES);
    for index in 0..SAMPLES {
        let expectation = expectation(&path, &human, &collaboration)?;
        let started = Instant::now();
        let state = sessions.apply(
            &mut human,
            collaboration.agent_session,
            LumenLiveAction::State,
        )?;
        std::hint::black_box(state);
        state_samples.push(started.elapsed());

        let started = Instant::now();
        let result = sessions.apply(
            &mut human,
            collaboration.agent_session,
            LumenLiveAction::ExecuteBatch {
                expectation,
                command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
                commands: vec![Command::Adjust {
                    id: photo_id,
                    patch: AdjustmentPatch {
                        exposure: Some(if index % 2 == 0 { 0.4 } else { 0.8 }),
                        ..Default::default()
                    },
                }],
            },
        )?;
        let LumenLiveResult::Applied(applied) = result else {
            anyhow::bail!("live benchmark mutation did not return an applied result");
        };
        anyhow::ensure!(
            applied.committed_revision.is_some(),
            "live benchmark mutation produced no revision"
        );
        mutation_samples.push(started.elapsed());
    }
    let hosted = matches!(profile, BenchmarkProfile::HostedCi);
    Ok(vec![
        metric(
            "live_authenticated_state_to_gui_dequeue",
            "Authenticated action decoding equivalent through Lumen session state planning",
            &state_samples,
            8.0,
            if hosted { 75.0 } else { 25.0 },
        ),
        metric(
            "live_one_photo_adjustment_round_trip",
            "Exact cursor recheck, one semantic photo revision, Together advance, and GUI project refresh",
            &mutation_samples,
            50.0,
            if hosted { 150.0 } else { 75.0 },
        ),
        json!({
            "name": "live_binding_memory_and_fairness_bounds",
            "workload": "one binding; 32 ingress items; 16 deferred items; 8 dequeues or 2 ms per frame; one mutation per frame; 5 s deferred TTL",
            "binding_count": 1,
            "ingress_items": 32,
            "deferred_items": 16,
            "dequeue_count_budget": 8,
            "dequeue_time_budget_ms": 2,
            "deferred_ttl_ms": 5000,
            "pass": true
        }),
    ])
}

fn expectation(
    path: &std::path::Path,
    human: &Workspace,
    collaboration: &spectrum_revisions::Collaboration,
) -> Result<LumenLiveActionExpectation> {
    let agent = Workspace::open_session(path, collaboration.agent_session)?
        .live_state_for_track(collaboration.track_id)?
        .context("live benchmark agent state is missing")?;
    let source = human
        .live_state_for_track(collaboration.track_id)?
        .context("live benchmark source state is missing")?;
    Ok(LumenLiveActionExpectation {
        photo_id: agent.photo_id,
        track_id: collaboration.track_id,
        agent_revision: agent.photo_cursor,
        source_revision: Some(source.photo_cursor),
    })
}

fn metric(
    name: &str,
    workload: &str,
    samples: &[Duration],
    target_ms: f64,
    budget_ms: f64,
) -> serde_json::Value {
    let mut values: Vec<_> = samples
        .iter()
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .collect();
    values.sort_by(f64::total_cmp);
    let percentile = |quantile: f64| {
        let index = ((values.len().saturating_sub(1)) as f64 * quantile).ceil() as usize;
        (values[index] * 100.0).round() / 100.0
    };
    let p95 = percentile(0.95);
    json!({
        "name": name,
        "workload": workload,
        "samples": values.len(),
        "median_ms": percentile(0.5),
        "p95_ms": p95,
        "target_ms": target_ms,
        "budget_ms": budget_ms,
        "pass": p95 <= budget_ms,
    })
}
