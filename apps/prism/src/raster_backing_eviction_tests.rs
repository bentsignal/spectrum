use std::{
    fs::{self, File, FileTimes},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};

use crate::{DerivedBackingCache, DerivedBackingLimits, PrepareDerivedBacking};

static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(1);
const CHILD_ROOT: &str = "PRISM_EVICTION_CHILD_ROOT";
const CHILD_SOURCE: &str = "PRISM_EVICTION_CHILD_SOURCE";
const CHILD_LIMIT: &str = "PRISM_EVICTION_CHILD_LIMIT";
const CHILD_EXPECT_READY: &str = "PRISM_EVICTION_CHILD_EXPECT_READY";
const CHILD_MARKER: &str = "PRISM_EVICTION_CHILD_MARKER";

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "prism-derived-eviction-{label}-{}-{}",
            std::process::id(),
            TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        make_tree_writable(&self.0);
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn make_tree_writable(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if !metadata.file_type().is_symlink() {
        make_writable(path, &metadata);
    }
    if metadata.is_dir()
        && let Ok(entries) = fs::read_dir(path)
    {
        for entry in entries.flatten() {
            make_tree_writable(&entry.path());
        }
    }
}

#[cfg(unix)]
fn make_writable(path: &Path, metadata: &fs::Metadata) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = metadata.permissions();
    permissions.set_mode(permissions.mode() | 0o200);
    let _ = fs::set_permissions(path, permissions);
}

#[cfg(not(unix))]
fn make_writable(path: &Path, metadata: &fs::Metadata) {
    let mut permissions = metadata.permissions();
    permissions.set_readonly(false);
    let _ = fs::set_permissions(path, permissions);
}

fn test_pixels(width: u32, height: u32, seed: u8) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        Rgba([
            (x * 31 + y * 7) as u8 ^ seed,
            (x * 5 + y * 29) as u8 ^ seed.rotate_left(2),
            (x * 17 + y * 11) as u8 ^ seed.rotate_right(1),
            u8::MAX,
        ])
    })
}

fn write_source(path: &Path, seed: u8) {
    DynamicImage::ImageRgba8(test_pixels(16, 12, seed))
        .save_with_format(path, ImageFormat::WebP)
        .unwrap();
}

fn version_root(root: &Path) -> PathBuf {
    root.join("v2")
}

fn entry_path(root: &Path, key: &str) -> PathBuf {
    version_root(root).join(key)
}

fn complete_entry_logical_bytes(path: &Path) -> u64 {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum()
}

fn set_access_mtime(path: &Path, modified: SystemTime) {
    File::options()
        .read(true)
        .write(true)
        .open(path.join(".access"))
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
}

fn prepare_ready(cache: &DerivedBackingCache, source: &Path) -> crate::DerivedRasterBacking {
    match cache.prepare(source).unwrap() {
        PrepareDerivedBacking::Ready { backing, .. } => backing,
        PrepareDerivedBacking::InProgress(_) => panic!("test cache unexpectedly busy"),
    }
}

#[test]
fn schema_v2_uses_a_versioned_complete_entry_layout() {
    let directory = TestDirectory::new("schema-v2");
    let source = directory.path().join("source.webp");
    write_source(&source, 7);
    let cache_root = directory.path().join("cache");
    let cache = DerivedBackingCache::new(&cache_root, DerivedBackingLimits::default());
    let identity = cache.identify(&source).unwrap();
    drop(prepare_ready(&cache, &source));

    let entry = entry_path(&cache_root, identity.key());
    assert!(entry.is_dir());
    assert!(!cache_root.join(identity.key()).exists());
    let mut names = fs::read_dir(&entry)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        [
            ".access",
            ".lease",
            "manifest.json",
            "pixels.rgba8",
            "ready"
        ]
        .map(str::to_owned)
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(entry.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["schema_version"].as_u64(), Some(2));
}

