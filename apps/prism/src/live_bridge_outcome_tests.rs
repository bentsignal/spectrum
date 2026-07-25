use std::{
    fs,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use image::{Rgba, RgbaImage};
use spectrum_imaging::PixelRegion;
use spectrum_live_bridge::{
    BridgeEventKind, BridgeHost, HostApplyOutcome, InteractionPolicy, RequestId, ResponseBody,
};
use spectrum_revisions::CollaborationMode;

use crate::{
    BrushMode, BrushSample, BrushStroke, BrushStyle, Command, LayerKind,
    PRISM_COMMAND_OPERATIONS_VERSION, PaintSelection, PrismLiveAction, PrismLiveInteractionState,
    Workspace,
    live_bridge_sessions::PrismLiveTestFault,
    live_bridge_tests::{Fixture, HostHarness, rename},
};

fn write_live_clone_source(path: &std::path::Path) -> Vec<u8> {
    let mut image = RgbaImage::new(8, 8);
    for y in 0..8 {
        for x in 0..8 {
            image.put_pixel(
                x,
                y,
                Rgba([
                    20 + x as u8 * 20,
                    30 + y as u8 * 18,
                    200 - x as u8 * 10,
                    255,
                ]),
            );
        }
    }
    image.put_pixel(1, 1, Rgba([200, 50, 10, 128]));
    image.save(path).unwrap();
    fs::read(path).unwrap()
}

fn current_clone_stroke(x: f32, y: f32) -> BrushStroke {
    BrushStroke::new(
        BrushStyle {
            mode: BrushMode::Paint,
            color: [0; 4],
            size: 2.0,
            hardness: 1.0,
            opacity: 1.0,
            spacing: 0.25,
        },
        vec![BrushSample {
            x,
            y,
            pressure: 1.0,
        }],
    )
    .unwrap()
    .as_current_clone()
    .unwrap()
}

fn live_clone_setup_commands(source: &std::path::Path) -> Vec<Command> {
    let source = fs::canonicalize(source).unwrap();
    vec![
        Command::AddRaster {
            path: source,
            name: Some("Live Clone source".into()),
            x: 0.0,
            y: 0.0,
        },
        Command::AddPaintLayer {
            name: Some("Live Clone result".into()),
            width: 8,
            height: 8,
        },
        Command::SetCloneSource {
            id: 1,
            document_x: 1.5,
            document_y: 1.5,
            resolved_source: None,
        },
        Command::AddBrushStroke {
            id: 2,
            stroke: current_clone_stroke(4.5, 4.5),
            selection: PaintSelection::None,
        },
    ]
}

fn rendered_live_clone(document: &crate::Document) -> RgbaImage {
    let layer = document.layer(2).unwrap();
    let LayerKind::Paint { program } = &layer.kind else {
        panic!("live Clone batch did not retain its Paint layer")
    };
    crate::paint_render::render_paint_region_with_sources(
        program,
        layer.pixel_mask.as_ref(),
        PixelRegion {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        },
        &document.sampled_sources,
        None,
    )
    .unwrap()
}

#[test]
fn one_coalesced_wake_drains_more_than_one_frame_budget_promptly() {
    let mut fixture = Fixture::new(CollaborationMode::Separate);
    let (wake, awakened) = std::sync::mpsc::sync_channel(1);
    let wake_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let callback_wake_count = Arc::clone(&wake_count);
    let mut harness = HostHarness::new_with_wake(
        &fixture,
        Arc::new(move || {
            callback_wake_count.fetch_add(1, std::sync::atomic::Ordering::Release);
            let _ = wake.try_send(());
        }),
    );
    let requests = (0..12)
        .map(|_| {
            harness.request(
                &fixture,
                PrismLiveAction::State,
                InteractionPolicy::Immediate,
            )
        })
        .collect::<Vec<_>>();
    let workers = requests
        .into_iter()
        .map(|request| {
            let host = Arc::clone(&harness.host);
            thread::spawn(move || host.apply_if_current(&request))
        })
        .collect::<Vec<_>>();

    let deadline = Instant::now() + Duration::from_secs(1);
    while harness.drain.pending_ingress_count() != workers.len() && Instant::now() < deadline {
        thread::yield_now();
    }
    assert_eq!(harness.drain.pending_ingress_count(), workers.len());
    while wake_count.load(std::sync::atomic::Ordering::Acquire) != workers.len()
        && Instant::now() < deadline
    {
        thread::yield_now();
    }
    assert_eq!(
        wake_count.load(std::sync::atomic::Ordering::Acquire),
        workers.len()
    );
    awakened
        .recv_timeout(Duration::from_millis(100))
        .expect("accepted ingress must produce a wake");
    assert!(
        awakened.try_recv().is_err(),
        "wake channel should model egui's coalesced repaint notification"
    );

    let first = harness
        .drain
        .drain(&mut fixture.human, PrismLiveInteractionState::Idle);
    assert!((1..=8).contains(&first.received));
    assert!(
        harness.drain.has_pending(),
        "budget exhaustion must request another repaint"
    );
    let mut frames = 1;
    let mut received = first.received;
    while harness.drain.has_pending() {
        received += harness
            .drain
            .drain(&mut fixture.human, PrismLiveInteractionState::Idle)
            .received;
        frames += 1;
        assert!(frames <= workers.len(), "each repaint must make progress");
    }
    assert_eq!(received, workers.len());
    assert!(frames > 1);
    assert_eq!(harness.drain.pending_ingress_count(), 0);

    for worker in workers {
        assert!(matches!(
            worker.join().unwrap().unwrap(),
            HostApplyOutcome::Applied(ResponseBody::Applied { .. })
        ));
    }
}

#[test]
fn close_drains_accounted_ingress_to_zero_and_rejects_late_admission() {
    let fixture = Fixture::new(CollaborationMode::Separate);
    let mut harness = HostHarness::new(&fixture);
    let request = harness.request(
        &fixture,
        PrismLiveAction::State,
        InteractionPolicy::Immediate,
    );
    let queued = request.clone();
    let host = Arc::clone(&harness.host);
    let worker = thread::spawn(move || host.apply_if_current(&queued));
    let deadline = Instant::now() + Duration::from_secs(1);
    while harness.drain.pending_ingress_count() != 1 && Instant::now() < deadline {
        thread::yield_now();
    }
    assert_eq!(harness.drain.pending_ingress_count(), 1);

    harness.drain.close();
    assert_eq!(harness.drain.pending_ingress_count(), 0);
    assert!(matches!(
        worker.join().unwrap().unwrap(),
        HostApplyOutcome::Applied(ResponseBody::Refused { .. })
    ));
    assert!(harness.host.apply_if_current(&request).is_err());
    assert_eq!(harness.drain.pending_ingress_count(), 0);
}

#[test]
fn precommit_invalid_batch_and_undo_root_are_cached_refusals_without_history_change() {
    let mut fixture = Fixture::new(CollaborationMode::Separate);
    let mut harness = HostHarness::new(&fixture);
    let history_before = Workspace::open_session(&fixture.path, fixture.agent_session)
        .unwrap()
        .history()
        .unwrap()
        .unwrap()
        .revisions
        .len();
    let invalid = harness.request(
        &fixture,
        PrismLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
            commands: vec![crate::Command::SetOpacity {
                id: u64::MAX,
                opacity: 0.5,
            }],
        },
        InteractionPolicy::Immediate,
    );
    let invalid_response = harness.round_trip(
        &mut fixture.human,
        invalid.clone(),
        PrismLiveInteractionState::Idle,
    );
    assert!(matches!(
        invalid_response.body,
        ResponseBody::Refused { .. }
    ));
    assert!(matches!(
        harness.server.handle_request(invalid).unwrap().body,
        ResponseBody::Refused { .. }
    ));

    let undo = harness.request(
        &fixture,
        PrismLiveAction::Undo {
            expectation: fixture.expectation(),
        },
        InteractionPolicy::Immediate,
    );
    let undo_response = harness.round_trip(
        &mut fixture.human,
        undo.clone(),
        PrismLiveInteractionState::Idle,
    );
    assert!(matches!(undo_response.body, ResponseBody::Refused { .. }));
    assert!(matches!(
        harness.server.handle_request(undo).unwrap().body,
        ResponseBody::Refused { .. }
    ));
    assert_eq!(
        Workspace::open_session(&fixture.path, fixture.agent_session)
            .unwrap()
            .history()
            .unwrap()
            .unwrap()
            .revisions
            .len(),
        history_before
    );
}

