use std::{
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use lumen_core::{
    AdjustmentPatch, Command, LUMEN_COMMAND_OPERATIONS_VERSION, LUMEN_LIVE_ACTION_FAMILY,
    LUMEN_LIVE_ACTION_VERSION, LUMEN_LIVE_APPLICATION, LUMEN_LIVE_DEFERRED_TTL,
    LUMEN_LIVE_DRAIN_COUNT_BUDGET, LUMEN_LIVE_DRAIN_TIME_BUDGET, LUMEN_LIVE_INGRESS_CAPACITY,
    LUMEN_LIVE_MAX_DEFERRED, LumenLiveAction, LumenLiveActionExpectation, LumenLiveHost,
    LumenLiveInteractionState, LumenLiveResult, LumenLiveSessions, Project, Workspace,
};
use serde_json::json;
use spectrum_live_bridge::{
    ActionEnvelope, BindingId, BridgeError, BridgeHost, BridgeResult, EventLog, HostApplyOutcome,
    InteractionPolicy, PROTOCOL_FAMILY, PROTOCOL_VERSION, RequestEnvelope, RequestId, ResponseBody,
};
use spectrum_revisions::{Actor, ActorKind, Collaboration, CollaborationMode, SessionId};

use super::BenchmarkProfile;

const SAMPLES: usize = 12;
const STATE_TARGET_MS: f64 = 25.0;
// Eleven independent workstation runs measured 16.43–28.94 ms p95. Keep the
// 25 ms feel target visible and round the smallest evidence-backed regression
// ceiling up to 35 ms; this is deliberately not extra hosted-runner headroom.
const STATE_INTERACTIVE_BUDGET_MS: f64 = 35.0;
const RETAINED_REQUEST_BUDGET_BYTES: usize = 1024 * 1024;

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
    let bounds_metric = live_host_bounds_metric(&mut human, &collaboration)?;
    let hosted = matches!(profile, BenchmarkProfile::HostedCi);
    Ok(vec![
        metric(
            "live_authenticated_state_to_gui_dequeue",
            "Authenticated action decoding equivalent through Lumen session state planning",
            &state_samples,
            STATE_TARGET_MS,
            if hosted {
                75.0
            } else {
                STATE_INTERACTIVE_BUDGET_MS
            },
        ),
        metric(
            "live_one_photo_adjustment_round_trip",
            "Exact cursor recheck, one semantic photo revision, Together advance, and GUI project refresh",
            &mutation_samples,
            50.0,
            if hosted { 150.0 } else { 75.0 },
        ),
        bounds_metric,
    ])
}

