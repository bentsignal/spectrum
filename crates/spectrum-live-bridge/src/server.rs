use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use spectrum_revisions::{ProjectId, RevisionId, TrackId};

use crate::{
    AUTH_DEADLINE, AuthChallenge, AuthProof, BindingId, BridgeError, BridgeResult, Capability,
    ClientMessage, EventLog, ExpectedCursor, InstanceId, LocalStream,
    MAX_AUTHENTICATED_CONNECTIONS, RequestCache, RequestEnvelope, ResponseBody, ResponseEnvelope,
    ServerMessage, StateSnapshot, read_frame, verify_proof, write_frame,
};

pub trait BridgeHost: Send + Sync + 'static {
    fn current_cursors(&self) -> BridgeResult<Vec<ExpectedCursor>>;
    fn snapshot(&self) -> BridgeResult<StateSnapshot>;
    fn apply(&self, request: &RequestEnvelope) -> BridgeResult<ResponseBody>;
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub application: String,
    pub project_id: ProjectId,
    pub instance_id: InstanceId,
    pub binding_id: BindingId,
    pub binding_epoch: u64,
}

pub struct BridgeServer<H> {
    config: ServerConfig,
    capability: Capability,
    host: Arc<H>,
    events: Arc<EventLog>,
    cache: Mutex<RequestCache>,
    replayed_client_nonces: Mutex<HashSet<[u8; 32]>>,
    authenticated_connections: AtomicUsize,
}

impl<H: BridgeHost> BridgeServer<H> {
    pub fn new(config: ServerConfig, capability: Capability, host: Arc<H>) -> Self {
        Self {
            config,
            capability,
            host,
            events: Arc::new(EventLog::new()),
            cache: Mutex::new(RequestCache::default()),
            replayed_client_nonces: Mutex::new(HashSet::new()),
            authenticated_connections: AtomicUsize::new(0),
        }
    }

    pub fn events(&self) -> &Arc<EventLog> {
        &self.events
    }

    pub fn serve_connection(&self, mut stream: LocalStream) -> BridgeResult<()> {
        let _slot = self.reserve_connection()?;
        stream.set_read_timeout(Some(AUTH_DEADLINE))?;
        stream.set_write_timeout(Some(AUTH_DEADLINE))?;
        let challenge = AuthChallenge::new(
            self.config.binding_id,
            self.config.binding_epoch,
            self.config.instance_id,
            self.config.project_id,
            self.capability.id(),
        )?;
        let issued = Instant::now();
        write_frame(&mut stream, &challenge)?;
        let proof: AuthProof = read_frame(&mut stream)?;
        if issued.elapsed() > AUTH_DEADLINE {
            return Err(BridgeError::Authentication(
                "authentication deadline exceeded".into(),
            ));
        }
        {
            let mut used = self
                .replayed_client_nonces
                .lock()
                .map_err(|_| BridgeError::Closed)?;
            if !used.insert(proof.client_nonce) {
                return Err(BridgeError::Authentication(
                    "client nonce was replayed".into(),
                ));
            }
            if used.len() > 4_096 {
                used.clear();
                used.insert(proof.client_nonce);
            }
        }
        verify_proof(&self.capability, &challenge, &proof)?;
        stream.set_read_timeout(Some(crate::IDLE_TIMEOUT))?;
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;

        let mut mutations = TokenBucket::new(20.0, 40.0);
        let mut reads = TokenBucket::new(100.0, 200.0);
        loop {
            let message: ClientMessage = match read_frame(&mut stream) {
                Ok(message) => message,
                Err(BridgeError::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::UnexpectedEof
                            | std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::BrokenPipe
                    ) =>
                {
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            match message {
                ClientMessage::Request(request) => {
                    mutations.take()?;
                    let response = self.handle_request(*request)?;
                    write_frame(&mut stream, &ServerMessage::Response(response))?;
                }
                ClientMessage::Subscribe { after_seq } => {
                    reads.take()?;
                    let snapshot = self.host.snapshot()?;
                    self.validate_snapshot(&snapshot)?;
                    write_frame(&mut stream, &ServerMessage::Snapshot(snapshot))?;
                    match self.events.subscribe(after_seq) {
                        Ok(subscription) => {
                            while let Some(event) = subscription.try_next()? {
                                write_frame(&mut stream, &ServerMessage::Event(event))?;
                            }
                        }
                        Err(BridgeError::ResyncRequired {
                            oldest_seq,
                            newest_seq,
                        }) => write_frame(
                            &mut stream,
                            &ServerMessage::ResyncRequired {
                                oldest_seq,
                                newest_seq,
                            },
                        )?,
                        Err(error) => return Err(error),
                    }
                }
                ClientMessage::Ping { nonce } => {
                    reads.take()?;
                    write_frame(&mut stream, &ServerMessage::Pong { nonce })?;
                }
            }
        }
    }

    pub fn handle_request(&self, request: RequestEnvelope) -> BridgeResult<ResponseEnvelope> {
        request.validate()?;
        if request.binding_id != self.config.binding_id
            || request.binding_epoch != self.config.binding_epoch
            || request.project_id != self.config.project_id
            || request.application != self.config.application
        {
            return Err(BridgeError::StaleBinding);
        }
        if let Some(cached) = self
            .cache
            .lock()
            .map_err(|_| BridgeError::Closed)?
            .lookup(request.request_id, &request)?
        {
            return Ok(cached.response);
        }
        let current = self.host.current_cursors()?;
        ensure_exact_cursors(&request.expected_cursors, &current)?;
        let body = self.host.apply(&request)?;
        let response = ResponseEnvelope {
            request_id: request.request_id,
            body,
        };
        response.validate()?;
        self.cache.lock().map_err(|_| BridgeError::Closed)?.insert(
            request.request_id,
            &request,
            response.clone(),
        )?;
        Ok(response)
    }

    pub fn append_event(
        &self,
        family: impl Into<String>,
        version: u32,
        payload: serde_json::Value,
    ) -> BridgeResult<u64> {
        self.events.append(family, version, payload)
    }

    fn validate_snapshot(&self, snapshot: &StateSnapshot) -> BridgeResult<()> {
        if snapshot.project_id != self.config.project_id
            || snapshot.binding_id != self.config.binding_id
            || snapshot.binding_epoch != self.config.binding_epoch
        {
            return Err(BridgeError::Protocol(
                "host returned a snapshot for another binding".into(),
            ));
        }
        Ok(())
    }

    fn reserve_connection(&self) -> BridgeResult<ConnectionSlot<'_>> {
        let previous = self
            .authenticated_connections
            .fetch_add(1, Ordering::AcqRel);
        if previous >= MAX_AUTHENTICATED_CONNECTIONS {
            self.authenticated_connections
                .fetch_sub(1, Ordering::AcqRel);
            return Err(BridgeError::Limit(
                "too many authenticated connections".into(),
            ));
        }
        Ok(ConnectionSlot {
            count: &self.authenticated_connections,
        })
    }
}

fn ensure_exact_cursors(
    expected: &[ExpectedCursor],
    current: &[ExpectedCursor],
) -> BridgeResult<()> {
    let lookup: std::collections::HashMap<TrackId, RevisionId> = current
        .iter()
        .map(|cursor| (cursor.track_id, cursor.revision_id))
        .collect();
    if expected
        .iter()
        .any(|cursor| lookup.get(&cursor.track_id) != Some(&cursor.revision_id))
    {
        return Err(BridgeError::CursorConflict);
    }
    Ok(())
}

struct ConnectionSlot<'a> {
    count: &'a AtomicUsize,
}