#[test]
fn post_agent_commit_failure_is_unknown_and_exact_retry_cannot_duplicate_intent() {
    let mut fixture = Fixture::new(CollaborationMode::Separate);
    let mut harness = HostHarness::new(&fixture);
    let history_before = Workspace::open_session(&fixture.path, fixture.agent_session)
        .unwrap()
        .history()
        .unwrap()
        .unwrap()
        .revisions
        .len();
    let request = harness.request(
        &fixture,
        PrismLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
            commands: vec![rename("Committed before fault")],
        },
        InteractionPolicy::Immediate,
    );
    harness
        .drain
        .inject_session_fault(PrismLiveTestFault::AfterAgentMutation);

    let (error, report) = harness.round_trip_error(
        &mut fixture.human,
        request.clone(),
        PrismLiveInteractionState::Idle,
    );
    assert!(error.to_string().contains("inspect current state"));
    assert!(report.outcome_unknown);
    assert!(!report.workspace_changed);
    let agent = Workspace::open_session(&fixture.path, fixture.agent_session).unwrap();
    assert_eq!(agent.document.name, "Committed before fault");
    assert_eq!(
        agent.history().unwrap().unwrap().revisions.len(),
        history_before + 1
    );

    let retry = harness.server.handle_request(request).unwrap();
    assert!(matches!(retry.body, ResponseBody::OutcomeUnknown));
    let inspect = harness.request(
        &fixture,
        PrismLiveAction::State,
        InteractionPolicy::Immediate,
    );
    assert!(matches!(
        harness
            .round_trip(&mut fixture.human, inspect, PrismLiveInteractionState::Idle)
            .body,
        ResponseBody::Applied { .. }
    ));
    assert_eq!(
        Workspace::open_session(&fixture.path, fixture.agent_session)
            .unwrap()
            .history()
            .unwrap()
            .unwrap()
            .revisions
            .len(),
        history_before + 1
    );
}

