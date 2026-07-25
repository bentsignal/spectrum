use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

use spectrum_revisions::{ProjectId, RevisionId, TrackId};

use crate::framing::read_frame_counted;
use crate::{
    AUTH_DEADLINE, AuthChallenge, AuthProof, BindingId, BridgeError, BridgeEventKind, BridgeResult,
    Capability, ClientMessage, EventLog, ExpectedCursor, InstanceId, LocalStream,
    MAX_AUTHENTICATED_CONNECTIONS, MAX_DEFERRED_PER_CONNECTION, MAX_IN_FLIGHT_PER_CONNECTION,
    MAX_INGRESS_BURST_BYTES, MAX_INGRESS_BYTES_PER_SECOND, MAX_QUEUED_BYTES_PER_CONNECTION,
    MAX_QUEUED_BYTES_PER_HOST, MAX_SUBSCRIPTIONS_PER_CONNECTION, RequestCache, RequestEnvelope,
    RequestLookup, ResponseBody, ResponseEnvelope, ServerMessage, StateSnapshot, Subscription,
    read_frame, verify_proof, write_frame,
};

const CONNECTION_POLL: Duration = Duration::from_micros(250);
const CONNECTION_WRITE_TIMEOUT: Duration = Duration::from_millis(50);

/// Result of the host's mandatory second cursor check and mutation.
///
/// Implementations must compare every request expectation and perform the
/// mutation in the same app-thread/store critical section. Returning
/// `Conflict` proves that no requested mutation or durable event occurred.
pub enum HostApplyOutcome {
    Applied(ResponseBody),
    Conflict(Vec<ExpectedCursor>),
}

pub trait BridgeHost: Send + Sync + 'static {
    fn current_cursors(&self) -> BridgeResult<Vec<ExpectedCursor>>;
    fn snapshot(&self) -> BridgeResult<StateSnapshot>;
    fn apply_if_current(&self, request: &RequestEnvelope) -> BridgeResult<HostApplyOutcome>;
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub application: String,
    pub project_id: ProjectId,
    pub instance_id: InstanceId,
    pub binding_id: BindingId,
    pub binding_epoch: u64,
}

struct MutationState {
    cache: RequestCache,
}

pub struct BridgeServer<H> {
    config: ServerConfig,
    capability: Capability,
    host: Arc<H>,
    events: Arc<EventLog>,
    mutations: Mutex<MutationState>,
    replayed_client_nonces: Mutex<HashSet<[u8; 32]>>,
    authenticated_connections: AtomicUsize,
    inbound_queued: Arc<AtomicUsize>,
}

