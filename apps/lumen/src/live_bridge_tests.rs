use image::{Rgba, RgbaImage};
use std::{sync::Arc, thread, time::Duration};

use spectrum_live_bridge::{
    ActionEnvelope, BindingId, BridgeError, BridgeEventKind, BridgeHost, EventLog,
    HostApplyOutcome, InteractionPolicy, PROTOCOL_FAMILY, PROTOCOL_VERSION, RequestEnvelope,
    RequestId, ResponseBody,
};
use spectrum_revisions::{Actor, ActorKind, CollaborationMode, SessionId};

use crate::{
    AdjustmentPatch, Command, LUMEN_COMMAND_OPERATIONS_VERSION, LUMEN_LIVE_ACTION_FAMILY,
    LUMEN_LIVE_ACTION_VERSION, LUMEN_LIVE_APPLICATION, LumenLiveAction, LumenLiveActionExpectation,
    LumenLiveApplyError, LumenLiveHost, LumenLiveInteractionState, LumenLiveResult,
    LumenLiveSessions, LumenLiveTestFault, Project, Workspace,
};

struct Fixture {
    _directory: tempfile::TempDir,
    path: std::path::PathBuf,
    human: Workspace,
    collaboration: spectrum_revisions::Collaboration,
    photo_id: u64,
}

fn request(
    fixture: &Fixture,
    host: &LumenLiveHost,
    interaction: InteractionPolicy,
    action: LumenLiveAction,
) -> RequestEnvelope {
    let (project_id, _, _, _) = fixture.human.live_catalog_identity().unwrap();
    RequestEnvelope {
        protocol: PROTOCOL_FAMILY.into(),
        version: PROTOCOL_VERSION,
        request_id: RequestId::new(),
        binding_id: host.binding_id(),
        binding_epoch: 1,
        project_id,
        application: LUMEN_LIVE_APPLICATION.into(),
        session_id: fixture.collaboration.agent_session,
        expected_cursors: host.current_cursors().unwrap(),
        actor_label: "Test Agent".into(),
        interaction,
        action: ActionEnvelope {
            family: LUMEN_LIVE_ACTION_FAMILY.into(),
            version: LUMEN_LIVE_ACTION_VERSION,
            capabilities: Vec::new(),
            action: serde_json::to_value(action).unwrap(),
        },
    }
}

impl Fixture {
    fn new(mode: CollaborationMode) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.png");
        RgbaImage::from_pixel(4, 4, Rgba([30, 80, 140, 255]))
            .save(&source)
            .unwrap();
        let mut project = Project::new("Live fixture");
        project.import(&[source]).unwrap();
        let photo_id = project.photos[0].id;
        let path = directory.path().join("fixture.lumen");
        let human_session = SessionId::new();
        let human = Workspace::create_durable(
            project,
            &path,
            actor("person:test", "Test User", ActorKind::Human),
            human_session,
        )
        .unwrap();
        let collaboration = Workspace::start_collaboration(
            &path,
            Some(human_session),
            photo_id,
            actor("agent:test", "Test Agent", ActorKind::Agent),
            mode,
        )
        .unwrap();
        Self {
            _directory: directory,
            path,
            human,
            collaboration,
            photo_id,
        }
    }

    fn expectation(&self) -> LumenLiveActionExpectation {
        let agent = Workspace::open_session(&self.path, self.collaboration.agent_session).unwrap();
        let agent = agent
            .live_state_for_track(self.collaboration.track_id)
            .unwrap()
            .unwrap();
        let human = self
            .human
            .live_state_for_track(self.collaboration.track_id)
            .unwrap()
            .unwrap();
        LumenLiveActionExpectation {
            photo_id: self.photo_id,
            track_id: self.collaboration.track_id,
            agent_revision: agent.photo_cursor,
            source_revision: (self.collaboration.mode == CollaborationMode::Together)
                .then_some(human.photo_cursor),
        }
    }
}

fn actor(id: &str, name: &str, kind: ActorKind) -> Actor {
    Actor {
        id: id.into(),
        display_name: name.into(),
        kind,
    }
}

