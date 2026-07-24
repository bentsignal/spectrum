use std::{
    ffi::CString,
    fs,
    os::{
        fd::AsRawFd as _,
        unix::{ffi::OsStrExt as _, fs::PermissionsExt as _},
    },
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use super::linux_io::{
    SlotMutationPoint, clear_slot_hardlink_race, set_slot_hardlink_child_race, update_private_slot,
};
use super::{
    FileIdentity, PUBLISH_MIRROR_FILE, PrivateDirectory, RevisionStore, SessionId,
    WRITE_BLOCK_BYTES, linux_io::ExchangeIntent, open_nofollow, prepare_reusable_slot,
    validated_identity,
};
use crate::{Actor, ActorKind, NewProject, Payload};

fn private_directory(path: &Path) -> PrivateDirectory {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    PrivateDirectory::open(path).unwrap()
}

#[test]
fn anonymous_clone_links_without_cap_dac_read_search() {
    let root = tempfile::tempdir().unwrap();
    let cache = root.path().join("cache");
    fs::create_dir(&cache).unwrap();
    let private = private_directory(&cache);
    let source_path = root.path().join("source");
    fs::write(&source_path, b"anonymous clone").unwrap();
    let source = open_nofollow(&source_path, false).unwrap();
    let candidate = private
        .clone_unnamed(&source)
        .expect("test filesystem supports anonymous reflink clones");
    use std::os::unix::fs::MetadataExt as _;
    assert_eq!(candidate.metadata().unwrap().nlink(), 0);
    private
        .link_unnamed_for_test(&candidate, "linked-candidate")
        .unwrap();
    assert_eq!(
        fs::read(cache.join("linked-candidate")).unwrap(),
        b"anonymous clone"
    );
}

#[test]
fn exchange_intent_decoder_rejects_noncanonical_and_impossible_records() {
    let intent = ExchangeIntent {
        canonical_identity: FileIdentity {
            device: 1,
            inode: 2,
        },
        candidate_identity: FileIdentity {
            device: 1,
            inode: 3,
        },
        generation: 7,
        state_id: None,
        target_generation: 8,
        target_state_id: None,
    };
    let mut noncanonical_absent_state = intent.encode();
    noncanonical_absent_state[57] = 1;
    assert!(ExchangeIntent::decode(&noncanonical_absent_state).is_err());
    let mut noncanonical_absent_target = intent.encode();
    noncanonical_absent_target[74] = 1;
    assert!(ExchangeIntent::decode(&noncanonical_absent_target).is_err());

    let mut equal_identities = intent;
    equal_identities.candidate_identity = equal_identities.canonical_identity;
    assert!(ExchangeIntent::decode(&equal_identities.encode()).is_err());

    let mut nonadvancing = intent;
    nonadvancing.target_generation = nonadvancing.generation;
    assert!(ExchangeIntent::decode(&nonadvancing.encode()).is_err());
    assert_eq!(
        ExchangeIntent::decode(&intent.encode())
            .unwrap()
            .target_generation,
        8
    );
}

#[test]
fn descriptor_bound_inspection_cannot_be_redirected_by_path_aba() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().join("canonical.prism");
    let replacement = directory.path().join("replacement.prism");
    let project = |application_id: &str| NewProject {
        application_id: application_id.into(),
        application_version: "1.0.0".into(),
        actor: Actor {
            id: format!("{application_id}:actor"),
            display_name: "ABA test".into(),
            kind: ActorKind::System,
        },
        session_id: SessionId::new(),
        root_label: Some("Created".into()),
        track_kind: "test.document".into(),
        track_label: "Document".into(),
        initial_snapshots: vec![Payload::new(
            crate::Encoding::new("test.snapshot", 1),
            b"root".to_vec(),
        )],
        assets: Vec::new(),
    };
    let (original, original_info) =
        RevisionStore::create(&canonical, project("spectrum.aba-original")).unwrap();
    drop(original);
    let (replacement_store, replacement_info) =
        RevisionStore::create(&replacement, project("spectrum.aba-replacement")).unwrap();
    drop(replacement_store);
    let held = open_nofollow(&canonical, false).unwrap();
    let held_identity = validated_identity(&held, false).unwrap();

    fs::rename(&replacement, &canonical).unwrap();
    assert_ne!(
        validated_identity(&open_nofollow(&canonical, false).unwrap(), false).unwrap(),
        held_identity
    );
    assert_eq!(
        RevisionStore::inspect_file(&held).unwrap().info.project_id,
        original_info.project_id
    );
    assert_eq!(
        RevisionStore::inspect(&canonical).unwrap().info.project_id,
        replacement_info.project_id
    );
}