#[test]
fn deterministic_lru_uses_access_mtime_then_key() {
    let directory = TestDirectory::new("deterministic-lru");
    let cache_root = directory.path().join("cache");
    let sources = (0..4)
        .map(|index| {
            let path = directory.path().join(format!("source-{index}.webp"));
            write_source(&path, (20 + index) as u8);
            path
        })
        .collect::<Vec<_>>();
    let default_cache = DerivedBackingCache::new(&cache_root, DerivedBackingLimits::default());
    let identities = sources
        .iter()
        .map(|source| default_cache.identify(source).unwrap())
        .collect::<Vec<_>>();
    drop(prepare_ready(&default_cache, &sources[0]));
    drop(prepare_ready(&default_cache, &sources[1]));
    let first_entry = entry_path(&cache_root, identities[0].key());
    let second_entry = entry_path(&cache_root, identities[1].key());
    let first_bytes = complete_entry_logical_bytes(&first_entry);
    let second_bytes = complete_entry_logical_bytes(&second_entry);
    assert_eq!(first_bytes, second_bytes);
    let clock = SystemTime::now();
    set_access_mtime(&first_entry, clock - Duration::from_secs(20));
    set_access_mtime(&second_entry, clock - Duration::from_secs(10));

    let bounded = DerivedBackingCache::new(
        &cache_root,
        DerivedBackingLimits {
            max_cache_bytes: first_bytes + second_bytes,
            ..DerivedBackingLimits::default()
        },
    );
    drop(prepare_ready(&bounded, &sources[2]));
    assert!(!first_entry.exists());
    assert!(second_entry.exists());
    let third_entry = entry_path(&cache_root, identities[2].key());
    assert!(third_entry.exists());

    let tied = SystemTime::now() - Duration::from_secs(5);
    set_access_mtime(&second_entry, tied);
    set_access_mtime(&third_entry, tied);
    assert_eq!(
        fs::metadata(second_entry.join(".access"))
            .unwrap()
            .modified()
            .unwrap(),
        fs::metadata(third_entry.join(".access"))
            .unwrap()
            .modified()
            .unwrap()
    );
    let expected_evicted = [identities[1].key(), identities[2].key()]
        .into_iter()
        .min()
        .unwrap();
    drop(prepare_ready(&bounded, &sources[3]));
    assert!(!entry_path(&cache_root, expected_evicted).exists());
    assert!(entry_path(&cache_root, identities[3].key()).exists());
}