fn wait_for_ingress(drain: &crate::LumenLiveDrain) {
    for _ in 0..100 {
        if drain.pending_ingress_count() > 0 {
            return;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("live request did not reach the GUI ingress queue");
}

#[test]
fn together_batch_is_one_exactly_counted_revision_and_updates_the_human_photo() {
    let mut fixture = Fixture::new(CollaborationMode::Together);
    let mut sessions = LumenLiveSessions::new(&fixture.path);
    let expectation = fixture.expectation();
    let result = sessions
        .apply(
            &mut fixture.human,
            fixture.collaboration.agent_session,
            LumenLiveAction::ExecuteBatch {
                expectation,
                command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
                commands: vec![
                    Command::Adjust {
                        id: fixture.photo_id,
                        patch: AdjustmentPatch {
                            exposure: Some(1.25),
                            ..Default::default()
                        },
                    },
                    Command::Adjust {
                        id: fixture.photo_id,
                        patch: AdjustmentPatch {
                            contrast: Some(18.0),
                            ..Default::default()
                        },
                    },
                ],
            },
        )
        .unwrap();
    let LumenLiveResult::Applied(applied) = result else {
        panic!("expected applied live batch");
    };
    let revision = applied.committed_revision.unwrap();
    assert_eq!(revision.command_count, 2);
    assert_eq!(
        fixture
            .human
            .project
            .photo(fixture.photo_id)
            .unwrap()
            .adjustments
            .exposure,
        1.25
    );
    assert_eq!(
        fixture
            .human
            .project
            .photo(fixture.photo_id)
            .unwrap()
            .adjustments
            .contrast,
        18.0
    );
}

#[test]
fn stale_together_expectation_is_definitely_unapplied() {
    let mut fixture = Fixture::new(CollaborationMode::Together);
    let mut expectation = fixture.expectation();
    expectation.source_revision = Some(spectrum_revisions::RevisionId::new());
    let before = fixture
        .human
        .history_for(fixture.photo_id)
        .unwrap()
        .unwrap()
        .revisions
        .len();
    let error = LumenLiveSessions::new(&fixture.path)
        .apply(
            &mut fixture.human,
            fixture.collaboration.agent_session,
            LumenLiveAction::ExecuteBatch {
                expectation,
                command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
                commands: vec![Command::FlipHorizontal {
                    id: fixture.photo_id,
                }],
            },
        )
        .unwrap_err();
    assert!(matches!(error, LumenLiveApplyError::DefinitelyUnapplied(_)));
    assert_eq!(
        fixture
            .human
            .history_for(fixture.photo_id)
            .unwrap()
            .unwrap()
            .revisions
            .len(),
        before
    );
}

#[test]
fn cross_project_workspace_and_unknown_session_are_definitely_unapplied() {
    let fixture = Fixture::new(CollaborationMode::Together);
    let mut other = Fixture::new(CollaborationMode::Together);
    let cross_project = LumenLiveSessions::new(&fixture.path)
        .apply(
            &mut other.human,
            fixture.collaboration.agent_session,
            LumenLiveAction::State,
        )
        .unwrap_err();
    assert!(matches!(
        cross_project,
        LumenLiveApplyError::DefinitelyUnapplied(_)
    ));

    let mut human = fixture.human;
    let unknown_session = LumenLiveSessions::new(&fixture.path)
        .apply(&mut human, SessionId::new(), LumenLiveAction::State)
        .unwrap_err();
    assert!(matches!(
        unknown_session,
        LumenLiveApplyError::DefinitelyUnapplied(_)
    ));
}

#[test]
fn planner_error_with_unchanged_cursor_is_definitely_unapplied() {
    let mut fixture = Fixture::new(CollaborationMode::Together);
    let expectation = fixture.expectation();
    let error = LumenLiveSessions::new(&fixture.path)
        .apply(
            &mut fixture.human,
            fixture.collaboration.agent_session,
            LumenLiveAction::ExecuteBatch {
                expectation,
                command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
                commands: vec![Command::ApplyPreset {
                    preset_id: u64::MAX,
                    ids: vec![fixture.photo_id],
                }],
            },
        )
        .unwrap_err();
    assert!(matches!(error, LumenLiveApplyError::DefinitelyUnapplied(_)));
}

#[test]
fn separate_live_edit_never_moves_the_human_cursor() {
    let mut fixture = Fixture::new(CollaborationMode::Separate);
    let expectation = fixture.expectation();
    let before = fixture
        .human
        .live_state_for_track(fixture.collaboration.track_id)
        .unwrap()
        .unwrap()
        .photo_cursor;
    LumenLiveSessions::new(&fixture.path)
        .apply(
            &mut fixture.human,
            fixture.collaboration.agent_session,
            LumenLiveAction::ExecuteBatch {
                expectation,
                command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
                commands: vec![Command::Rotate {
                    id: fixture.photo_id,
                    clockwise: true,
                }],
            },
        )
        .unwrap();
    assert_eq!(
        fixture
            .human
            .live_state_for_track(fixture.collaboration.track_id)
            .unwrap()
            .unwrap()
            .photo_cursor,
        before
    );
    assert_eq!(
        fixture
            .human
            .project
            .photo(fixture.photo_id)
            .unwrap()
            .adjustments
            .rotation,
        0
    );
}

#[test]
fn post_commit_fault_is_reported_as_outcome_unknown() {
    let mut fixture = Fixture::new(CollaborationMode::Together);
    let mut sessions = LumenLiveSessions::new(&fixture.path);
    sessions.inject_fault(LumenLiveTestFault::AfterAgentMutation);
    let expectation = fixture.expectation();
    let before_agent = expectation.agent_revision;
    let error = sessions
        .apply(
            &mut fixture.human,
            fixture.collaboration.agent_session,
            LumenLiveAction::ExecuteBatch {
                expectation,
                command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
                commands: vec![Command::FlipVertical {
                    id: fixture.photo_id,
                }],
            },
        )
        .unwrap_err();
    assert!(matches!(error, LumenLiveApplyError::OutcomeUnknown(_)));
    let reopened = Workspace::open_session(&fixture.path, fixture.collaboration.agent_session)
        .unwrap()
        .live_state_for_track(fixture.collaboration.track_id)
        .unwrap()
        .unwrap();
    assert_ne!(
        reopened.photo_cursor, before_agent,
        "the injected error occurs only after the durable agent revision exists"
    );
}

#[test]
fn host_snapshot_is_catalog_only_and_gui_drain_applies_photo_action() {
    let mut fixture = Fixture::new(CollaborationMode::Together);
    let expectation = fixture.expectation();
    let (host, mut drain) = LumenLiveHost::new(&fixture.human, BindingId::new(), 1).unwrap();
    host.attach_events(Arc::new(EventLog::new())).unwrap();
    let snapshot = host.with_snapshot(Ok).unwrap();
    let (_, catalog_track, catalog_cursor, _) = fixture.human.live_catalog_identity().unwrap();
    assert_eq!(snapshot.cursors.len(), 1);
    assert_eq!(snapshot.cursors[0].track_id, catalog_track);
    assert_eq!(snapshot.cursors[0].revision_id, catalog_cursor);
    assert!(snapshot.application_state.is_empty());

    let request = request(
        &fixture,
        &host,
        InteractionPolicy::Immediate,
        LumenLiveAction::ExecuteBatch {
            expectation,
            command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
            commands: vec![Command::Adjust {
                id: fixture.photo_id,
                patch: AdjustmentPatch {
                    exposure: Some(0.75),
                    ..Default::default()
                },
            }],
        },
    );
    let worker_host = Arc::clone(&host);
    let worker = thread::spawn(move || worker_host.apply_if_current(&request));
    wait_for_ingress(&drain);
    let report = drain.drain(&mut fixture.human, LumenLiveInteractionState::Idle);
    assert_eq!(report.applied, 1);
    let HostApplyOutcome::Applied(ResponseBody::Applied { .. }) = worker.join().unwrap().unwrap()
    else {
        panic!("expected applied live response");
    };
    assert_eq!(
        fixture
            .human
            .project
            .photo(fixture.photo_id)
            .unwrap()
            .adjustments
            .exposure,
        0.75
    );
}

#[test]
fn host_publishes_the_exact_agent_revision_event() {
    let mut fixture = Fixture::new(CollaborationMode::Separate);
    let expectation = fixture.expectation();
    let (host, mut drain) = LumenLiveHost::new(&fixture.human, BindingId::new(), 1).unwrap();
    let events = Arc::new(EventLog::new());
    let subscription = events.subscribe(0).unwrap();
    host.attach_events(events).unwrap();
    let request = request(
        &fixture,
        &host,
        InteractionPolicy::Immediate,
        LumenLiveAction::ExecuteBatch {
            expectation,
            command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
            commands: vec![Command::Rotate {
                id: fixture.photo_id,
                clockwise: true,
            }],
        },
    );
    let request_id = request.request_id;
    let worker_host = Arc::clone(&host);
    let worker = thread::spawn(move || worker_host.apply_if_current(&request));
    wait_for_ingress(&drain);
    assert_eq!(
        drain
            .drain(&mut fixture.human, LumenLiveInteractionState::Idle)
            .applied,
        1
    );
    assert!(matches!(
        worker.join().unwrap().unwrap(),
        HostApplyOutcome::Applied(ResponseBody::Applied { .. })
    ));
    let event = subscription.try_next().unwrap().unwrap();
    let BridgeEventKind::RevisionCommitted {
        request_id: event_request,
        session_id,
        cursors,
        ..
    } = event.event
    else {
        panic!("expected exact Lumen revision event");
    };
    assert_eq!(event_request, Some(request_id));
    assert_eq!(session_id, fixture.collaboration.agent_session);
    assert_eq!(cursors.len(), 1);
    let agent = Workspace::open_session(&fixture.path, fixture.collaboration.agent_session)
        .unwrap()
        .live_state_for_track(fixture.collaboration.track_id)
        .unwrap()
        .unwrap();
    assert_eq!(cursors[0].to_revision_id, agent.photo_cursor);
}

#[test]
fn active_interaction_refuses_immediate_and_releases_one_deferred_action() {
    let mut fixture = Fixture::new(CollaborationMode::Separate);
    let (host, mut drain) = LumenLiveHost::new(&fixture.human, BindingId::new(), 1).unwrap();
    let events = Arc::new(EventLog::new());
    let subscription = events.subscribe(0).unwrap();
    host.attach_events(events).unwrap();

    let immediate = request(
        &fixture,
        &host,
        InteractionPolicy::Immediate,
        LumenLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
            commands: vec![Command::FlipHorizontal {
                id: fixture.photo_id,
            }],
        },
    );
    let worker_host = Arc::clone(&host);
    let worker = thread::spawn(move || worker_host.apply_if_current(&immediate));
    wait_for_ingress(&drain);
    assert_eq!(
        drain
            .drain(&mut fixture.human, LumenLiveInteractionState::Active)
            .refused,
        1
    );
    assert!(matches!(
        worker.join().unwrap().unwrap(),
        HostApplyOutcome::Applied(ResponseBody::Refused { .. })
    ));

    let deferred = request(
        &fixture,
        &host,
        InteractionPolicy::Deferred,
        LumenLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
            commands: vec![Command::FlipVertical {
                id: fixture.photo_id,
            }],
        },
    );
    let worker_host = Arc::clone(&host);
    let worker = thread::spawn(move || worker_host.apply_if_current(&deferred));
    wait_for_ingress(&drain);
    assert_eq!(
        drain
            .drain(&mut fixture.human, LumenLiveInteractionState::Active)
            .deferred,
        1
    );
    assert!(matches!(
        worker.join().unwrap().unwrap(),
        HostApplyOutcome::Applied(ResponseBody::Deferred)
    ));
    assert!(drain.has_pending());
    assert!(matches!(
        subscription.try_next().unwrap().unwrap().event,
        BridgeEventKind::InteractionBegan { .. }
    ));
    assert_eq!(
        drain
            .drain(&mut fixture.human, LumenLiveInteractionState::Idle)
            .applied,
        1
    );
    assert!(!drain.has_pending());
    assert!(matches!(
        subscription.try_next().unwrap().unwrap().event,
        BridgeEventKind::RevisionCommitted { .. }
    ));
    assert!(matches!(
        subscription.try_next().unwrap().unwrap().event,
        BridgeEventKind::CollaborationSplit { .. }
    ));
    assert!(matches!(
        subscription.try_next().unwrap().unwrap().event,
        BridgeEventKind::InteractionCommitted { .. }
    ));
    assert!(subscription.try_next().unwrap().is_none());
}

#[test]
fn uncertain_host_outcome_poison_closes_the_exact_binding() {
    let mut fixture = Fixture::new(CollaborationMode::Together);
    let expectation = fixture.expectation();
    let (host, mut drain) = LumenLiveHost::new(&fixture.human, BindingId::new(), 1).unwrap();
    host.attach_events(Arc::new(EventLog::new())).unwrap();
    drain.inject_session_fault(LumenLiveTestFault::AfterAgentMutation);
    let request = request(
        &fixture,
        &host,
        InteractionPolicy::Immediate,
        LumenLiveAction::ExecuteBatch {
            expectation,
            command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
            commands: vec![Command::FlipHorizontal {
                id: fixture.photo_id,
            }],
        },
    );
    let worker_host = Arc::clone(&host);
    let worker = thread::spawn(move || worker_host.apply_if_current(&request));
    wait_for_ingress(&drain);
    let report = drain.drain(&mut fixture.human, LumenLiveInteractionState::Idle);
    assert!(report.outcome_unknown);
    assert!(report.reopen_required);
    assert!(matches!(
        worker.join().unwrap(),
        Err(BridgeError::Protocol(_))
    ));
    assert!(matches!(host.current_cursors(), Err(BridgeError::Closed)));
}
