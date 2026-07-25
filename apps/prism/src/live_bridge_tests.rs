use serde_json::json;
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use spectrum_live_bridge::{
    ActionEnvelope, BindingId, BridgeEventKind, BridgeServer, ExpectedCursor, InstanceId,
    InteractionPolicy, PROTOCOL_FAMILY, PROTOCOL_VERSION, RequestEnvelope, RequestId, ResponseBody,
    ServerConfig,
};
use spectrum_revisions::{Actor, ActorKind, CollaborationMode, RevisionId, SessionId};

use crate::*;

struct Fixture {
    _directory: tempfile::TempDir,
    path: std::path::PathBuf,
    human: Workspace,
    agent_session: SessionId,
}

impl Fixture {
    fn new(mode: CollaborationMode) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("live.prism");
        let human_session = SessionId::new();
        let human = Workspace::create_durable(
            Document::new("Live", 320, 200),
            &path,
            Actor {
                id: "human:test".into(),
                display_name: "Test Human".into(),
                kind: ActorKind::Human,
            },
            human_session,
        )
        .unwrap();
        let collaboration = Workspace::start_collaboration(
            &path,
            Some(human_session),
            Actor {
                id: "agent:test".into(),
                display_name: "Test Agent".into(),
                kind: ActorKind::Agent,
            },
            mode,
        )
        .unwrap();
        Self {
            _directory: directory,
            path,
            human,
            agent_session: collaboration.agent_session,
        }
    }

    fn state(&self) -> (LiveWorkspaceState, LiveWorkspaceState) {
        (
            self.human.live_state().unwrap().unwrap(),
            Workspace::open_session(&self.path, self.agent_session)
                .unwrap()
                .live_state()
                .unwrap()
                .unwrap(),
        )
    }

    fn expectation(&self) -> PrismLiveActionExpectation {
        let (human, agent) = self.state();
        let collaboration = Workspace::collaboration(&self.path, self.agent_session).unwrap();
        PrismLiveActionExpectation {
            agent_revision: agent.cursor,
            source_revision: (collaboration.mode == CollaborationMode::Together)
                .then_some(human.cursor),
        }
    }

    fn sessions(&self) -> PrismLiveSessions {
        PrismLiveSessions::new(&self.path)
    }
}

fn rename(name: &str) -> Command {
    Command::RenameDocument { name: name.into() }
}

struct HostHarness {
    server: Arc<BridgeServer<PrismLiveHost>>,
    drain: PrismLiveDrain,
    binding_id: BindingId,
    project_id: spectrum_revisions::ProjectId,
    track_id: spectrum_revisions::TrackId,
    human_cursor: RevisionId,
}

impl HostHarness {
    fn new(fixture: &Fixture) -> Self {
        let human = fixture.human.live_state().unwrap().unwrap();
        let binding_id = BindingId::new();
        let (host, drain) = PrismLiveHost::new(&fixture.human, binding_id, 1).unwrap();
        let server = Arc::new(BridgeServer::new(
            ServerConfig {
                application: "spectrum.prism".into(),
                project_id: human.project_id,
                instance_id: InstanceId::new(),
                binding_id,
                binding_epoch: 1,
            },
            spectrum_live_bridge::Capability::generate().unwrap(),
            host.clone(),
        ));
        host.attach_events(Arc::clone(server.events())).unwrap();
        Self {
            server,
            drain,
            binding_id,
            project_id: human.project_id,
            track_id: human.track_id,
            human_cursor: human.cursor,
        }
    }

    fn request(
        &self,
        fixture: &Fixture,
        action: PrismLiveAction,
        interaction: InteractionPolicy,
    ) -> RequestEnvelope {
        RequestEnvelope {
            protocol: PROTOCOL_FAMILY.into(),
            version: PROTOCOL_VERSION,
            request_id: RequestId::new(),
            binding_id: self.binding_id,
            binding_epoch: 1,
            project_id: self.project_id,
            application: "spectrum.prism".into(),
            session_id: fixture.agent_session,
            expected_cursors: vec![ExpectedCursor {
                track_id: self.track_id,
                revision_id: self.human_cursor,
            }],
            actor_label: "Test Agent".into(),
            interaction,
            action: ActionEnvelope {
                family: PRISM_LIVE_ACTION_FAMILY.into(),
                version: PRISM_LIVE_ACTION_VERSION,
                capabilities: Vec::new(),
                action: serde_json::to_value(action).unwrap(),
            },
        }
    }