#[test]
fn retained_reader_prevents_eviction_until_drop() {
    let directory = TestDirectory::new("retained-reader");
    let cache_root = directory.path().join("cache");
    let first_source = directory.path().join("first.webp");
    let second_source = directory.path().join("second.webp");
    write_source(&first_source, 41);
    write_source(&second_source, 99);
    let default_cache = DerivedBackingCache::new(&cache_root, DerivedBackingLimits::default());
    let first_identity = default_cache.identify(&first_source).unwrap();
    let second_identity = default_cache.identify(&second_source).unwrap();
    let reader = prepare_ready(&default_cache, &first_source);
    let first_entry = entry_path(&cache_root, first_identity.key());
    let entry_bytes = complete_entry_logical_bytes(&first_entry);
    let bounded = DerivedBackingCache::new(
        &cache_root,
        DerivedBackingLimits {
            max_cache_bytes: entry_bytes,
            ..DerivedBackingLimits::default()
        },
    );
    let error = match bounded.prepare(&second_source) {
        Ok(_) => panic!("active reader unexpectedly allowed eviction"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("cannot evict active entries"));
    assert!(first_entry.exists());
    assert!(!entry_path(&cache_root, second_identity.key()).exists());

    drop(reader);
    drop(prepare_ready(&bounded, &second_source));
    assert!(!first_entry.exists());
    assert!(entry_path(&cache_root, second_identity.key()).exists());
}

#[test]
fn incomplete_entry_and_crash_tombstones_are_handled_fail_closed() {
    let directory = TestDirectory::new("incomplete-and-tombstone");
    let cache_root = directory.path().join("cache");
    fs::create_dir(&cache_root).unwrap();
    fs::create_dir(version_root(&cache_root)).unwrap();
    let incomplete = version_root(&cache_root).join("a".repeat(64));
    fs::create_dir(&incomplete).unwrap();
    fs::write(incomplete.join("pixels.rgba8"), b"partial").unwrap();
    let source = directory.path().join("source.webp");
    write_source(&source, 51);
    let cache = DerivedBackingCache::new(&cache_root, DerivedBackingLimits::default());
    assert!(cache.prepare(&source).is_err());

    fs::remove_dir_all(&incomplete).unwrap();
    let identity = cache.identify(&source).unwrap();
    let tombstone = version_root(&cache_root).join(format!(".evict-{}-999-1", identity.key()));
    fs::create_dir(&tombstone).unwrap();
    fs::write(tombstone.join("orphan"), b"crash").unwrap();
    drop(prepare_ready(&cache, &source));
    assert!(!tombstone.exists());
}

#[test]
fn cross_process_reader_lease_blocks_eviction() {
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let source = PathBuf::from(std::env::var_os(CHILD_SOURCE).unwrap());
        let limit = std::env::var(CHILD_LIMIT).unwrap().parse().unwrap();
        let expect_ready = std::env::var(CHILD_EXPECT_READY).unwrap() == "1";
        let cache = DerivedBackingCache::new(
            PathBuf::from(root),
            DerivedBackingLimits {
                max_cache_bytes: limit,
                ..DerivedBackingLimits::default()
            },
        );
        match cache.prepare(&source) {
            Ok(PrepareDerivedBacking::Ready { .. }) if expect_ready => {}
            Err(error)
                if !expect_ready && error.to_string().contains("cannot evict active entries") => {}
            Ok(PrepareDerivedBacking::InProgress(_)) => {
                panic!("child unexpectedly observed a busy maintenance lock")
            }
            Ok(PrepareDerivedBacking::Ready { .. }) => {
                panic!("child unexpectedly evicted an active reader")
            }
            Err(error) => panic!("child preparation failed unexpectedly: {error:#}"),
        }
        fs::write(
            PathBuf::from(std::env::var_os(CHILD_MARKER).unwrap()),
            b"ran",
        )
        .unwrap();
        return;
    }

    let directory = TestDirectory::new("cross-process-reader");
    let cache_root = directory.path().join("cache");
    let first_source = directory.path().join("first.webp");
    let second_source = directory.path().join("second.webp");
    write_source(&first_source, 61);
    write_source(&second_source, 62);
    let cache = DerivedBackingCache::new(&cache_root, DerivedBackingLimits::default());
    let first_identity = cache.identify(&first_source).unwrap();
    let second_identity = cache.identify(&second_source).unwrap();
    let reader = prepare_ready(&cache, &first_source);
    let entry_bytes = complete_entry_logical_bytes(&entry_path(&cache_root, first_identity.key()));
    let marker = directory.path().join("child-ran");

    let run_child = |expect_ready: bool| {
        let _ = fs::remove_file(&marker);
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "raster_backing_eviction_tests::cross_process_reader_lease_blocks_eviction",
                "--nocapture",
            ])
            .env(CHILD_ROOT, &cache_root)
            .env(CHILD_SOURCE, &second_source)
            .env(CHILD_LIMIT, entry_bytes.to_string())
            .env(CHILD_EXPECT_READY, if expect_ready { "1" } else { "0" })
            .env(CHILD_MARKER, &marker)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(fs::read(&marker).unwrap(), b"ran");
    };
    run_child(false);
    assert!(entry_path(&cache_root, first_identity.key()).exists());
    drop(reader);
    run_child(true);
    assert!(!entry_path(&cache_root, first_identity.key()).exists());
    assert!(entry_path(&cache_root, second_identity.key()).exists());
}
