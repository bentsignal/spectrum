use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use spectrum_live_bridge::{
    BridgeEventKind, BridgeHost, HostApplyOutcome, InteractionPolicy, ResponseBody,
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

    let error = harness.round_trip_error(
        &mut fixture.human,
        request.clone(),
        PrismLiveInteractionState::Idle,
    );
    assert!(error.to_string().contains("inspect current state"));
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

    let error = harness.round_trip_error(
        &mut fixture.human,
        request.clone(),
        PrismLiveInteractionState::Idle,
    );
    assert!(error.to_string().contains("inspect current state"));
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
    assert!(
        subscription.try_next().unwrap().is_none(),
        "an unknown post-commit outcome must not publish InteractionCanceled"
    );
}