    fn round_trip(
        &mut self,
        workspace: &mut Workspace,
        request: RequestEnvelope,
        interaction: PrismLiveInteractionState,
    ) -> spectrum_live_bridge::ResponseEnvelope {
        let server = Arc::clone(&self.server);
        let worker = thread::spawn(move || server.handle_request(request).unwrap());
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut received = false;
        while Instant::now() < deadline {
            if self.drain.drain(workspace, interaction).received > 0 {
                received = true;
                break;
            }
            thread::sleep(Duration::from_micros(100));
        }
        assert!(received, "live host did not enqueue within one second");
        worker.join().unwrap()
    }
}

#[test]
fn typed_action_rejects_wrong_family_capabilities_and_nested_history() {
    let envelope = ActionEnvelope {
        family: "wrong".into(),
        version: PRISM_LIVE_ACTION_VERSION,
        capabilities: Vec::new(),
        action: json!({"action": "state"}),
    };
    assert!(decode_live_action(&envelope).is_err());

    let mut envelope = ActionEnvelope {
        family: PRISM_LIVE_ACTION_FAMILY.into(),
        version: PRISM_LIVE_ACTION_VERSION,
        capabilities: vec!["future".into()],
        action: json!({"action": "state"}),
    };
    assert!(decode_live_action(&envelope).is_err());
    envelope.capabilities.clear();
    envelope.action = json!({
        "action": "execute_batch",
        "expectation": {
            "agent_revision": RevisionId::new(),
        },
        "command_version": PRISM_COMMAND_OPERATIONS_VERSION,
        "commands": [{"command": "undo"}],
    });
    assert!(decode_live_action(&envelope).is_err());
}

#[test]
fn execute_batch_is_one_agent_revision_and_together_advances_human() {
    let mut fixture = Fixture::new(CollaborationMode::Together);
    let before_history = Workspace::open_session(&fixture.path, fixture.agent_session)
        .unwrap()
        .history()
        .unwrap()
        .unwrap();
    let action = PrismLiveAction::ExecuteBatch {
        expectation: fixture.expectation(),
        command_version: PRISM_COMMAND_OPERATIONS_VERSION,
        commands: vec![
            rename("First"),
            Command::SetCanvas {
                width: 400,
                height: 240,
                background: [1, 2, 3, 255],
            },
        ],
    };
    let mut sessions = fixture.sessions();
    let PrismLiveResult::Applied(applied) = sessions
        .apply(&mut fixture.human, fixture.agent_session, action)
        .unwrap()
    else {
        panic!("mutation returned state");
    };
    assert_ne!(applied.before.agent_cursor, applied.after.agent_cursor);
    assert_eq!(applied.after.human_cursor, applied.after.agent_cursor);
    assert_eq!(fixture.human.document.name, "First");
    assert_eq!(
        (fixture.human.document.width, fixture.human.document.height),
        (400, 240)
    );

    let after_history = fixture.human.history().unwrap().unwrap();
    assert_eq!(
        after_history.revisions.len(),
        before_history.revisions.len() + 1
    );
    let revision = after_history
        .revisions
        .iter()
        .find(|revision| revision.id == applied.after.agent_cursor)
        .unwrap();
    assert_eq!(revision.command_count, 2);
}

#[test]
fn separate_mutation_preserves_human_canvas_and_requires_no_source_cursor() {
    let mut fixture = Fixture::new(CollaborationMode::Separate);
    let human_before = fixture.human.document.clone();
    let action = PrismLiveAction::ExecuteBatch {
        expectation: fixture.expectation(),
        command_version: PRISM_COMMAND_OPERATIONS_VERSION,
        commands: vec![rename("Agent branch")],
    };
    let mut sessions = fixture.sessions();
    let PrismLiveResult::Applied(applied) = sessions
        .apply(&mut fixture.human, fixture.agent_session, action)
        .unwrap()
    else {
        panic!("mutation returned state");
    };
    assert_eq!(fixture.human.document, human_before);
    assert_eq!(applied.before.human_cursor, applied.after.human_cursor);
    assert_ne!(applied.before.agent_cursor, applied.after.agent_cursor);
}

