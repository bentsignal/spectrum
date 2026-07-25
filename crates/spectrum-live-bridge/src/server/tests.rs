use std::sync::{
    Barrier, Weak,
    atomic::{AtomicUsize, Ordering},
};

use spectrum_revisions::SessionId;

use super::*;
use crate::{
    ActionEnvelope, BridgeClient, ClientConfig, CursorTransition, InteractionPolicy,
    PROTOCOL_FAMILY, PROTOCOL_VERSION, RequestId,
};
#[cfg(unix)]
use std::fs;

struct MockHost {
    cursor: Mutex<ExpectedCursor>,
    calls: AtomicUsize,
    apply_mode: AtomicUsize,
    binding: ServerConfig,
}

struct AtomicCutHost {
    cursor: Mutex<ExpectedCursor>,
    events: Mutex<Option<Weak<EventLog>>>,
    binding: ServerConfig,
}

impl BridgeHost for AtomicCutHost {
    fn current_cursors(&self) -> BridgeResult<Vec<ExpectedCursor>> {
        Ok(vec![self.cursor.lock().unwrap().clone()])
    }

    fn with_snapshot<R>(
        &self,
        attach: impl FnOnce(StateSnapshot) -> BridgeResult<R>,
    ) -> BridgeResult<R> {
        let cursor = self.cursor.lock().unwrap();
        attach(StateSnapshot {
            project_id: self.binding.project_id,
            binding_id: self.binding.binding_id,
            binding_epoch: self.binding.binding_epoch,
            cursors: vec![cursor.clone()],
            current_event_seq: 0,
            application_protocols: Default::default(),
            application_state: Default::default(),
        })
    }

    fn apply_if_current(&self, request: &RequestEnvelope) -> BridgeResult<HostApplyOutcome> {
        let mut cursor = self.cursor.lock().unwrap();
        if ensure_exact_cursors(&request.expected_cursors, std::slice::from_ref(&cursor)).is_err() {
            return Ok(HostApplyOutcome::Conflict(vec![cursor.clone()]));
        }
        let previous = cursor.revision_id;
        cursor.revision_id = RevisionId::new();
        let events = self
            .events
            .lock()
            .unwrap()
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or(BridgeError::Closed)?;
        events.append(BridgeEventKind::CursorMoved {
            session_id: request.session_id,
            cursors: vec![CursorTransition {
                track_id: cursor.track_id,
                from_revision_id: previous,
                to_revision_id: cursor.revision_id,
            }],
        })?;
        Ok(HostApplyOutcome::Applied(ResponseBody::Applied {
            result: serde_json::json!({"ok": true}),
            cursors: vec![cursor.clone()],
        }))
    }
}

impl BridgeHost for MockHost {
    fn current_cursors(&self) -> BridgeResult<Vec<ExpectedCursor>> {
        Ok(vec![self.cursor.lock().unwrap().clone()])
    }

    fn with_snapshot<R>(
        &self,
        attach: impl FnOnce(StateSnapshot) -> BridgeResult<R>,
    ) -> BridgeResult<R> {
        let cursor = self.cursor.lock().unwrap();
        attach(StateSnapshot {
            project_id: self.binding.project_id,
            binding_id: self.binding.binding_id,
            binding_epoch: self.binding.binding_epoch,
            cursors: vec![cursor.clone()],
            current_event_seq: 0,
            application_protocols: Default::default(),
            application_state: Default::default(),
        })
    }

    fn apply_if_current(&self, request: &RequestEnvelope) -> BridgeResult<HostApplyOutcome> {
        let mut cursor = self.cursor.lock().unwrap();
        if ensure_exact_cursors(&request.expected_cursors, std::slice::from_ref(&cursor)).is_err() {
            return Ok(HostApplyOutcome::Conflict(vec![cursor.clone()]));
        }
        self.calls.fetch_add(1, Ordering::Relaxed);
        cursor.revision_id = RevisionId::new();
        match self.apply_mode.load(Ordering::Relaxed) {
            1 => return Err(BridgeError::Protocol("failure after durable apply".into())),
            2 => {
                return Ok(HostApplyOutcome::Applied(ResponseBody::Applied {
                    result: serde_json::json!({"ok": true}),
                    cursors: Vec::new(),
                }));
            }
            _ => {}
        }
        Ok(HostApplyOutcome::Applied(ResponseBody::Applied {
            result: serde_json::json!({"ok": true}),
            cursors: vec![cursor.clone()],
        }))
    }
}

