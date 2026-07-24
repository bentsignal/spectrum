use std::{
    fs,
    os::fd::AsRawFd as _,
    os::unix::fs::{FileExt as _, PermissionsExt as _},
    process::Command,
    time::{Duration, Instant},
};

use super::linux_io::{
    SlotMutationPoint, clear_slot_hardlink_race, set_slot_hardlink_race, update_private_slot,
};
use super::*;

fn private_directory(path: &Path) -> PrivateDirectory {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    PrivateDirectory::open(path).unwrap()
}

fn wait_for_path(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "timed out waiting for {}", path.display());
}

#[test]
fn lock_waiter_child() {
    let Ok(cache) = std::env::var("SPECTRUM_LOCK_WAITER_CACHE") else {
        return;
    };
    let started = std::env::var("SPECTRUM_LOCK_WAITER_STARTED").unwrap();
    let acquired = std::env::var("SPECTRUM_LOCK_WAITER_ACQUIRED").unwrap();
    fs::write(started, b"started").unwrap();
    let _lock = lock_private(&Path::new(&cache).join(LOCK_FILE)).unwrap();
    fs::write(acquired, b"acquired").unwrap();
}

#[test]
fn lock_waiter_does_not_recover_a_slot_sealed_by_the_lock_holder() {
    let root = tempfile::tempdir().unwrap();
    let cache = root.path().join("cache");
    fs::create_dir(&cache).unwrap();
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).unwrap();
    let held_lock = lock_private(&cache.join(LOCK_FILE)).unwrap();
    let private = PrivateDirectory::open(&cache).unwrap();
    let mirror_path = cache.join(PUBLISH_MIRROR_FILE);
    fs::write(&mirror_path, b"sealed candidate bytes").unwrap();
    fs::set_permissions(&mirror_path, fs::Permissions::from_mode(0o600)).unwrap();
    let mirror = open_nofollow(&mirror_path, true).unwrap();
    let identity = validated_identity(&mirror, true).unwrap();
    let guard = private
        .begin_mutation(
            PUBLISH_MIRROR_FILE,
            &mirror,
            identity,
            SlotMutationPoint::CandidateDelta,
        )
        .unwrap();
    let gate = cache.join(".published-mutation-gate");
    let original_mode = mirror.metadata().unwrap().permissions().mode();
    let mut original_bytes = vec![0; mirror.metadata().unwrap().len() as usize];
    mirror.read_exact_at(&mut original_bytes, 0).unwrap();
    let started = root.path().join("waiter-started");
    let acquired = root.path().join("waiter-acquired");

    let mut waiter = Command::new(std::env::current_exe().unwrap())
        .arg("lock_waiter_child")
        .arg("--nocapture")
        .env("SPECTRUM_LOCK_WAITER_CACHE", &cache)
        .env("SPECTRUM_LOCK_WAITER_STARTED", &started)
        .env("SPECTRUM_LOCK_WAITER_ACQUIRED", &acquired)
        .spawn()
        .unwrap();
    wait_for_path(&started);
    std::thread::sleep(Duration::from_millis(150));

    assert!(
        !acquired.exists(),
        "waiter acquired the held publication lock"
    );
    assert!(
        !mirror_path.exists(),
        "waiter restored the sealed slot early"
    );
    assert_eq!(gate.metadata().unwrap().permissions().mode() & 0o777, 0);
    assert_eq!(
        mirror.metadata().unwrap().permissions().mode(),
        original_mode
    );
    let mut observed_bytes = vec![0; original_bytes.len()];
    mirror.read_exact_at(&mut observed_bytes, 0).unwrap();
    assert_eq!(observed_bytes, original_bytes);

    drop(guard);
    private
        .validate(PUBLISH_MIRROR_FILE, identity, true)
        .unwrap();
    drop(held_lock);
    assert!(waiter.wait().unwrap().success());
    assert!(acquired.exists());
    assert_eq!(fs::read(&mirror_path).unwrap(), original_bytes);
}

#[test]
fn lock_hardlink_race_never_repairs_permissions_through_an_alias() {
    let root = tempfile::tempdir().unwrap();
    let cache = root.path().join("cache");
    fs::create_dir(&cache).unwrap();
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).unwrap();
    let lock_path = cache.join(LOCK_FILE);
    let alias = root.path().join("publish-lock-alias");
    fs::write(&lock_path, b"existing lock").unwrap();
    fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o640)).unwrap();
    let original_mode = lock_path.metadata().unwrap().permissions().mode();

    PRIVATE_LOCK_HARDLINK_ALIAS.with(|hook| hook.replace(Some(alias.clone())));
    assert!(lock_private(&lock_path).is_err());
    PRIVATE_LOCK_HARDLINK_ALIAS.with(|hook| hook.replace(None));

    assert_eq!(fs::read(&alias).unwrap(), b"existing lock");
    assert_eq!(
        alias.metadata().unwrap().permissions().mode(),
        original_mode
    );
}