#[test]
fn post_human_sync_failure_is_unknown_with_both_committed_cursors_observable() {
    let mut fixture = Fixture::new(CollaborationMode::Together);
    let mut harness = HostHarness::new(&fixture);
    let history_before = fixture.human.history().unwrap().unwrap().revisions.len();
    let request = harness.request(
        &fixture,
        PrismLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
            commands: vec![rename("Synced before fault")],
        },
        InteractionPolicy::Immediate,
    );
    harness
        .drain
        .inject_session_fault(PrismLiveTestFault::AfterHumanSync);

    let (error, report) = harness.round_trip_error(
        &mut fixture.human,
        request.clone(),
        PrismLiveInteractionState::Idle,
    );
    assert!(error.to_string().contains("inspect current state"));
    assert!(report.outcome_unknown);
    assert!(report.workspace_changed);
    assert_eq!(fixture.human.document.name, "Synced before fault");
    let agent = Workspace::open_session(&fixture.path, fixture.agent_session).unwrap();
    assert_eq!(agent.document.name, "Synced before fault");
    assert_eq!(
        fixture.human.history().unwrap().unwrap().revisions.len(),
        history_before + 1
    );
    assert_eq!(
        fixture.human.live_state().unwrap().unwrap().cursor,
        agent.live_state().unwrap().unwrap().cursor
    );
    assert!(matches!(
        harness.server.handle_request(request).unwrap().body,
        ResponseBody::OutcomeUnknown
    ));
}