#[test]
fn publication_slot_alias_child() {
    let Ok(slot) = std::env::var("SPECTRUM_SLOT_RACE_PATH") else {
        return;
    };
    let alias = PathBuf::from(std::env::var("SPECTRUM_SLOT_RACE_ALIAS").unwrap());
    let ready = PathBuf::from(std::env::var("SPECTRUM_SLOT_RACE_READY").unwrap());
    let release = PathBuf::from(std::env::var("SPECTRUM_SLOT_RACE_RELEASE").unwrap());
    let descriptor = open_nofollow(Path::new(&slot), false).unwrap();
    fs::write(&ready, b"descriptor open").unwrap();
    let started = Instant::now();
    while !release.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "parent did not release hardlink race child"
        );
        std::thread::yield_now();
    }
    let source = CString::new(format!("/proc/self/fd/{}", descriptor.as_raw_fd())).unwrap();
    let alias = CString::new(alias.as_os_str().as_bytes()).unwrap();
    let linked = unsafe {
        libc::linkat(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            alias.as_ptr(),
            libc::AT_SYMLINK_FOLLOW,
        )
    };
    assert_eq!(linked, 0, "{}", std::io::Error::last_os_error());
}

#[test]
fn external_preopened_descriptor_race_never_mutates_alias_bytes_or_mode() {
    for point in [
        SlotMutationPoint::CandidateDelta,
        SlotMutationPoint::BulkCatchUp,
        SlotMutationPoint::PermissionRepair,
    ] {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("cache");
        fs::create_dir(&cache).unwrap();
        let private = private_directory(&cache);
        let source = root.path().join("source");
        let mirror_path = cache.join(PUBLISH_MIRROR_FILE);
        let alias = root.path().join(format!("alias-{point:?}"));
        let ready = root.path().join(format!("ready-{point:?}"));
        let release = root.path().join(format!("release-{point:?}"));
        fs::write(&source, vec![0x88; WRITE_BLOCK_BYTES * 2]).unwrap();
        fs::write(&mirror_path, vec![0x33; WRITE_BLOCK_BYTES * 2]).unwrap();
        let mode = if point == SlotMutationPoint::PermissionRepair {
            0o400
        } else {
            0o600
        };
        fs::set_permissions(&mirror_path, fs::Permissions::from_mode(mode)).unwrap();
        let mirror = open_nofollow(&mirror_path, false).unwrap();
        let identity = validated_identity(&mirror, true).unwrap();
        let original_bytes = fs::read(&mirror_path).unwrap();
        let original_mode = mirror.metadata().unwrap().permissions().mode();

        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("publication_slot_alias_child")
            .arg("--nocapture")
            .env("SPECTRUM_SLOT_RACE_PATH", &mirror_path)
            .env("SPECTRUM_SLOT_RACE_ALIAS", &alias)
            .env("SPECTRUM_SLOT_RACE_READY", &ready)
            .env("SPECTRUM_SLOT_RACE_RELEASE", &release)
            .spawn()
            .unwrap();
        let started = Instant::now();
        while !ready.exists() {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "hardlink race child did not open the slot"
            );
            std::thread::yield_now();
        }
        set_slot_hardlink_child_race(point, release, alias.clone());
        let result = if point == SlotMutationPoint::PermissionRepair {
            prepare_reusable_slot(&private, PUBLISH_MIRROR_FILE, &mirror, identity).map(|()| None)
        } else {
            update_private_slot(
                &private,
                PUBLISH_MIRROR_FILE,
                &source,
                &mirror,
                identity,
                (point == SlotMutationPoint::CandidateDelta)
                    .then(|| fs::Permissions::from_mode(0o640)),
                point,
            )
        };
        clear_slot_hardlink_race();
        assert!(child.wait().unwrap().success());
        assert!(
            !matches!(result, Ok(Some(_))),
            "{point:?} mutated a slot after a raced hardlink"
        );
        assert_eq!(fs::read(&alias).unwrap(), original_bytes);
        assert_eq!(
            alias.metadata().unwrap().permissions().mode(),
            original_mode
        );
        assert_eq!(
            cache
                .join(".published-mutation-gate")
                .metadata()
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}