#[test]
fn slot_races_after_validation_fail_before_candidate_or_bulk_mutation() {
    for point in [
        SlotMutationPoint::CandidateDelta,
        SlotMutationPoint::BulkCatchUp,
    ] {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("cache");
        fs::create_dir(&cache).unwrap();
        let private = private_directory(&cache);
        let source = root.path().join("source");
        let mirror_path = cache.join(PUBLISH_MIRROR_FILE);
        let alias = root.path().join(format!("alias-{point:?}"));
        fs::write(&source, vec![0x88; WRITE_BLOCK_BYTES * 2]).unwrap();
        fs::write(&mirror_path, vec![0x33; WRITE_BLOCK_BYTES * 2]).unwrap();
        fs::set_permissions(&mirror_path, fs::Permissions::from_mode(0o600)).unwrap();
        let mirror = open_nofollow(&mirror_path, true).unwrap();
        let identity = validated_identity(&mirror, true).unwrap();
        let original_bytes = fs::read(&mirror_path).unwrap();
        let original_mode = mirror.metadata().unwrap().permissions().mode();

        set_slot_hardlink_race(point, alias.clone());
        let permissions =
            (point == SlotMutationPoint::CandidateDelta).then(|| fs::Permissions::from_mode(0o640));
        assert!(
            update_private_slot(
                &private,
                PUBLISH_MIRROR_FILE,
                &source,
                &mirror,
                identity,
                permissions,
                point,
                None,
            )
            .is_err()
        );
        clear_slot_hardlink_race();

        assert_eq!(fs::read(&alias).unwrap(), original_bytes);
        assert_eq!(
            alias.metadata().unwrap().permissions().mode(),
            original_mode
        );
        let alias_descriptor = open_nofollow(&alias, false).unwrap();
        let exchange_xattr = c"user.spectrum.exchange-intent-v2";
        assert_eq!(
            unsafe {
                libc::fgetxattr(
                    alias_descriptor.as_raw_fd(),
                    exchange_xattr.as_ptr(),
                    std::ptr::null_mut(),
                    0,
                )
            },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ENODATA)
        );
        assert_eq!(
            cache.metadata().unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}

#[test]
fn slot_race_after_validation_fails_before_permission_repair() {
    let root = tempfile::tempdir().unwrap();
    let cache = root.path().join("cache");
    fs::create_dir(&cache).unwrap();
    let private = private_directory(&cache);
    let mirror_path = cache.join(PUBLISH_MIRROR_FILE);
    let alias = root.path().join("permission-alias");
    fs::write(&mirror_path, b"immutable old slot").unwrap();
    fs::set_permissions(&mirror_path, fs::Permissions::from_mode(0o400)).unwrap();
    let mirror = open_nofollow(&mirror_path, false).unwrap();
    let identity = validated_identity(&mirror, true).unwrap();
    let original_bytes = fs::read(&mirror_path).unwrap();
    let original_mode = mirror.metadata().unwrap().permissions().mode();

    set_slot_hardlink_race(SlotMutationPoint::PermissionRepair, alias.clone());
    assert!(prepare_reusable_slot(&private, PUBLISH_MIRROR_FILE, &mirror, identity).is_err());
    clear_slot_hardlink_race();

    assert_eq!(fs::read(&alias).unwrap(), original_bytes);
    assert_eq!(
        alias.metadata().unwrap().permissions().mode(),
        original_mode
    );
    assert_eq!(
        cache.metadata().unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn reopening_restores_a_slot_stranded_inside_the_sealed_mutation_gate() {
    let root = tempfile::tempdir().unwrap();
    let cache = root.path().join("cache");
    fs::create_dir(&cache).unwrap();
    let private = private_directory(&cache);
    let mirror_path = cache.join(PUBLISH_MIRROR_FILE);
    fs::write(&mirror_path, b"stranded slot").unwrap();
    let mirror = open_nofollow(&mirror_path, true).unwrap();
    let identity = validated_identity(&mirror, true).unwrap();

    let guard = private
        .begin_mutation(
            PUBLISH_MIRROR_FILE,
            &mirror,
            identity,
            SlotMutationPoint::CandidateDelta,
        )
        .unwrap();
    std::mem::forget(guard);
    assert!(!mirror_path.exists());
    let gate = cache.join(".published-mutation-gate");
    assert_eq!(gate.metadata().unwrap().permissions().mode() & 0o777, 0);
    fs::write(cache.join("root-stays-reachable"), b"reachable").unwrap();
    drop(private);

    let recovered = PrivateDirectory::open(&cache).unwrap();
    assert_eq!(gate.metadata().unwrap().permissions().mode() & 0o777, 0o700);
    recovered
        .validate(PUBLISH_MIRROR_FILE, identity, true)
        .unwrap();
    assert_eq!(fs::read(&mirror_path).unwrap(), b"stranded slot");
}

#[test]
fn opening_private_directory_prepares_the_mutation_gate_before_hot_writes() {
    let root = tempfile::tempdir().unwrap();
    let cache = root.path().join("cache");
    fs::create_dir(&cache).unwrap();
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).unwrap();
    let gate = cache.join(".published-mutation-gate");
    assert!(!gate.exists());

    drop(PrivateDirectory::open(&cache).unwrap());

    assert!(gate.metadata().unwrap().file_type().is_dir());
    assert_eq!(gate.metadata().unwrap().permissions().mode() & 0o777, 0o700);
}