#[test]
fn together_sync_commit_fault_reopens_the_human_workspace_before_reporting_unknown() {
    let mut fixture = Fixture::new(CollaborationMode::Together);
    let mut harness = HostHarness::new(&fixture);
    let request = harness.request(
        &fixture,
        PrismLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
            commands: vec![rename("Recovered Together commit")],
        },
        InteractionPolicy::Immediate,
    );
    fixture.human.fail_next_together_sync_after_durable_commit();

    let (error, report) =
        harness.round_trip_error(&mut fixture.human, request, PrismLiveInteractionState::Idle);
    assert!(error.to_string().contains("inspect current state"));
    assert!(report.outcome_unknown);
    assert!(report.workspace_changed);
    assert_eq!(fixture.human.document.name, "Recovered Together commit");
    let agent = Workspace::open_session(&fixture.path, fixture.agent_session).unwrap();
    let human_state = fixture.human.live_state().unwrap().unwrap();
    let agent_state = agent.live_state().unwrap().unwrap();
    assert_eq!(human_state.cursor, agent_state.cursor);
    let collaboration = Workspace::collaboration(&fixture.path, fixture.agent_session).unwrap();
    assert_eq!(collaboration.followed_revision, human_state.cursor);
    assert!(matches!(
        fixture.human.sync_together().unwrap(),
        spectrum_revisions::CollaborationSync::Waiting(_)
    ));
    assert_eq!(fixture.human.document.name, "Recovered Together commit");
}

#[test]
fn failed_together_recovery_poisons_every_stale_workspace_entry_until_project_reopen() {
    let mut fixture = Fixture::new(CollaborationMode::Together);
    let mut harness = HostHarness::new(&fixture);
    let human_session = fixture.human.session_id().unwrap();
    let root = fixture
        .human
        .history()
        .unwrap()
        .unwrap()
        .revisions
        .first()
        .unwrap()
        .id;
    let request = harness.request(
        &fixture,
        PrismLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
            commands: vec![rename("Durable but unavailable")],
        },
        InteractionPolicy::Immediate,
    );
    fixture.human.fail_next_together_sync_after_durable_commit();
    fixture.human.fail_next_together_recovery_open();

    let (error, report) = harness.round_trip_error(
        &mut fixture.human,
        request.clone(),
        PrismLiveInteractionState::Idle,
    );
    assert!(error.to_string().contains("inspect current state"));
    assert!(report.outcome_unknown);
    assert!(report.workspace_changed);
    assert!(report.reopen_required);
    let stale_document = fixture.human.document.clone();
    let poisoned_generation = fixture.human.document_generation();
    let reopen_error = |error: anyhow::Error| {
        assert!(
            error.to_string().contains("reopen the project"),
            "unexpected poison error: {error:#}"
        );
    };
    reopen_error(fixture.human.live_state().unwrap_err());
    reopen_error(fixture.human.sync_together().unwrap_err());
    reopen_error(fixture.human.history().unwrap_err());
    reopen_error(
        fixture
            .human
            .execute(rename("Stale direct edit"))
            .unwrap_err(),
    );
    reopen_error(
        fixture
            .human
            .execute_batch(vec![rename("Stale direct batch")])
            .unwrap_err(),
    );
    reopen_error(fixture.human.move_to_revision(root).unwrap_err());
    reopen_error(fixture.human.save(None).unwrap_err());
    reopen_error(fixture.human.checkpoint().unwrap_err());
    reopen_error(fixture.human.begin_interaction().unwrap_err());
    reopen_error(
        fixture
            .human
            .preview(rename("Stale interaction preview"))
            .unwrap_err(),
    );
    reopen_error(fixture.human.commit_interaction().unwrap_err());
    let moved = fixture.path.with_file_name("must-not-move.prism");
    reopen_error(fixture.human.move_project(&moved).unwrap_err());
    assert!(!moved.exists());
    assert!(!fixture.human.can_undo());
    assert!(!fixture.human.can_redo());
    assert!(!fixture.human.interaction_active());
    assert_eq!(fixture.human.document, stale_document);
    assert_eq!(
        fixture.human.document_generation(),
        poisoned_generation,
        "refused stale workspace operations must not mutate GUI-visible state"
    );

    let mut read = request.clone();
    read.request_id = RequestId::new();
    read.action.action = serde_json::to_value(PrismLiveAction::State).unwrap();
    let read_response =
        harness.round_trip(&mut fixture.human, read, PrismLiveInteractionState::Idle);
    assert!(matches!(read_response.body, ResponseBody::Refused { .. }));

    let mut mutation = request;
    mutation.request_id = RequestId::new();
    let mutation_response = harness.round_trip(
        &mut fixture.human,
        mutation,
        PrismLiveInteractionState::Idle,
    );
    assert!(matches!(
        mutation_response.body,
        ResponseBody::Refused { .. }
    ));
    assert_ne!(fixture.human.document.name, "Durable but unavailable");
    assert_eq!(
        Workspace::open_session(&fixture.path, human_session)
            .unwrap()
            .document
            .name,
        "Durable but unavailable"
    );
}

