use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use spectrum_revisions::{
    Actor, ActorKind, AppendRevision, Compatibility, Encoding, LiveRevisionStore, NewProject,
    Payload, SessionId,
};
use uuid::Uuid;

const ACTION_COUNT: usize = 1_000;
const SNAPSHOT_INTERVAL: usize = 100;
const SNAPSHOT_BYTES: usize = 64 * 1024;
const ASSET_BYTES: usize = 32 * 1024 * 1024;

struct V1Compatibility;

impl Compatibility for V1Compatibility {
    fn supports_snapshot(&self, encoding: &Encoding) -> bool {
        encoding.family == "benchmark.snapshot" && encoding.version == 1
    }

    fn supports_operations(&self, encoding: &Encoding) -> bool {
        encoding.family == "benchmark.operations" && encoding.version == 1
    }
}

struct TemporaryProject {
    directory: PathBuf,
}

impl TemporaryProject {
    fn new() -> Result<Self, Box<dyn Error>> {
        let directory =
            std::env::temp_dir().join(format!("spectrum-revision-benchmark-{}", Uuid::new_v4()));
        fs::create_dir(&directory)?;
        Ok(Self { directory })
    }

    fn path(&self) -> PathBuf {
        self.directory.join("benchmark.prism")
    }
}

impl Drop for TemporaryProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[derive(Debug)]
struct Results {
    commit_total: Duration,
    commit_p50: Duration,
    commit_p95: Duration,
    commit_p99: Duration,
    commit_max: Duration,
    asset_write: Duration,
    asset_read: Duration,
    post_asset_commit: Duration,
    reopen_and_plan: Duration,
    integrity_check: Duration,
    file_bytes: u64,
}

fn main() -> Result<(), Box<dyn Error>> {
    let strict = std::env::args().any(|argument| argument == "--strict");
    let temporary = TemporaryProject::new()?;
    let path = temporary.path();
    let results = run(&path)?;
    print_results(&results);
    if strict {
        enforce(&results)?;
        println!("strict revision benchmark: PASS");
    }
    Ok(())
}

fn run(path: &Path) -> Result<Results, Box<dyn Error>> {
    let session_id = SessionId::new();
    let cache = path
        .parent()
        .ok_or("benchmark path has no parent")?
        .join("cache");
    let (mut live, project) = LiveRevisionStore::create(
        path,
        &cache,
        NewProject {
            application_id: "spectrum.benchmark".into(),
            application_version: "1.0.0".into(),
            actor: Actor {
                id: "benchmark:human".into(),
                display_name: "Benchmark".into(),
                kind: ActorKind::Human,
            },
            session_id,
            root_label: Some("Benchmark root".into()),
            track_kind: "benchmark.document".into(),
            track_label: "Benchmark document".into(),
            initial_snapshots: vec![Payload::new(
                Encoding::new("benchmark.snapshot", 1),
                vec![0; SNAPSHOT_BYTES],
            )],
            assets: Vec::new(),
        },
    )?;
    let mut cursor = project.root_revision;
    let mut commit_latencies = Vec::with_capacity(ACTION_COUNT);
    let commit_start = Instant::now();
    for index in 1..=ACTION_COUNT {
        let command = format!("move object {index} from x={} to x={index}", index - 1);
        let snapshots = if index % SNAPSHOT_INTERVAL == 0 {
            vec![Payload::new(
                Encoding::new("benchmark.snapshot", 1),
                snapshot_bytes(index),
            )]
        } else {
            Vec::new()
        };
        let started = Instant::now();
        cursor = live
            .mutate(|store| {
                store.append(AppendRevision {
                    track_id: project.default_track_id,
                    session_id,
                    expected_parent: cursor,
                    application_version: "1.0.0".into(),
                    label: Some(format!("Move object {index}")),
                    command_count: 1,
                    operation_payloads: vec![Payload::new(
                        Encoding::new("benchmark.operations", 1),
                        command.into_bytes(),
                    )],
                    snapshots,
                    assets: Vec::new(),
                })
            })?
            .id;
        commit_latencies.push(started.elapsed());
    }
    let commit_total = commit_start.elapsed();

    let asset = deterministic_bytes(ASSET_BYTES);
    let asset_write_start = Instant::now();
    let asset_id = live.mutate(|store| store.put_asset("application/octet-stream", &asset))?;
    let asset_write = asset_write_start.elapsed();
    let asset_read_start = Instant::now();
    let loaded = live
        .store()
        .asset(asset_id)?
        .ok_or("benchmark asset disappeared")?;
    let asset_read = asset_read_start.elapsed();
    if loaded != asset {
        return Err("benchmark asset changed".into());
    }

    let post_asset_start = Instant::now();
    cursor = live
        .mutate(|store| {
            store.append(AppendRevision {
                track_id: project.default_track_id,
                session_id,
                expected_parent: cursor,
                application_version: "1.0.0".into(),
                label: Some("Edit after large asset".into()),
                command_count: 1,
                operation_payloads: vec![Payload::new(
                    Encoding::new("benchmark.operations", 1),
                    b"small edit after large asset".to_vec(),
                )],
                snapshots: Vec::new(),
                assets: Vec::new(),
            })
        })?
        .id;
    let post_asset_commit = post_asset_start.elapsed();

    live.publish()?;
    drop(live);
    let reopen_start = Instant::now();
    let reopened = LiveRevisionStore::open(path, &cache)?;
    let plan = reopened.store().replay_plan(cursor, &V1Compatibility)?;
    let reopen_and_plan = reopen_start.elapsed();
    if plan.steps.len() >= SNAPSHOT_INTERVAL {
        return Err("snapshot did not bound replay tail".into());
    }
    let integrity_start = Instant::now();
    reopened.store().verify_integrity()?;
    let integrity_check = integrity_start.elapsed();
    drop(reopened);

    commit_latencies.sort_unstable();
    Ok(Results {
        commit_total,
        commit_p50: percentile(&commit_latencies, 50),
        commit_p95: percentile(&commit_latencies, 95),
        commit_p99: percentile(&commit_latencies, 99),
        commit_max: *commit_latencies.last().unwrap_or(&Duration::ZERO),
        asset_write,
        asset_read,
        post_asset_commit,
        reopen_and_plan,
        integrity_check,
        file_bytes: fs::metadata(path)?.len(),
    })
}