impl Drop for ConnectionSlot<'_> {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::AcqRel);
    }
}

struct TokenBucket {
    tokens: f64,
    rate: f64,
    burst: f64,
    refreshed: Instant,
}

impl TokenBucket {
    fn new(rate: f64, burst: f64) -> Self {
        Self {
            tokens: burst,
            rate,
            burst,
            refreshed: Instant::now(),
        }
    }

    fn take(&mut self) -> BridgeResult<()> {
        let now = Instant::now();
        self.tokens = (self.tokens + now.duration_since(self.refreshed).as_secs_f64() * self.rate)
            .min(self.burst);
        self.refreshed = now;
        if self.tokens < 1.0 {
            return Err(BridgeError::Limit("message rate exceeded".into()));
        }
        self.tokens -= 1.0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use spectrum_revisions::SessionId;

    use super::*;
    use crate::{ActionEnvelope, InteractionPolicy, PROTOCOL_FAMILY, PROTOCOL_VERSION, RequestId};

    struct MockHost {
        cursor: ExpectedCursor,
        calls: AtomicUsize,
        binding: ServerConfig,
    }

    impl BridgeHost for MockHost {
        fn current_cursors(&self) -> BridgeResult<Vec<ExpectedCursor>> {
            Ok(vec![self.cursor.clone()])
        }

        fn snapshot(&self) -> BridgeResult<StateSnapshot> {
            Ok(StateSnapshot {
                project_id: self.binding.project_id,
                binding_id: self.binding.binding_id,
                binding_epoch: self.binding.binding_epoch,
                cursors: vec![self.cursor.clone()],
                application_state: Default::default(),
            })
        }

        fn apply(&self, _: &RequestEnvelope) -> BridgeResult<ResponseBody> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(ResponseBody::Deferred)
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
            cursor: cursor.clone(),
            calls: AtomicUsize::new(0),
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
    fn exact_retry_is_applied_once() {
        let (host, server, request) = setup();
        server.handle_request(request.clone()).unwrap();
        server.handle_request(request).unwrap();
        assert_eq!(host.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn cursor_conflict_prevents_application() {
        let (host, server, mut request) = setup();
        request.expected_cursors[0].revision_id = RevisionId::new();
        assert!(matches!(
            server.handle_request(request),
            Err(BridgeError::CursorConflict)
        ));
        assert_eq!(host.calls.load(Ordering::Relaxed), 0);
    }

    #[cfg(unix)]
    #[test]
    fn authenticated_endpoint_round_trip() {
        let (host, _, request) = setup();
        let config = host.binding.clone();
        let capability = Capability::generate().unwrap();
        let client_capability = Capability::from_secret(capability.id(), *capability.secret());
        let server = Arc::new(BridgeServer::new(config, capability, host.clone()));
        let temporary = tempfile::tempdir().unwrap();
        fs::set_permissions(
            temporary.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let address = crate::EndpointAddress::Unix {
            path: temporary.path().join("roundtrip.sock"),
        };
        let listener = crate::LocalListener::bind(&address).unwrap();
        let server_thread = std::thread::spawn({
            let server = server.clone();
            move || {
                let (stream, _) = listener.accept().unwrap();
                server.serve_connection(stream).unwrap();
            }
        });
        let mut client =
            crate::BridgeClient::connect(&crate::ClientConfig::local(address), &client_capability)
                .unwrap();
        client.ping(91).unwrap();
        let response = client.request(request).unwrap();
        assert!(matches!(response.body, ResponseBody::Deferred));
        drop(client);
        server_thread.join().unwrap();
        assert_eq!(host.calls.load(Ordering::Relaxed), 1);
    }
}