#[test]
fn required_live_clone_batch_is_one_v14_revision_with_embedded_follower_parity() {
    let mut fixture = Fixture::new(CollaborationMode::Together);
    let source = fixture.path.with_file_name("live-clone-source.png");
    let source_bytes = write_live_clone_source(&source);
    let human_session = fixture.human.session_id().unwrap();
    let history_before = fixture.human.history().unwrap().unwrap().revisions.len();
    let mut harness = HostHarness::new(&fixture);
    let request = harness.request(
        &fixture,
        PrismLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
            commands: live_clone_setup_commands(&source),
        },
        InteractionPolicy::Immediate,
    );

    let response = harness.round_trip(&mut fixture.human, request, PrismLiveInteractionState::Idle);
    assert!(
        matches!(response.body, ResponseBody::Applied { .. }),
        "unexpected live Clone response: {:?}",
        response.body
    );
    let history = fixture.human.history().unwrap().unwrap();
    assert_eq!(history.revisions.len(), history_before + 1);
    assert_eq!(history.revisions.last().unwrap().command_count, 4);
    assert_eq!(PRISM_COMMAND_OPERATIONS_VERSION, 15);

    let canonical = rusqlite::Connection::open(&fixture.path).unwrap();
    let operation_version: i64 = canonical
        .query_row(
            "SELECT version FROM operation_payloads
             WHERE instr(CAST(bytes AS TEXT), 'set_clone_source') > 0
             LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(operation_version, 14);
    let embedded_source: Vec<u8> = canonical
        .query_row("SELECT bytes FROM assets", [], |row| row.get(0))
        .unwrap();
    assert_eq!(embedded_source, source_bytes);
    let asset_count: i64 = canonical
        .query_row("SELECT count(*) FROM assets", [], |row| row.get(0))
        .unwrap();
    assert_eq!(asset_count, 1);
    drop(canonical);
    fs::remove_file(&source).unwrap();

    let agent = Workspace::open_session(&fixture.path, fixture.agent_session).unwrap();
    let reopened = Workspace::open_session(&fixture.path, human_session).unwrap();
    let human_state = fixture.human.live_state().unwrap().unwrap();
    assert_eq!(
        human_state.cursor,
        agent.live_state().unwrap().unwrap().cursor
    );
    assert_eq!(
        human_state.cursor,
        reopened.live_state().unwrap().unwrap().cursor
    );
    assert_eq!(fixture.human.document.sampled_sources.len(), 1);
    assert_eq!(agent.document.sampled_sources.len(), 1);
    assert_eq!(reopened.document.sampled_sources.len(), 1);
    let human_pixels = rendered_live_clone(&fixture.human.document);
    assert_eq!(rendered_live_clone(&agent.document), human_pixels);
    assert_eq!(rendered_live_clone(&reopened.document), human_pixels);
    assert!(
        human_pixels.pixels().any(|pixel| pixel.0[3] != 0),
        "the live Clone batch must produce visible sampled pixels"
    );
}