fn setup() -> (Arc<MockHost>, BridgeServer<MockHost>, RequestEnvelope) {
    let config = ServerConfig {
        application: "mock".into(),
        project_id: ProjectId::new(),
        instance_id: InstanceId::new(),
        binding_id: BindingId::new(),
        binding_epoch: 1,
    };
    let cursor = ExpectedCursor {
        track_id: TrackId::new(),
        revision_id: RevisionId::new(),
    };
    let host = Arc::new(MockHost {
        cursor: Mutex::new(cursor.clone()),
        calls: AtomicUsize::new(0),
        apply_mode: AtomicUsize::new(0),
        binding: config.clone(),
    });
    let server = BridgeServer::new(
        config.clone(),
        Capability::generate().unwrap(),
        host.clone(),
    );
    let request = RequestEnvelope {
        protocol: PROTOCOL_FAMILY.into(),
        version: PROTOCOL_VERSION,
        request_id: RequestId::new(),
        binding_id: config.binding_id,
        binding_epoch: 1,
        project_id: config.project_id,
        application: "mock".into(),
        session_id: SessionId::new(),
        expected_cursors: vec![cursor],
        actor_label: "test agent".into(),
        interaction: InteractionPolicy::Immediate,
        action: ActionEnvelope {
            family: "mock.action".into(),
            version: 1,
            capabilities: vec![],
            action: serde_json::json!({"kind": "noop"}),
        },
    };
    (host, server, request)
}