#[test]
fn stale_agent_or_together_source_cursor_cannot_mutate() {
    let mut fixture = Fixture::new(CollaborationMode::Together);
    let document_before = fixture.human.document.clone();
    let history_before = fixture.human.history().unwrap().unwrap().revisions.len();
    let mut expectation = fixture.expectation();
    expectation.agent_revision = RevisionId::new();
    let action = PrismLiveAction::ExecuteBatch {
        expectation,
        command_version: PRISM_COMMAND_OPERATIONS_VERSION,
        commands: vec![rename("Must not land")],
    };
    let mut sessions = fixture.sessions();
    assert!(
        sessions
            .apply(&mut fixture.human, fixture.agent_session, action)
            .is_err()
    );
    assert_eq!(fixture.human.document, document_before);
    assert_eq!(
        fixture.human.history().unwrap().unwrap().revisions.len(),
        history_before
    );

    let mut expectation = fixture.expectation();
    expectation.source_revision = Some(RevisionId::new());
    let action = PrismLiveAction::ExecuteBatch {
        expectation,
        command_version: PRISM_COMMAND_OPERATIONS_VERSION,
        commands: vec![rename("Still must not land")],
    };
    assert!(
        sessions
            .apply(&mut fixture.human, fixture.agent_session, action)
            .is_err()
    );
    assert_eq!(fixture.human.document, document_before);
}

#[test]
fn collaboration_from_another_bound_human_is_refused() {
    let fixture = Fixture::new(CollaborationMode::Separate);
    let other_path = fixture._directory.path().join("other.prism");
    let other_session = SessionId::new();
    let mut other = Workspace::create_durable(
        Document::new("Other", 100, 100),
        &other_path,
        Actor {
            id: "human:other".into(),
            display_name: "Other Human".into(),
            kind: ActorKind::Human,
        },
        other_session,
    )
    .unwrap();
    let action = PrismLiveAction::State;
    let mut sessions = fixture.sessions();
    assert!(
        sessions
            .apply(&mut other, fixture.agent_session, action)
            .is_err()
    );
}

#[test]
fn undo_redo_and_move_cursor_use_explicit_actions() {
    let mut fixture = Fixture::new(CollaborationMode::Separate);
    let root = fixture.state().1.cursor;
    let mut sessions = fixture.sessions();
    let apply = PrismLiveAction::ExecuteBatch {
        expectation: fixture.expectation(),
        command_version: PRISM_COMMAND_OPERATIONS_VERSION,
        commands: vec![rename("Changed")],
    };
    sessions
        .apply(&mut fixture.human, fixture.agent_session, apply)
        .unwrap();

    let undo = PrismLiveAction::Undo {
        expectation: fixture.expectation(),
    };
    sessions
        .apply(&mut fixture.human, fixture.agent_session, undo)
        .unwrap();
    assert_eq!(fixture.state().1.cursor, root);

    let redo = PrismLiveAction::Redo {
        expectation: fixture.expectation(),
    };
    sessions
        .apply(&mut fixture.human, fixture.agent_session, redo)
        .unwrap();
    assert_ne!(fixture.state().1.cursor, root);

    let movement = PrismLiveAction::MoveAgentCursor {
        expectation: fixture.expectation(),
        target: root,
    };
    sessions
        .apply(&mut fixture.human, fixture.agent_session, movement)
        .unwrap();
    assert_eq!(fixture.state().1.cursor, root);
}

#[test]
fn external_agent_advance_invalidates_the_old_expectation_without_overwrite() {
    let mut fixture = Fixture::new(CollaborationMode::Separate);
    let stale_expectation = fixture.expectation();
    let mut external = Workspace::open_session(&fixture.path, fixture.agent_session).unwrap();
    external.execute(rename("External advance")).unwrap();
    let external_state = external.live_state().unwrap().unwrap();
    let mut sessions = fixture.sessions();
    let action = PrismLiveAction::ExecuteBatch {
        expectation: stale_expectation,
        command_version: PRISM_COMMAND_OPERATIONS_VERSION,
        commands: vec![rename("Must not overwrite")],
    };
    assert!(
        sessions
            .apply(&mut fixture.human, fixture.agent_session, action)
            .is_err()
    );
    let current = Workspace::open_session(&fixture.path, fixture.agent_session).unwrap();
    assert_eq!(
        current.live_state().unwrap().unwrap().cursor,
        external_state.cursor
    );
    assert_eq!(current.document.name, "External advance");
}

