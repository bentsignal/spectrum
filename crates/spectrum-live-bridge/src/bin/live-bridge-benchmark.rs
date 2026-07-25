use std::{
    error::Error,
    io::Cursor,
    time::{Duration, Instant},
};

use spectrum_live_bridge::{
    AuthChallenge, BindingId, BridgeError, Capability, CapabilityId, ClientMessage, EventLog,
    InstanceId, read_frame, verify_proof, write_frame,
};
use spectrum_revisions::ProjectId;

const SAMPLES: usize = 2_000;
const EVENT_COUNT: usize = 1_000;

struct Results {
    handshake_p95: Duration,
    ping_p95: Duration,
    event_duration: Duration,
    retained_estimate: usize,
    slow_resync: Duration,
}

fn main() -> Result<(), Box<dyn Error>> {
    let strict = std::env::args().any(|argument| argument == "--strict");
    let hosted = std::env::args().any(|argument| argument == "--hosted");
    let results = run()?;
    println!(
        "live bridge: handshake p95 {:.3} ms · 1 KiB ping p95 {:.3} ms",
        milliseconds(results.handshake_p95),
        milliseconds(results.ping_p95)
    );
    println!(
        "{EVENT_COUNT} ordered events {:.2} ms · retained estimate {:.2} MiB · slow resync {:.3} ms",
        milliseconds(results.event_duration),
        results.retained_estimate as f64 / 1_048_576.0,
        milliseconds(results.slow_resync)
    );
    if strict {
        enforce(&results, hosted)?;
        println!(
            "strict live bridge benchmark ({}): PASS",
            if hosted { "hosted" } else { "interactive" }
        );
    }
    Ok(())
}

fn run() -> Result<Results, Box<dyn Error>> {
    let capability = Capability::generate()?;
    let binding = BindingId::new();
    let instance = InstanceId::new();
    let project = ProjectId::new();
    let capability_id: CapabilityId = capability.id();
    let mut handshake = Vec::with_capacity(SAMPLES);
    let mut ping = Vec::with_capacity(SAMPLES);
    let ping_message = ClientMessage::Ping { nonce: 7 };
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let challenge = AuthChallenge::new(binding, 1, instance, project, capability_id)?;
        let proof = capability.prove(&challenge)?;
        verify_proof(&capability, &challenge, &proof)?;
        handshake.push(started.elapsed());

        let payload = serde_json::json!({
            "message": ping_message,
            "padding": "x".repeat(960)
        });
        let started = Instant::now();
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &payload)?;
        let decoded: serde_json::Value = read_frame(&mut Cursor::new(bytes))?;
        std::hint::black_box(decoded);
        ping.push(started.elapsed());
    }
    handshake.sort_unstable();
    ping.sort_unstable();

    let events = EventLog::new();
    let subscription = events.subscribe(0)?;
    let event_started = Instant::now();
    for index in 0..EVENT_COUNT {
        events.append(
            "benchmark.event",
            1,
            serde_json::json!({"index": index, "padding": "x".repeat(960)}),
        )?;
        let event = subscription
            .try_next()?
            .ok_or("ordered subscriber ended early")?;
        let expected = index as u64 + 1;
        if event.seq != expected {
            return Err(format!("event gap at {expected}, got {}", event.seq).into());
        }
    }
    let event_duration = event_started.elapsed();
    let retained_estimate = EVENT_COUNT * 1_100 + 8 * 2_048;

    let slow = events.subscribe(EVENT_COUNT as u64)?;
    let healthy = events.subscribe(EVENT_COUNT as u64)?;
    let slow_started = Instant::now();
    for index in 0..300_u64 {
        events.append(
            "benchmark.event",
            1,
            serde_json::json!({"index": index, "padding": "x".repeat(3_900)}),
        )?;
        let _ = healthy.try_next()?.ok_or("healthy subscriber stalled")?;
    }
    match slow.try_next() {
        Err(BridgeError::ResyncRequired { .. }) => {}
        _ => return Err("slow subscriber was not forced to resync".into()),
    }
    let slow_resync = slow_started.elapsed();

    Ok(Results {
        handshake_p95: percentile(&handshake, 95),
        ping_p95: percentile(&ping, 95),
        event_duration,
        retained_estimate,
        slow_resync,
    })
}

fn enforce(results: &Results, hosted: bool) -> Result<(), Box<dyn Error>> {
    let handshake_limit = if hosted { 25.0 } else { 5.0 };
    let ping_limit = if hosted { 10.0 } else { 2.0 };
    require_under("handshake p95", results.handshake_p95, handshake_limit)?;
    require_under("1 KiB ping p95", results.ping_p95, ping_limit)?;
    if results.retained_estimate > 16 * 1024 * 1024 {
        return Err("8-client retained-memory estimate exceeds 16 MiB".into());
    }
    require_under("slow-subscriber isolation", results.slow_resync, 100.0)?;
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

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    let index = (samples.len() - 1) * percentile / 100;
    samples[index]
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