#[test]
fn closing_server_refuses_new_application_work() {
    let (host, server, request) = setup();
    server.close();
    assert!(matches!(
        server.handle_request(request),
        Err(BridgeError::Closed)
    ));
    assert_eq!(host.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn exact_retry_is_applied_once_across_concurrent_callers() {
    let (host, server, request) = setup();
    let server = Arc::new(server);
    let barrier = Arc::new(Barrier::new(17));
    let mut threads = Vec::new();
    for _ in 0..16 {
        let server = Arc::clone(&server);
        let request = request.clone();
        let barrier = Arc::clone(&barrier);
        threads.push(thread::spawn(move || {
            barrier.wait();
            server.handle_request(request).unwrap()
        }));
    }
    barrier.wait();
    for thread in threads {
        assert!(matches!(
            thread.join().unwrap().body,
            ResponseBody::Applied { .. }
        ));
    }
    assert_eq!(host.calls.load(Ordering::Relaxed), 1);
}

#[test]
fn exact_retry_is_applied_once_across_authenticated_connections() {
    let (host, _, request) = setup();
    let capability = Capability::generate().unwrap();
    let client_capability = capability.duplicate();
    let server = Arc::new(BridgeServer::new(
        host.binding.clone(),
        capability,
        Arc::clone(&host),
    ));
    let (address, _temporary) = test_endpoint();
    let listener = crate::LocalListener::bind(&address).unwrap();
    let acceptor = thread::spawn({
        let server = Arc::clone(&server);
        move || {
            let mut connections = Vec::new();
            for _ in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                let server = Arc::clone(&server);
                connections.push(thread::spawn(move || {
                    server.serve_connection(stream).unwrap()
                }));
            }
            for connection in connections {
                connection.join().unwrap();
            }
        }
    });
    let barrier = Arc::new(Barrier::new(3));
    let callers = (0..2)
        .map(|_| {
            let address = address.clone();
            let capability = client_capability.duplicate();
            let request = request.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let mut client = BridgeClient::connect(&ClientConfig::local(address), &capability)?;
                client.request(request)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let mut caller_failure = None;
    for caller in callers {
        match caller.join().expect("client thread panicked") {
            Ok(response) => {
                assert!(matches!(response.body, ResponseBody::Applied { .. }));
            }
            Err(error) => caller_failure = Some(error),
        }
    }
    if let Some(error) = caller_failure {
        panic!("authenticated concurrent caller failed before both accepts: {error}");
    }
    acceptor.join().unwrap();
    assert_eq!(host.calls.load(Ordering::Relaxed), 1);
}

#[test]
fn two_request_ids_cannot_both_apply_against_one_cursor() {
    let (host, server, request) = setup();
    let server = Arc::new(server);
    let barrier = Arc::new(Barrier::new(3));
    let mut requests = Vec::new();
    for request_id in [request.request_id, RequestId::new()] {
        let mut request = request.clone();
        request.request_id = request_id;
        let server = Arc::clone(&server);
        let barrier = Arc::clone(&barrier);
        requests.push(thread::spawn(move || {
            barrier.wait();
            server.handle_request(request).unwrap()
        }));
    }
    barrier.wait();
    let bodies = requests
        .into_iter()
        .map(|thread| thread.join().unwrap().body)
        .collect::<Vec<_>>();
    assert_eq!(
        bodies
            .iter()
            .filter(|body| matches!(body, ResponseBody::Applied { .. }))
            .count(),
        1
    );
    assert_eq!(
        bodies
            .iter()
            .filter(|body| matches!(body, ResponseBody::Conflict { .. }))
            .count(),
        1
    );
    assert_eq!(host.calls.load(Ordering::Relaxed), 1);
}

#[test]
fn empty_or_stale_expectations_prevent_application() {
    let (host, server, mut request) = setup();
    request.expected_cursors.clear();
    assert!(server.handle_request(request.clone()).is_err());
    request.expected_cursors.push(ExpectedCursor {
        track_id: TrackId::new(),
        revision_id: RevisionId::new(),
    });
    let response = server.handle_request(request).unwrap();
    assert!(matches!(response.body, ResponseBody::Conflict { .. }));
    assert_eq!(host.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn every_failure_after_host_authority_makes_exact_retry_outcome_unknown() {
    for mode in [1, 2] {
        let (host, server, request) = setup();
        host.apply_mode.store(mode, Ordering::Relaxed);
        assert!(server.handle_request(request.clone()).is_err());
        let retry = server.handle_request(request.clone()).unwrap();
        assert_eq!(retry.body, ResponseBody::OutcomeUnknown);
        let mut collision = request;
        collision.actor_label = "different content".into();
        assert!(server.handle_request(collision).is_err());
        assert_eq!(host.calls.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn bounded_opaque_action_string_larger_than_four_kib_crosses_the_wire() {
    let (host, _, mut request) = setup();
    request.action.action = serde_json::json!({"payload": "x".repeat(64 * 1024)});
    let capability = Capability::generate().unwrap();
    let client_capability = capability.duplicate();
    let server = Arc::new(BridgeServer::new(
        host.binding.clone(),
        capability,
        Arc::clone(&host),
    ));
    let (address, _temporary) = test_endpoint();
    let listener = crate::LocalListener::bind(&address).unwrap();
    let server_thread = thread::spawn({
        let server = Arc::clone(&server);
        move || {
            let (stream, _) = listener.accept().unwrap();
            server.serve_connection(stream).unwrap();
        }
    });
    let mut client =
        BridgeClient::connect(&ClientConfig::local(address), &client_capability).unwrap();
    assert!(matches!(
        client.request(request).unwrap().body,
        ResponseBody::Applied { .. }
    ));
    drop(client);
    server_thread.join().unwrap();
}

#[test]
fn writer_failure_cancels_reader_before_connection_slot_is_released() {
    let (host, _, _) = setup();
    let capability = Capability::generate().unwrap();
    let client_capability = capability.duplicate();
    let server = Arc::new(BridgeServer::new(
        host.binding.clone(),
        capability,
        Arc::clone(&host),
    ));
    let (address, _temporary) = test_endpoint();
    let listener = crate::LocalListener::bind(&address).unwrap();
    let (done_sender, done_receiver) = mpsc::channel();
    let server_thread = thread::spawn({
        let server = Arc::clone(&server);
        move || {
            let result = listener
                .accept()
                .and_then(|(stream, _)| server.serve_connection(stream));
            done_sender.send(result).unwrap();
        }
    });
    let mut client =
        BridgeClient::connect(&ClientConfig::local(address), &client_capability).unwrap();
    client.subscribe(0).unwrap();
    for index in 0..250 {
        server
            .append_event(BridgeEventKind::InteractionBegan {
                interaction_id: format!("{index}-{}", "x".repeat(3_900)),
                interaction_kind: "blocked-writer".into(),
            })
            .unwrap();
    }
    let result = done_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("writer failure did not cancel and join the blocked reader");
    assert!(result.is_err());
    assert_eq!(server.active_connection_count(), 0);
    drop(client);
    server_thread.join().unwrap();
}

#[test]
fn host_owned_snapshot_cut_and_mutation_publish_have_no_gap_or_deadlock() {
    let config = ServerConfig {
        application: "atomic-cut".into(),
        project_id: ProjectId::new(),
        instance_id: InstanceId::new(),
        binding_id: BindingId::new(),
        binding_epoch: 1,
    };
    let initial = ExpectedCursor {
        track_id: TrackId::new(),
        revision_id: RevisionId::new(),
    };
    let host = Arc::new(AtomicCutHost {
        cursor: Mutex::new(initial.clone()),
        events: Mutex::new(None),
        binding: config.clone(),
    });
    let server = Arc::new(BridgeServer::new(
        config.clone(),
        Capability::generate().unwrap(),
        Arc::clone(&host),
    ));
    *host.events.lock().unwrap() = Some(Arc::downgrade(server.events()));
    let request = RequestEnvelope {
        protocol: PROTOCOL_FAMILY.into(),
        version: PROTOCOL_VERSION,
        request_id: RequestId::new(),
        binding_id: config.binding_id,
        binding_epoch: config.binding_epoch,
        project_id: config.project_id,
        application: config.application,
        session_id: SessionId::new(),
        expected_cursors: vec![initial.clone()],
        actor_label: "cut race".into(),
        interaction: InteractionPolicy::Immediate,
        action: ActionEnvelope {
            family: "atomic-cut.action".into(),
            version: 1,
            capabilities: vec![],
            action: serde_json::json!({"kind": "advance"}),
        },
    };
    let barrier = Arc::new(Barrier::new(3));
    let (result_sender, result_receiver) = mpsc::channel();
    let mutation = thread::spawn({
        let server = Arc::clone(&server);
        let barrier = Arc::clone(&barrier);
        let result_sender = result_sender.clone();
        move || {
            barrier.wait();
            result_sender.send(server.handle_request(request)).unwrap();
        }
    });
    let snapshot = thread::spawn({
        let server = Arc::clone(&server);
        let barrier = Arc::clone(&barrier);
        move || {
            barrier.wait();
            server.snapshot_subscription(0, Arc::new(AtomicUsize::new(0)))
        }
    });
    barrier.wait();
    let response = result_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("mutation deadlocked against snapshot cut")
        .unwrap();
    assert!(matches!(response.body, ResponseBody::Applied { .. }));
    let (snapshot, subscription) = snapshot
        .join()
        .expect("snapshot thread panicked")
        .expect("snapshot cut failed");
    mutation.join().unwrap();

    let mut rebuilt = snapshot.cursors[0].revision_id;
    while let Some(event) = subscription.try_next().unwrap() {
        if let BridgeEventKind::CursorMoved { cursors, .. } = event.event {
            assert_eq!(cursors[0].from_revision_id, rebuilt);
            rebuilt = cursors[0].to_revision_id;
        }
    }
    assert_eq!(rebuilt, host.cursor.lock().unwrap().revision_id);
    assert_eq!(
        snapshot.current_event_seq,
        u64::from(snapshot.cursors[0].revision_id == rebuilt)
    );
}

#[test]
fn authenticated_endpoint_round_trip_and_future_event_delivery() {
    let (host, _, request) = setup();
    let config = host.binding.clone();
    let capability = Capability::generate().unwrap();
    let client_capability = Capability::from_secret(capability.id(), *capability.secret());
    let server = Arc::new(BridgeServer::new(config, capability, host.clone()));
    let (address, _temporary) = test_endpoint();
    let listener = crate::LocalListener::bind(&address).unwrap();
    let server_thread = thread::spawn({
        let server = Arc::clone(&server);
        move || {
            let (stream, _) = listener.accept().unwrap();
            server.serve_connection(stream).unwrap();
        }
    });
    let mut client =
        BridgeClient::connect(&ClientConfig::local(address), &client_capability).unwrap();
    client.ping(91).unwrap();
    let response = client.request(request).unwrap();
    assert!(matches!(response.body, ResponseBody::Applied { .. }));
    let snapshot = client.subscribe(0).unwrap();
    assert_eq!(snapshot.current_event_seq, 0);
    let cursor = host.cursor.lock().unwrap().clone();
    server
        .append_event(BridgeEventKind::CursorMoved {
            session_id: SessionId::new(),
            cursors: vec![CursorTransition {
                track_id: cursor.track_id,
                from_revision_id: cursor.revision_id,
                to_revision_id: RevisionId::new(),
            }],
        })
        .unwrap();
    assert!(matches!(
        client.read_subscription_message().unwrap(),
        ServerMessage::Event(event) if event.seq == 1
    ));
    drop(client);
    server_thread.join().unwrap();
    assert_eq!(host.calls.load(Ordering::Relaxed), 1);
}

#[test]
fn authentication_stall_hits_real_transport_deadline() {
    let (host, _, _) = setup();
    let capability = Capability::generate().unwrap();
    let server = Arc::new(BridgeServer::new(
        host.binding.clone(),
        capability,
        Arc::clone(&host),
    ));
    let (address, _temporary) = test_endpoint();
    let listener = crate::LocalListener::bind(&address).unwrap();
    let (result_sender, result_receiver) = mpsc::channel();
    let server_thread = thread::spawn({
        let server = Arc::clone(&server);
        move || {
            let (stream, _) = listener.accept().unwrap();
            let _ = result_sender.send(server.serve_connection(stream));
        }
    });
    let mut stalled = crate::LocalStream::connect(&address).unwrap();
    let _: AuthChallenge = read_frame(&mut stalled).unwrap();
    let result = result_receiver
        .recv_timeout(AUTH_DEADLINE + Duration::from_secs(1))
        .expect("server did not enforce its authentication deadline");
    assert!(matches!(
        result,
        Err(BridgeError::Io(ref error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            )
    ));
    drop(stalled);
    server_thread.join().unwrap();
}

#[test]
fn rate_limit_is_typed_before_repeat_offender_disconnect() {
    let (host, _, _) = setup();
    let capability = Capability::generate().unwrap();
    let client_capability = capability.duplicate();
    let server = Arc::new(BridgeServer::new(
        host.binding.clone(),
        capability,
        Arc::clone(&host),
    ));
    let (address, _temporary) = test_endpoint();
    let listener = crate::LocalListener::bind(&address).unwrap();
    let server_thread = thread::spawn({
        let server = Arc::clone(&server);
        move || {
            let (stream, _) = listener.accept().unwrap();
            server.serve_connection(stream).unwrap();
        }
    });
    let mut client =
        BridgeClient::connect(&ClientConfig::local(address), &client_capability).unwrap();
    let first = (0..1_000)
        .find_map(|nonce| match client.ping(nonce) {
            Err(BridgeError::RateLimited {
                retry_after_millis,
                disconnect: false,
            }) => Some(retry_after_millis),
            Ok(()) => None,
            Err(error) => panic!("unexpected first rate-limit error: {error}"),
        })
        .expect("read bucket never produced its typed first violation");
    assert!(first <= 20);
    let second = (1_000..2_000).find_map(|nonce| match client.ping(nonce) {
        Err(BridgeError::RateLimited {
            retry_after_millis,
            disconnect: true,
        }) => Some(retry_after_millis),
        Ok(())
        | Err(BridgeError::RateLimited {
            disconnect: false, ..
        }) => None,
        Err(error) => panic!("unexpected repeat rate-limit error: {error}"),
    });
    assert!(second.is_some(), "repeat offender was not disconnected");
    drop(client);
    server_thread.join().unwrap();
}

fn test_endpoint() -> (crate::EndpointAddress, Option<tempfile::TempDir>) {
    #[cfg(unix)]
    {
        let temporary = tempfile::tempdir().unwrap();
        fs::set_permissions(
            temporary.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let address = crate::EndpointAddress::Unix {
            path: temporary
                .path()
                .join(format!("{}.sock", uuid::Uuid::new_v4())),
        };
        (address, Some(temporary))
    }
    #[cfg(windows)]
    {
        (
            crate::EndpointAddress::WindowsPipe {
                name: format!(r"\\.\pipe\spectrum-live-{}", uuid::Uuid::new_v4()),
            },
            None,
        )
    }
}
