use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use spectrum_live_bridge::{
    BridgeEventKind, BridgeHost, HostApplyOutcome, InteractionPolicy, RequestId, ResponseBody,
};
use spectrum_revisions::CollaborationMode;

use crate::{
    PRISM_COMMAND_OPERATIONS_VERSION, PrismLiveAction, PrismLiveInteractionState, Workspace,
    live_bridge_sessions::PrismLiveTestFault,
    live_bridge_tests::{Fixture, HostHarness, rename},
};

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
fn failed_together_recovery_poisons_live_reads_and_mutations_until_project_reopen() {
    let mut fixture = Fixture::new(CollaborationMode::Together);
    let mut harness = HostHarness::new(&fixture);
    let human_session = fixture.human.session_id().unwrap();
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
    assert!(
        fixture
            .human
            .live_state()
            .unwrap_err()
            .to_string()
            .contains("reopen the project")
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
