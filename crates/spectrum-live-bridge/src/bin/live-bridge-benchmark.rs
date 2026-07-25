use std::{
    error::Error,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use spectrum_live_bridge::{
    BindingId, BridgeClient, BridgeEventKind, BridgeHost, BridgeResult, BridgeServer, Capability,
    ClientConfig, EndpointAddress, ExpectedCursor, HostApplyOutcome, InstanceId, LocalListener,
    RequestEnvelope, ResponseBody, ServerConfig, ServerMessage, StateSnapshot,
};
use spectrum_revisions::{ProjectId, RevisionId, TrackId};

const HANDSHAKE_SAMPLES: usize = 100;
const PING_SAMPLES: usize = 100;
const EVENT_COUNT: usize = 1_000;
const CLIENT_COUNT: usize = 8;
const SLOW_EVENT_COUNT: usize = 100;

struct Results {
    authenticated_connect_p95: Duration,
    endpoint_ping_p95: Duration,
    wire_event_duration: Duration,
    retained_state_bytes: usize,
    slow_disconnect: Duration,
}

fn main() -> Result<(), Box<dyn Error>> {
    let strict = std::env::args().any(|argument| argument == "--strict");
    let hosted = std::env::args().any(|argument| argument == "--hosted");
    let results = run()?;
    println!(
        "live bridge endpoint: authenticated connect+ping p95 {:.3} ms · 1 KiB ping p95 {:.3} ms",
        milliseconds(results.authenticated_connect_p95),
        milliseconds(results.endpoint_ping_p95)
    );
    println!(
        "{EVENT_COUNT} wire-ordered 1 KiB events {:.2} ms · eight-client retained state {:.2} MiB · slow-client disconnect {:.3} ms",
        milliseconds(results.wire_event_duration),
        results.retained_state_bytes as f64 / 1_048_576.0,
        milliseconds(results.slow_disconnect)
    );
    if strict {
        enforce(&results, hosted)?;
        println!(
            "strict endpoint/server live bridge benchmark ({}): PASS",
            if hosted { "hosted" } else { "interactive" }
        );
    }
    Ok(())
}

fn run() -> Result<Results, Box<dyn Error>> {
    let expected_connections = HANDSHAKE_SAMPLES + 1 + CLIENT_COUNT;
    let harness = Harness::start(expected_connections)?;
    let mut handshake = Vec::with_capacity(HANDSHAKE_SAMPLES);
    for nonce in 0..HANDSHAKE_SAMPLES {
        let started = Instant::now();
        let mut client = harness.connect()?;
        client.ping(nonce as u64)?;
        handshake.push(started.elapsed());
        drop(client);
        wait_for_connections(&harness.server, 0, Duration::from_secs(1))?;
    }
    handshake.sort_unstable();
    wait_for_connections(&harness.server, 0, Duration::from_secs(2))?;

    let mut event_client = harness.connect()?;
    let mut ping = Vec::with_capacity(PING_SAMPLES);
    for nonce in 0..PING_SAMPLES {
        let started = Instant::now();
        event_client.ping_with_padding(nonce as u64, "x".repeat(960))?;
        ping.push(started.elapsed());
    }
    ping.sort_unstable();

    let snapshot = event_client.subscribe(0)?;
    if snapshot.current_event_seq != 0 {
        return Err("fresh benchmark binding reported a nonzero event sequence".into());
    }
    let event_started = Instant::now();
    for index in 0..EVENT_COUNT {
        harness.server.append_event(benchmark_event(index, 960))?;
        match event_client.read_subscription_message()? {
            ServerMessage::Event(event) if event.seq == index as u64 + 1 => {}
            message => {
                return Err(format!("wire event gap at {}, got {message:?}", index + 1).into());
            }
        }
    }
    let wire_event_duration = event_started.elapsed();
    drop(event_client);
    wait_for_connections(&harness.server, 0, Duration::from_secs(2))?;

    let mut clients = (0..CLIENT_COUNT)
        .map(|_| harness.connect())
        .collect::<BridgeResult<Vec<_>>>()?;
    for client in &mut clients {
        let snapshot = client.subscribe(EVENT_COUNT as u64)?;
        if snapshot.current_event_seq != EVENT_COUNT as u64 {
            return Err("eight-client snapshot did not pin the current event sequence".into());
        }
    }
    let slow_started = Instant::now();
    for index in 0..SLOW_EVENT_COUNT {
        harness
            .server
            .append_event(benchmark_event(EVENT_COUNT + index, 3_900))?;
        for client in clients.iter_mut().skip(1) {
            match client.read_subscription_message()? {
                ServerMessage::Event(_) => {}
                message => return Err(format!("healthy subscriber got {message:?}").into()),
            }
        }
    }
    wait_for_connections(
        &harness.server,
        CLIENT_COUNT - 1,
        Duration::from_millis(100),
    )?;
    let slow_disconnect = slow_started.elapsed();
    let retained_state_bytes = harness.server.retained_state_bytes();
    drop(clients);
    wait_for_connections(&harness.server, 0, Duration::from_secs(2))?;
    harness.finish()?;

    Ok(Results {
        authenticated_connect_p95: percentile(&handshake, 95),
        endpoint_ping_p95: percentile(&ping, 95),
        wire_event_duration,
        retained_state_bytes,
        slow_disconnect,
    })
}

fn benchmark_event(index: usize, padding: usize) -> BridgeEventKind {
    BridgeEventKind::InteractionBegan {
        interaction_id: format!("{index}-{}", "x".repeat(padding)),
        interaction_kind: "endpoint-benchmark".into(),
    }
}

fn enforce(results: &Results, hosted: bool) -> Result<(), Box<dyn Error>> {
    let handshake_limit = if hosted { 25.0 } else { 5.0 };
    let ping_limit = if hosted { 10.0 } else { 2.0 };
    require_under(
        "endpoint authenticated connect+ping p95",
        results.authenticated_connect_p95,
        handshake_limit,
    )?;
    require_under(
        "endpoint 1 KiB ping p95",
        results.endpoint_ping_p95,
        ping_limit,
    )?;
    if results.retained_state_bytes > 16 * 1024 * 1024 {
        return Err("measured eight-client retained state exceeds 16 MiB".into());
    }
    require_under("slow-client disconnect", results.slow_disconnect, 100.0)?;
    Ok(())
}

fn require_under(label: &str, actual: Duration, limit_ms: f64) -> Result<(), Box<dyn Error>> {
    if milliseconds(actual) > limit_ms {
        return Err(format!(
            "{label} took {:.3} ms, above {limit_ms:.1} ms",
            milliseconds(actual)
        )
        .into());
    }
    Ok(())
}

struct BenchmarkHost {
    config: ServerConfig,
    cursor: Mutex<ExpectedCursor>,
}

impl BridgeHost for BenchmarkHost {
    fn current_cursors(&self) -> BridgeResult<Vec<ExpectedCursor>> {
        Ok(vec![
            self.cursor
                .lock()
                .map_err(|_| spectrum_live_bridge::BridgeError::Closed)?
                .clone(),
        ])
    }

    fn snapshot(&self) -> BridgeResult<StateSnapshot> {
        Ok(StateSnapshot {
            project_id: self.config.project_id,
            binding_id: self.config.binding_id,
            binding_epoch: self.config.binding_epoch,
            cursors: self.current_cursors()?,
            current_event_seq: 0,
            application_protocols: Default::default(),
            application_state: Default::default(),
        })
    }

    fn apply_if_current(&self, _request: &RequestEnvelope) -> BridgeResult<HostApplyOutcome> {
        Ok(HostApplyOutcome::Applied(ResponseBody::Applied {
            result: serde_json::json!({"ok": true}),
            cursors: self.current_cursors()?,
        }))
    }
}

struct Harness {
    server: Arc<BridgeServer<BenchmarkHost>>,
    capability: Capability,
    address: EndpointAddress,
    acceptor: thread::JoinHandle<()>,
    cleanup: Option<PathBuf>,
}

impl Harness {
    fn start(expected_connections: usize) -> Result<Self, Box<dyn Error>> {
        let cleanup = runtime_directory()?;
        let address = endpoint_address(cleanup.as_ref());
        let listener = LocalListener::bind(&address)?;
        let capability = Capability::generate()?;
        let config = ServerConfig {
            application: "spectrum.benchmark".into(),
            project_id: ProjectId::new(),
            instance_id: InstanceId::new(),
            binding_id: BindingId::new(),
            binding_epoch: 1,
        };
        let host = Arc::new(BenchmarkHost {
            config: config.clone(),
            cursor: Mutex::new(ExpectedCursor {
                track_id: TrackId::new(),
                revision_id: RevisionId::new(),
            }),
        });
        let server = Arc::new(BridgeServer::new(config, capability.duplicate(), host));
        let acceptor = {
            let server = Arc::clone(&server);
            thread::spawn(move || {
                let mut connections = Vec::with_capacity(expected_connections);
                for _ in 0..expected_connections {
                    let Ok((stream, _)) = listener.accept() else {
                        return;
                    };
                    let server = Arc::clone(&server);
                    connections.push(thread::spawn(move || {
                        let _ = server.serve_connection(stream);
                    }));
                }
                for connection in connections {
                    let _ = connection.join();
                }
            })
        };
        Ok(Self {
            server,
            capability,
            address,
            acceptor,
            cleanup,
        })
    }

    fn connect(&self) -> BridgeResult<BridgeClient> {
        BridgeClient::connect(
            &ClientConfig {
                endpoint: self.address.clone(),
                initial_backoff: Duration::from_millis(1),
                maximum_backoff: Duration::from_millis(10),
                attempts: 5,
            },
            &self.capability,
        )
    }

    fn finish(self) -> Result<(), Box<dyn Error>> {
        self.acceptor
            .join()
            .map_err(|_| "benchmark acceptor panicked")?;
        if let Some(path) = self.cleanup {
            fs::remove_dir_all(path)?;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn runtime_directory() -> Result<Option<PathBuf>, Box<dyn Error>> {
    use std::os::unix::fs::DirBuilderExt;

    let random = uuid::Uuid::new_v4().simple().to_string();
    let path = PathBuf::from("/tmp").join(format!("slb-{}-{}", std::process::id(), &random[..8]));
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700).create(&path)?;
    Ok(Some(path))
}

#[cfg(windows)]
fn runtime_directory() -> Result<Option<PathBuf>, Box<dyn Error>> {
    Ok(None)
}

#[cfg(unix)]
fn endpoint_address(directory: Option<&PathBuf>) -> EndpointAddress {
    EndpointAddress::Unix {
        path: directory
            .expect("Unix benchmark directory")
            .join("bridge.sock"),
    }
}

#[cfg(windows)]
fn endpoint_address(_directory: Option<&PathBuf>) -> EndpointAddress {
    EndpointAddress::WindowsPipe {
        name: format!(r"\\.\pipe\spectrum-live-{}", uuid::Uuid::new_v4()),
    }
}

fn wait_for_connections(
    server: &BridgeServer<BenchmarkHost>,
    expected_maximum: usize,
    timeout: Duration,
) -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    while server.active_connection_count() > expected_maximum {
        if started.elapsed() >= timeout {
            return Err(format!(
                "connection count stayed at {}, above {expected_maximum}",
                server.active_connection_count()
            )
            .into());
        }
        thread::sleep(Duration::from_micros(100));
    }
    Ok(())
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    let index = (samples.len() - 1) * percentile / 100;
    samples[index]
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