#[test]
fn durable_expected_parent_rejects_two_writers_opened_at_one_agent_cursor() {
    let fixture = Fixture::new(CollaborationMode::Separate);
    let mut winner = Workspace::open_session(&fixture.path, fixture.agent_session).unwrap();
    let mut stale = Workspace::open_session(&fixture.path, fixture.agent_session).unwrap();
    winner.execute(rename("Winner")).unwrap();
    assert!(stale.execute(rename("Stale overwrite")).is_err());
    let current = Workspace::open_session(&fixture.path, fixture.agent_session).unwrap();
    assert_eq!(current.document.name, "Winner");
}

#[test]
fn host_rechecks_the_human_cursor_on_the_gui_thread_before_mutation() {
    let mut fixture = Fixture::new(CollaborationMode::Together);
    let mut harness = HostHarness::new(&fixture);
    let request = harness.request(
        &fixture,
        PrismLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
            commands: vec![rename("Must conflict")],
        },
        InteractionPolicy::Immediate,
    );
    fixture.human.execute(rename("Direct human edit")).unwrap();
    let response = harness.round_trip(&mut fixture.human, request, PrismLiveInteractionState::Idle);
    assert!(matches!(response.body, ResponseBody::Conflict { .. }));
    assert_eq!(fixture.human.document.name, "Direct human edit");
    let agent = Workspace::open_session(&fixture.path, fixture.agent_session).unwrap();
    assert_ne!(agent.document.name, "Must conflict");
}

#[test]
fn host_applies_one_batch_and_publishes_the_exact_revision_event() {
    let mut fixture = Fixture::new(CollaborationMode::Separate);
    let mut harness = HostHarness::new(&fixture);
    let subscription = harness.server.events().subscribe(0).unwrap();
    let request = harness.request(
        &fixture,
        PrismLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
            commands: vec![rename("Bridge commit")],
        },
        InteractionPolicy::Immediate,
    );
    let response = harness.round_trip(
        &mut fixture.human,
        request.clone(),
        PrismLiveInteractionState::Idle,
    );
    assert!(matches!(response.body, ResponseBody::Applied { .. }));
    let event = subscription.try_next().unwrap().unwrap();
    let BridgeEventKind::RevisionCommitted {
        request_id,
        session_id,
        cursors,
        ..
    } = event.event
    else {
        panic!("expected revision event");
    };
    assert_eq!(request_id, Some(request.request_id));
    assert_eq!(session_id, fixture.agent_session);
    assert_eq!(cursors.len(), 1);
    let agent = Workspace::open_session(&fixture.path, fixture.agent_session).unwrap();
    assert_eq!(agent.document.name, "Bridge commit");
    assert_eq!(
        agent.history().unwrap().unwrap().current,
        cursors[0].to_revision_id
    );
}

#[test]
fn active_interaction_refuses_immediate_and_releases_one_deferred_request() {
    let mut fixture = Fixture::new(CollaborationMode::Separate);
    let mut harness = HostHarness::new(&fixture);
    let immediate = harness.request(
        &fixture,
        PrismLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
            commands: vec![rename("Immediate")],
        },
        InteractionPolicy::Immediate,
    );
    let response = harness.round_trip(
        &mut fixture.human,
        immediate,
        PrismLiveInteractionState::Active,
    );
    assert!(matches!(response.body, ResponseBody::Refused { .. }));
    assert_ne!(
        Workspace::open_session(&fixture.path, fixture.agent_session)
            .unwrap()
            .document
            .name,
        "Immediate"
    );

    let deferred = harness.request(
        &fixture,
        PrismLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
            commands: vec![rename("Deferred")],
        },
        InteractionPolicy::Deferred,
    );
    let response = harness.round_trip(
        &mut fixture.human,
        deferred,
        PrismLiveInteractionState::Active,
    );
    assert!(matches!(response.body, ResponseBody::Deferred));
    assert!(harness.drain.has_pending());
    let report = harness
        .drain
        .drain(&mut fixture.human, PrismLiveInteractionState::Idle);
    assert_eq!(report.applied, 1);
    assert!(!harness.drain.has_pending());
    assert_eq!(
        Workspace::open_session(&fixture.path, fixture.agent_session)
            .unwrap()
            .document
            .name,
        "Deferred"
    );

    let confirmation = harness.request(
        &fixture,
        PrismLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
            commands: vec![rename("Unconfirmed")],
        },
        InteractionPolicy::RequireUserConfirmation,
    );
    let response = harness.round_trip(
        &mut fixture.human,
        confirmation,
        PrismLiveInteractionState::Idle,
    );
    assert!(matches!(response.body, ResponseBody::Refused { .. }));
}
