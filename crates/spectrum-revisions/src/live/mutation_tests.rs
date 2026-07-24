use std::{fs, os::unix::fs::PermissionsExt as _};

use super::linux_io::{
    SlotMutationPoint, clear_slot_hardlink_race, set_slot_hardlink_race, update_private_slot,
};
use super::*;

fn private_directory(path: &Path) -> PrivateDirectory {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    PrivateDirectory::open(path).unwrap()
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
            )
            .is_err()
        );
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