fn enforce(results: &Results) -> Result<(), Box<dyn Error>> {
    require_under("p95 durable commit", results.commit_p95, 25)?;
    require_under("p99 durable commit", results.commit_p99, 50)?;
    require_under("maximum durable commit", results.commit_max, 250)?;
    require_under("32 MiB asset write", results.asset_write, 2_000)?;
    require_under("32 MiB asset read", results.asset_read, 1_000)?;
    require_under("post-asset durable commit", results.post_asset_commit, 100)?;
    require_under("reopen and replay-plan", results.reopen_and_plan, 250)?;
    require_under("integrity check", results.integrity_check, 2_000)?;
    if results.file_bytes > 40 * 1024 * 1024 {
        return Err(format!(
            "portable project grew to {:.1} MiB, above the 40 MiB strict limit",
            mib(results.file_bytes)
        )
        .into());
    }
    Ok(())
}

fn require_under(label: &str, actual: Duration, limit_ms: u128) -> Result<(), Box<dyn Error>> {
    if actual.as_millis() > limit_ms {
        return Err(format!(
            "{label} took {:.2} ms, above the {limit_ms} ms strict limit",
            milliseconds(actual)
        )
        .into());
    }
    Ok(())
}

fn print_results(results: &Results) {
    println!("revisions: {}", ACTION_COUNT + 1);
    println!(
        "durable commits: total {:.1} ms · p50 {:.2} ms · p95 {:.2} ms · p99 {:.2} ms · max {:.2} ms",
        milliseconds(results.commit_total),
        milliseconds(results.commit_p50),
        milliseconds(results.commit_p95),
        milliseconds(results.commit_p99),
        milliseconds(results.commit_max)
    );
    println!(
        "32 MiB asset: write {:.1} ms ({:.0} MiB/s) · read {:.1} ms ({:.0} MiB/s)",
        milliseconds(results.asset_write),
        throughput(ASSET_BYTES, results.asset_write),
        milliseconds(results.asset_read),
        throughput(ASSET_BYTES, results.asset_read)
    );
    println!(
        "small commit after 32 MiB asset: {:.2} ms",
        milliseconds(results.post_asset_commit)
    );
    println!(
        "reopen + bounded plan: {:.2} ms · integrity: {:.1} ms · file: {:.1} MiB",
        milliseconds(results.reopen_and_plan),
        milliseconds(results.integrity_check),
        mib(results.file_bytes)
    );
}

fn percentile(sorted: &[Duration], percentile: usize) -> Duration {
    let index = (sorted.len().saturating_sub(1) * percentile) / 100;
    sorted.get(index).copied().unwrap_or(Duration::ZERO)
}

fn snapshot_bytes(index: usize) -> Vec<u8> {
    let mut bytes = vec![0; SNAPSHOT_BYTES];
    bytes[..size_of::<usize>()].copy_from_slice(&index.to_le_bytes());
    bytes
}

fn deterministic_bytes(length: usize) -> Vec<u8> {
    (0..length)
        .map(|index| ((index.wrapping_mul(31).wrapping_add(index / 251)) % 256) as u8)
        .collect()
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn throughput(bytes: usize, duration: Duration) -> f64 {
    bytes as f64 / (1024.0 * 1024.0) / duration.as_secs_f64()
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}