#[test]
fn failed_live_clone_follow_recovery_poison_blocks_exact_and_new_retries() {
    let mut fixture = Fixture::new(CollaborationMode::Together);
    let source = fixture
        .path
        .with_file_name("poisoned-live-clone-source.png");
    write_live_clone_source(&source);
    let mut setup_harness = HostHarness::new(&fixture);
    let setup = setup_harness.request(
        &fixture,
        PrismLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
            commands: live_clone_setup_commands(&source),
        },
        InteractionPolicy::Immediate,
    );
    let setup_response =
        setup_harness.round_trip(&mut fixture.human, setup, PrismLiveInteractionState::Idle);
    assert!(
        matches!(setup_response.body, ResponseBody::Applied { .. }),
        "unexpected live Clone setup response: {:?}",
        setup_response.body
    );
    fs::remove_file(source).unwrap();

    let history_before = Workspace::open_session(&fixture.path, fixture.agent_session)
        .unwrap()
        .history()
        .unwrap()
        .unwrap()
        .revisions
        .len();
    let mut harness = HostHarness::new(&fixture);
    let request = harness.request(
        &fixture,
        PrismLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
            commands: vec![Command::AddBrushStroke {
                id: 2,
                stroke: current_clone_stroke(6.5, 6.5),
                selection: PaintSelection::None,
            }],
        },
        InteractionPolicy::Immediate,
    );
    fixture.human.fail_next_together_sync_after_durable_commit();
    fixture.human.fail_next_together_recovery_open();

    let (error, report) = harness.round_trip_error(
        &mut fixture.human,
        request.clone(),
        PrismLiveInteractionState::Idle,
    );
    assert!(error.to_string().contains("inspect current state"));
    assert!(report.outcome_unknown);
    assert!(report.reopen_required);
    assert!(matches!(
        harness.server.handle_request(request.clone()).unwrap().body,
        ResponseBody::OutcomeUnknown
    ));

    let mut new_retry = request.clone();
    new_retry.request_id = RequestId::new();
    assert!(matches!(
        harness
            .round_trip(
                &mut fixture.human,
                new_retry,
                PrismLiveInteractionState::Idle
            )
            .body,
        ResponseBody::Refused { .. }
    ));
    let direct_retry = fixture
        .human
        .execute(Command::AddBrushStroke {
            id: 2,
            stroke: current_clone_stroke(6.5, 6.5),
            selection: PaintSelection::None,
        })
        .unwrap_err();
    assert!(direct_retry.to_string().contains("reopen the project"));

    let agent = Workspace::open_session(&fixture.path, fixture.agent_session).unwrap();
    assert_eq!(
        agent.history().unwrap().unwrap().revisions.len(),
        history_before + 1,
        "the unknown Clone intent must commit exactly once"
    );
    let reopened = Workspace::open(&fixture.path).unwrap();
    assert_eq!(
        rendered_live_clone(&reopened.document),
        rendered_live_clone(&agent.document)
    );
}

#[test]
fn deferred_post_commit_unknown_never_claims_the_interaction_was_canceled() {
    let mut fixture = Fixture::new(CollaborationMode::Separate);
    let mut harness = HostHarness::new(&fixture);
    let subscription = harness.server.events().subscribe(0).unwrap();
    let request = harness.request(
        &fixture,
        PrismLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
            commands: vec![rename("Deferred commit before fault")],
        },
        InteractionPolicy::Deferred,
    );
    harness
        .drain
        .inject_session_fault(PrismLiveTestFault::AfterAgentMutation);
    let response = harness.round_trip(
        &mut fixture.human,
        request,
        PrismLiveInteractionState::Active,
    );
    assert!(matches!(response.body, ResponseBody::Deferred));
    assert!(matches!(
        subscription.try_next().unwrap().unwrap().event,
        BridgeEventKind::InteractionBegan { .. }
    ));

    let report = harness
        .drain
        .drain(&mut fixture.human, PrismLiveInteractionState::Idle);
    assert_eq!(report.applied, 0);
    assert_eq!(report.refused, 1);
    assert_eq!(
        Workspace::open_session(&fixture.path, fixture.agent_session)
            .unwrap()
            .document
            .name,
        "Deferred commit before fault"
    );
    let inspect = harness.request(
        &fixture,
        PrismLiveAction::State,
        InteractionPolicy::Immediate,
    );
    assert!(matches!(
        harness
            .round_trip(&mut fixture.human, inspect, PrismLiveInteractionState::Idle)
            .body,
        ResponseBody::Applied { .. }
    ));
    assert!(matches!(
        subscription.try_next().unwrap().unwrap().event,
        BridgeEventKind::InteractionOutcomeUnknown { .. }
    ));
    assert!(
        subscription.try_next().unwrap().is_none(),
        "deferred interaction must have exactly one terminal event"
    );
}