fn live_host_bounds_metric(
    workspace: &mut Workspace,
    collaboration: &Collaboration,
) -> Result<serde_json::Value> {
    let (host, mut drain) = LumenLiveHost::new(workspace, BindingId::new(), 1)?;
    host.attach_events(Arc::new(EventLog::new()))?;

    let ingress_attempts = LUMEN_LIVE_INGRESS_CAPACITY + 1;
    let ingress_workers = (0..ingress_attempts)
        .map(|_| {
            Ok(spawn_request(
                Arc::clone(&host),
                host_request(
                    workspace,
                    collaboration,
                    &host,
                    InteractionPolicy::Immediate,
                    LumenLiveAction::State,
                )?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    wait_for_saturated_ingress(&drain, &ingress_workers)?;
    let mut retained_request_bytes = host.retained_request_bytes();
    let mut max_received_per_drain = 0;
    let mut max_drain_elapsed = Duration::ZERO;
    let mut receive_start_within_time_budget = true;
    while drain.pending_ingress_count() > 0 {
        let started = Instant::now();
        let report = drain.drain(workspace, LumenLiveInteractionState::Idle);
        max_drain_elapsed = max_drain_elapsed.max(started.elapsed());
        max_received_per_drain = max_received_per_drain.max(report.received);
        receive_start_within_time_budget &= report.received <= 1
            || report.last_receive_started_after < LUMEN_LIVE_DRAIN_TIME_BUDGET;
    }
    let mut ingress_applied = 0;
    let mut ingress_rate_limited = 0;
    for worker in ingress_workers {
        match join_request(worker)? {
            Ok(HostApplyOutcome::Applied(ResponseBody::Applied { .. })) => ingress_applied += 1,
            Err(BridgeError::RateLimited { .. }) => ingress_rate_limited += 1,
            _ => anyhow::bail!("unexpected saturated ingress outcome"),
        }
    }

    let live_expectation = expectation(
        workspace
            .catalog_path
            .as_deref()
            .context("live benchmark workspace path is missing")?,
        workspace,
        collaboration,
    )?;
    let deferred_action = LumenLiveAction::ExecuteBatch {
        expectation: live_expectation.clone(),
        command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
        commands: vec![Command::Adjust {
            id: live_expectation.photo_id,
            patch: AdjustmentPatch {
                exposure: Some(0.25),
                ..Default::default()
            },
        }],
    };
    let deferred_workers = (0..LUMEN_LIVE_MAX_DEFERRED)
        .map(|_| {
            Ok(spawn_request(
                Arc::clone(&host),
                host_request(
                    workspace,
                    collaboration,
                    &host,
                    InteractionPolicy::Deferred,
                    deferred_action.clone(),
                )?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    wait_for_ingress(&drain, LUMEN_LIVE_MAX_DEFERRED)?;
    let mut deferred_accepted = 0;
    while drain.pending_ingress_count() > 0 {
        let started = Instant::now();
        let report = drain.drain(workspace, LumenLiveInteractionState::Active);
        max_drain_elapsed = max_drain_elapsed.max(started.elapsed());
        max_received_per_drain = max_received_per_drain.max(report.received);
        receive_start_within_time_budget &= report.received <= 1
            || report.last_receive_started_after < LUMEN_LIVE_DRAIN_TIME_BUDGET;
        deferred_accepted += report.deferred;
    }
    for worker in deferred_workers {
        if !matches!(
            join_request(worker)?,
            Ok(HostApplyOutcome::Applied(ResponseBody::Deferred))
        ) {
            anyhow::bail!("saturated deferred request was not acknowledged as Deferred");
        }
    }
    retained_request_bytes = retained_request_bytes.max(host.retained_request_bytes());

    let overflow = spawn_request(
        Arc::clone(&host),
        host_request(
            workspace,
            collaboration,
            &host,
            InteractionPolicy::Deferred,
            deferred_action,
        )?,
    );
    let overflow_report =
        drain_until_received(&mut drain, workspace, LumenLiveInteractionState::Active)?;
    max_received_per_drain = max_received_per_drain.max(overflow_report.received);
    receive_start_within_time_budget &= overflow_report.received <= 1
        || overflow_report.last_receive_started_after < LUMEN_LIVE_DRAIN_TIME_BUDGET;
    let deferred_overflow_refused = matches!(
        join_request(overflow)?,
        Ok(HostApplyOutcome::Applied(ResponseBody::Refused { .. }))
    ) && overflow_report.refused == 1;

    let ttl_started = Instant::now();
    thread::sleep(LUMEN_LIVE_DEFERRED_TTL + Duration::from_millis(10));
    let expired_report = drain.drain(workspace, LumenLiveInteractionState::Active);
    let ttl_elapsed = ttl_started.elapsed();
    let deferred_expired = expired_report.refused;

    let one_per_frame_action = LumenLiveAction::ExecuteBatch {
        expectation: live_expectation.clone(),
        command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
        commands: vec![Command::Adjust {
            id: live_expectation.photo_id,
            patch: AdjustmentPatch {
                contrast: Some(1.0),
                ..Default::default()
            },
        }],
    };
    let release_workers = (0..2)
        .map(|_| {
            Ok(spawn_request(
                Arc::clone(&host),
                host_request(
                    workspace,
                    collaboration,
                    &host,
                    InteractionPolicy::Deferred,
                    one_per_frame_action.clone(),
                )?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    wait_for_ingress(&drain, 2)?;
    let mut queued_deferred = 0;
    while drain.pending_ingress_count() > 0 {
        let report = drain.drain(workspace, LumenLiveInteractionState::Active);
        max_received_per_drain = max_received_per_drain.max(report.received);
        receive_start_within_time_budget &= report.received <= 1
            || report.last_receive_started_after < LUMEN_LIVE_DRAIN_TIME_BUDGET;
        queued_deferred += report.deferred;
    }
    anyhow::ensure!(queued_deferred == 2, "two release probes were not deferred");
    for worker in release_workers {
        anyhow::ensure!(
            matches!(
                join_request(worker)?,
                Ok(HostApplyOutcome::Applied(ResponseBody::Deferred))
            ),
            "release probe was not acknowledged as Deferred"
        );
    }
    let before_first = drain.deferred_count();
    let first_release = drain.drain(workspace, LumenLiveInteractionState::Idle);
    let after_first = drain.deferred_count();
    let second_release = drain.drain(workspace, LumenLiveInteractionState::Idle);
    let after_second = drain.deferred_count();
    let max_released_per_idle_drain = (before_first - after_first).max(after_first - after_second);
    let released_one_per_idle_drain = before_first == 2
        && after_first == 1
        && after_second == 0
        && first_release.received == 0
        && second_release.received == 0;
    let retained_after_drain = host.retained_request_bytes();
    drain.close();

    let max_drain_elapsed_ms = max_drain_elapsed.as_secs_f64() * 1000.0;
    let pass = ingress_applied == LUMEN_LIVE_INGRESS_CAPACITY
        && ingress_rate_limited == 1
        && max_received_per_drain <= LUMEN_LIVE_DRAIN_COUNT_BUDGET
        && receive_start_within_time_budget
        && deferred_accepted == LUMEN_LIVE_MAX_DEFERRED
        && drain.deferred_count() == 0
        && deferred_overflow_refused
        && deferred_expired == LUMEN_LIVE_MAX_DEFERRED
        && ttl_elapsed >= LUMEN_LIVE_DEFERRED_TTL
        && released_one_per_idle_drain
        && max_released_per_idle_drain == 1
        && retained_request_bytes <= RETAINED_REQUEST_BUDGET_BYTES
        && retained_after_drain == 0;
    Ok(json!({
        "name": "live_binding_memory_and_fairness_bounds",
        "workload": "real Lumen host: saturated ingress and deferred queues, TTL expiry, and idle-frame release",
        "ingress_capacity": LUMEN_LIVE_INGRESS_CAPACITY,
        "ingress_attempts": ingress_attempts,
        "ingress_applied": ingress_applied,
        "ingress_rate_limited": ingress_rate_limited,
        "deferred_capacity": LUMEN_LIVE_MAX_DEFERRED,
        "deferred_accepted": deferred_accepted,
        "deferred_overflow_refused": deferred_overflow_refused,
        "deferred_expired": deferred_expired,
        "deferred_ttl_ms": LUMEN_LIVE_DEFERRED_TTL.as_millis(),
        "ttl_observed_ms": (ttl_elapsed.as_secs_f64() * 1000.0 * 100.0).round() / 100.0,
        "max_received_per_drain": max_received_per_drain,
        "dequeue_count_budget": LUMEN_LIVE_DRAIN_COUNT_BUDGET,
        "dequeue_time_budget_ms": LUMEN_LIVE_DRAIN_TIME_BUDGET.as_millis(),
        "receive_start_within_time_budget": receive_start_within_time_budget,
        "max_drain_elapsed_ms": (max_drain_elapsed_ms * 100.0).round() / 100.0,
        "max_released_per_idle_drain": max_released_per_idle_drain,
        "retained_request_bytes": retained_request_bytes,
        "retained_request_budget_bytes": RETAINED_REQUEST_BUDGET_BYTES,
        "retained_request_bytes_after_drain": retained_after_drain,
        "pass": pass,
    }))
}

type RequestWorker = JoinHandle<BridgeResult<HostApplyOutcome>>;

fn spawn_request(host: Arc<LumenLiveHost>, request: RequestEnvelope) -> RequestWorker {
    thread::spawn(move || host.apply_if_current(&request))
}

fn join_request(worker: RequestWorker) -> Result<BridgeResult<HostApplyOutcome>> {
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("live host benchmark request worker panicked"))
}

fn wait_for_ingress(drain: &lumen_core::LumenLiveDrain, expected: usize) -> Result<()> {
    let started = Instant::now();
    while drain.pending_ingress_count() < expected {
        if started.elapsed() >= Duration::from_secs(2) {
            anyhow::bail!(
                "live host benchmark ingress reached {}, expected {expected}",
                drain.pending_ingress_count()
            );
        }
        thread::yield_now();
    }
    Ok(())
}

fn wait_for_saturated_ingress(
    drain: &lumen_core::LumenLiveDrain,
    workers: &[RequestWorker],
) -> Result<()> {
    let started = Instant::now();
    while drain.pending_ingress_count() != LUMEN_LIVE_INGRESS_CAPACITY
        || !workers.iter().any(JoinHandle::is_finished)
    {
        if started.elapsed() >= Duration::from_secs(2) {
            anyhow::bail!(
                "live host saturation did not settle at {} queued requests and one completed overflow",
                LUMEN_LIVE_INGRESS_CAPACITY
            );
        }
        thread::yield_now();
    }
    Ok(())
}

fn drain_until_received(
    drain: &mut lumen_core::LumenLiveDrain,
    workspace: &mut Workspace,
    interaction: LumenLiveInteractionState,
) -> Result<lumen_core::LumenLiveDrainReport> {
    let started = Instant::now();
    loop {
        let report = drain.drain(workspace, interaction);
        if report.received > 0 {
            return Ok(report);
        }
        if started.elapsed() >= Duration::from_secs(2) {
            anyhow::bail!("live host benchmark request never reached the drain");
        }
        thread::yield_now();
    }
}

fn host_request(
    workspace: &Workspace,
    collaboration: &Collaboration,
    host: &LumenLiveHost,
    interaction: InteractionPolicy,
    action: LumenLiveAction,
) -> Result<RequestEnvelope> {
    let (project_id, _, _, _) = workspace
        .live_catalog_identity()
        .context("live host benchmark workspace is not durable")?;
    Ok(RequestEnvelope {
        protocol: PROTOCOL_FAMILY.into(),
        version: PROTOCOL_VERSION,
        request_id: RequestId::new(),
        binding_id: host.binding_id(),
        binding_epoch: 1,
        project_id,
        application: LUMEN_LIVE_APPLICATION.into(),
        session_id: collaboration.agent_session,
        expected_cursors: host.current_cursors()?,
        actor_label: "Lumen live benchmark".into(),
        interaction,
        action: ActionEnvelope {
            family: LUMEN_LIVE_ACTION_FAMILY.into(),
            version: LUMEN_LIVE_ACTION_VERSION,
            capabilities: Vec::new(),
            action: serde_json::to_value(action)?,
        },
    })
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