impl<H: BridgeHost> BridgeServer<H> {
    pub fn new(config: ServerConfig, capability: Capability, host: Arc<H>) -> Self {
        Self {
            config,
            capability,
            host,
            events: Arc::new(EventLog::new()),
            mutations: Mutex::new(MutationState {
                cache: RequestCache::default(),
            }),
            replayed_client_nonces: Mutex::new(HashSet::new()),
            authenticated_connections: AtomicUsize::new(0),
            inbound_queued: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn events(&self) -> &Arc<EventLog> {
        &self.events
    }

    pub fn serve_connection(&self, mut stream: LocalStream) -> BridgeResult<()> {
        let _slot = self.reserve_connection()?;
        self.authenticate(&mut stream)?;
        stream.set_read_timeout(Some(crate::IDLE_TIMEOUT))?;
        stream.set_write_timeout(Some(CONNECTION_WRITE_TIMEOUT))?;

        let mut reader = stream.try_clone()?;
        reader.set_read_timeout(Some(crate::IDLE_TIMEOUT))?;
        let (sender, receiver) = mpsc::sync_channel(MAX_IN_FLIGHT_PER_CONNECTION);
        let connection_inbound = Arc::new(AtomicUsize::new(0));
        thread::Builder::new()
            .name("spectrum-live-reader".into())
            .spawn({
                let connection_inbound = Arc::clone(&connection_inbound);
                let host_inbound = Arc::clone(&self.inbound_queued);
                move || read_connection(&mut reader, sender, connection_inbound, host_inbound)
            })
            .map_err(BridgeError::Io)?;

        self.connection_loop(stream, receiver)
    }

    pub fn handle_request(&self, request: RequestEnvelope) -> BridgeResult<ResponseEnvelope> {
        request.validate()?;
        self.validate_binding(&request)?;
        let mut state = self.mutations.lock().map_err(|_| BridgeError::Closed)?;
        match state.cache.lookup(request.request_id, &request)? {
            RequestLookup::Cached(cached) => return Ok(cached.response),
            RequestLookup::OutcomeUnknown => {
                return Ok(ResponseEnvelope {
                    request_id: request.request_id,
                    body: ResponseBody::OutcomeUnknown,
                });
            }
            RequestLookup::Miss => {}
        }

        let current = normalize_cursors(self.host.current_cursors()?)?;
        let body = if ensure_exact_cursors(&request.expected_cursors, &current).is_err() {
            ResponseBody::Conflict { current }
        } else {
            match self.host.apply_if_current(&request)? {
                HostApplyOutcome::Applied(body) => body,
                HostApplyOutcome::Conflict(current) => ResponseBody::Conflict {
                    current: normalize_cursors(current)?,
                },
            }
        };
        let response = ResponseEnvelope {
            request_id: request.request_id,
            body,
        };
        response.validate()?;
        state
            .cache
            .insert(request.request_id, &request, response.clone())?;
        Ok(response)
    }

    pub fn append_event(&self, event: BridgeEventKind) -> BridgeResult<u64> {
        self.events.append(event)
    }

    pub fn retained_state_bytes(&self) -> usize {
        let cache = self
            .mutations
            .lock()
            .ok()
            .map_or(0, |state| state.cache.retained_bytes());
        cache
            .saturating_add(self.events.retained_state_bytes())
            .saturating_add(self.inbound_queued.load(Ordering::Acquire))
    }

    pub fn active_connection_count(&self) -> usize {
        self.authenticated_connections.load(Ordering::Acquire)
    }

    fn authenticate(&self, stream: &mut LocalStream) -> BridgeResult<()> {
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
        write_frame(stream, &challenge)?;
        let proof: AuthProof = read_frame(stream)?;
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
        verify_proof(&self.capability, &challenge, &proof)
    }

    fn connection_loop(
        &self,
        mut writer: LocalStream,
        receiver: Receiver<Inbound>,
    ) -> BridgeResult<()> {
        let mut mutations = TokenBucket::new(20.0, 40.0);
        let mut reads = TokenBucket::new(100.0, 200.0);
        let mut subscriptions = Vec::<Subscription>::new();
        let connection_queued = Arc::new(AtomicUsize::new(0));
        let mut deferred = 0_usize;
        let mut rate_offenses = 0_u8;
        let mut input_closed = false;

        loop {
            match receiver.recv_timeout(CONNECTION_POLL) {
                Ok(Inbound::Message { message, charge }) => {
                    drop(charge);
                    let rate = match &message {
                        ClientMessage::Request(_) => mutations.take(),
                        ClientMessage::Subscribe { .. } | ClientMessage::Ping { .. } => {
                            reads.take()
                        }
                    };
                    if let Err(retry_after) = rate {
                        rate_offenses = rate_offenses.saturating_add(1);
                        let disconnect = rate_offenses > 1;
                        write_rate_limit(&mut writer, retry_after, disconnect)?;
                        if disconnect {
                            return Ok(());
                        }
                        continue;
                    }
                    match message {
                        ClientMessage::Request(request) => {
                            if deferred >= MAX_DEFERRED_PER_CONNECTION {
                                rate_offenses = rate_offenses.saturating_add(1);
                                let disconnect = rate_offenses > 1;
                                write_rate_limit(
                                    &mut writer,
                                    Duration::from_millis(250),
                                    disconnect,
                                )?;
                                if disconnect {
                                    return Ok(());
                                }
                                continue;
                            }
                            let response = self.handle_request(*request)?;
                            if matches!(response.body, ResponseBody::Deferred) {
                                deferred += 1;
                            }
                            write_frame(&mut writer, &ServerMessage::Response(response))?;
                        }
                        ClientMessage::Subscribe { after_seq } => {
                            if subscriptions.len() >= MAX_SUBSCRIPTIONS_PER_CONNECTION {
                                rate_offenses = rate_offenses.saturating_add(1);
                                let disconnect = rate_offenses > 1;
                                write_rate_limit(
                                    &mut writer,
                                    Duration::from_millis(250),
                                    disconnect,
                                )?;
                                if disconnect {
                                    return Ok(());
                                }
                                continue;
                            }
                            let mut snapshot = self.host.snapshot()?;
                            self.validate_snapshot(&snapshot)?;
                            snapshot.current_event_seq = self.events.current_seq();
                            write_frame(&mut writer, &ServerMessage::Snapshot(snapshot))?;
                            match self
                                .events
                                .subscribe_with_budget(after_seq, Arc::clone(&connection_queued))
                            {
                                Ok(subscription) => subscriptions.push(subscription),
                                Err(BridgeError::ResyncRequired {
                                    oldest_seq,
                                    newest_seq,
                                }) => write_frame(
                                    &mut writer,
                                    &ServerMessage::ResyncRequired {
                                        oldest_seq,
                                        newest_seq,
                                    },
                                )?,
                                Err(error) => return Err(error),
                            }
                        }
                        ClientMessage::Ping { nonce, .. } => {
                            write_frame(&mut writer, &ServerMessage::Pong { nonce })?;
                        }
                    }
                }
                Ok(Inbound::RateLimited { retry_after }) => {
                    rate_offenses = rate_offenses.saturating_add(1);
                    let disconnect = rate_offenses > 1;
                    write_rate_limit(&mut writer, retry_after, disconnect)?;
                    if disconnect {
                        return Ok(());
                    }
                }
                Ok(Inbound::Error(error)) => return connection_error(error),
                Ok(Inbound::Closed) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    input_closed = true;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }

            drain_subscriptions(&mut writer, &mut subscriptions)?;
            if input_closed {
                return Ok(());
            }
        }
    }

    fn validate_binding(&self, request: &RequestEnvelope) -> BridgeResult<()> {
        if request.binding_id != self.config.binding_id
            || request.binding_epoch != self.config.binding_epoch
            || request.project_id != self.config.project_id
            || request.application != self.config.application
        {
            return Err(BridgeError::StaleBinding);
        }
        Ok(())
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
        snapshot.validate()
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

fn read_connection(
    stream: &mut LocalStream,
    sender: SyncSender<Inbound>,
    connection_queued: Arc<AtomicUsize>,
    host_queued: Arc<AtomicUsize>,
) {
    let mut ingress = ByteTokenBucket::new(
        MAX_INGRESS_BYTES_PER_SECOND as f64,
        MAX_INGRESS_BURST_BYTES as f64,
    );
    let mut violations = 0_u8;
    loop {
        match read_frame_counted::<_, ClientMessage>(stream) {
            Ok((message, bytes)) => {
                if let Err(retry_after) = ingress.take(bytes) {
                    violations = violations.saturating_add(1);
                    if sender.send(Inbound::RateLimited { retry_after }).is_err() {
                        return;
                    }
                    if violations > 1 {
                        return;
                    }
                    continue;
                }
                if !reserve_atomic(&connection_queued, bytes, MAX_QUEUED_BYTES_PER_CONNECTION) {
                    violations = violations.saturating_add(1);
                    let _ = sender.send(Inbound::RateLimited {
                        retry_after: Duration::from_millis(10),
                    });
                    if violations > 1 {
                        return;
                    }
                    continue;
                }
                if !reserve_atomic(&host_queued, bytes, MAX_QUEUED_BYTES_PER_HOST) {
                    connection_queued.fetch_sub(bytes, Ordering::AcqRel);
                    violations = violations.saturating_add(1);
                    let _ = sender.send(Inbound::RateLimited {
                        retry_after: Duration::from_millis(10),
                    });
                    if violations > 1 {
                        return;
                    }
                    continue;
                }
                let charge = QueueCharge {
                    bytes,
                    connection: Arc::clone(&connection_queued),
                    host: Arc::clone(&host_queued),
                };
                if sender.send(Inbound::Message { message, charge }).is_err() {
                    return;
                }
            }
            Err(BridgeError::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::BrokenPipe
                ) =>
            {
                let _ = sender.send(Inbound::Closed);
                return;
            }
            Err(error) => {
                let _ = sender.send(Inbound::Error(error));
                return;
            }
        }
    }
}

fn drain_subscriptions(
    writer: &mut LocalStream,
    subscriptions: &mut Vec<Subscription>,
) -> BridgeResult<()> {
    let mut index = 0;
    while index < subscriptions.len() {
        match subscriptions[index].try_next() {
            Ok(Some(event)) => {
                write_frame(writer, &ServerMessage::Event(event))?;
                index += 1;
            }
            Ok(None) => index += 1,
            Err(BridgeError::ResyncRequired {
                oldest_seq,
                newest_seq,
            }) => {
                write_frame(
                    writer,
                    &ServerMessage::ResyncRequired {
                        oldest_seq,
                        newest_seq,
                    },
                )?;
                subscriptions.swap_remove(index);
            }
            Err(BridgeError::Closed) => {
                subscriptions.swap_remove(index);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn connection_error(error: BridgeError) -> BridgeResult<()> {
    match error {
        BridgeError::Io(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::WouldBlock
            ) =>
        {
            Ok(())
        }
        error => Err(error),
    }
}

fn write_rate_limit(
    writer: &mut LocalStream,
    retry_after: Duration,
    disconnect: bool,
) -> BridgeResult<()> {
    write_frame(
        writer,
        &ServerMessage::RateLimited {
            retry_after_millis: u64::try_from(retry_after.as_millis()).unwrap_or(u64::MAX),
            disconnect,
        },
    )
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

fn normalize_cursors(mut cursors: Vec<ExpectedCursor>) -> BridgeResult<Vec<ExpectedCursor>> {
    cursors.sort_by_key(|cursor| *cursor.track_id.as_bytes());
    if cursors
        .windows(2)
        .any(|pair| pair[0].track_id == pair[1].track_id)
    {
        return Err(BridgeError::Protocol(
            "host returned duplicate track cursors".into(),
        ));
    }
    Ok(cursors)
}

fn reserve_atomic(counter: &AtomicUsize, bytes: usize, maximum: usize) -> bool {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current
                .checked_add(bytes)
                .filter(|updated| *updated <= maximum)
        })
        .is_ok()
}

enum Inbound {
    Message {
        message: ClientMessage,
        charge: QueueCharge,
    },
    RateLimited {
        retry_after: Duration,
    },
    Error(BridgeError),
    Closed,
}

struct QueueCharge {
    bytes: usize,
    connection: Arc<AtomicUsize>,
    host: Arc<AtomicUsize>,
}

impl Drop for QueueCharge {
    fn drop(&mut self) {
        self.connection.fetch_sub(self.bytes, Ordering::AcqRel);
        self.host.fetch_sub(self.bytes, Ordering::AcqRel);
    }
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

    fn take(&mut self) -> Result<(), Duration> {
        let now = Instant::now();
        self.tokens = (self.tokens + now.duration_since(self.refreshed).as_secs_f64() * self.rate)
            .min(self.burst);
        self.refreshed = now;
        if self.tokens < 1.0 {
            return Err(Duration::from_secs_f64((1.0 - self.tokens) / self.rate));
        }
        self.tokens -= 1.0;
        Ok(())
    }
}

struct ByteTokenBucket(TokenBucket);

impl ByteTokenBucket {
    fn new(rate: f64, burst: f64) -> Self {
        Self(TokenBucket::new(rate, burst))
    }

    fn take(&mut self, bytes: usize) -> Result<(), Duration> {
        let now = Instant::now();
        self.0.tokens = (self.0.tokens
            + now.duration_since(self.0.refreshed).as_secs_f64() * self.0.rate)
            .min(self.0.burst);
        self.0.refreshed = now;
        let required = bytes as f64;
        if self.0.tokens < required {
            return Err(Duration::from_secs_f64(
                (required - self.0.tokens) / self.0.rate,
            ));
        }
        self.0.tokens -= required;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
