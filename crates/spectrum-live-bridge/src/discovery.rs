use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use spectrum_revisions::ProjectId;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    BindingId, BridgeError, BridgeResult, Capability, CapabilityId, DISCOVERY_EXPIRY,
    DISCOVERY_FAMILY, DISCOVERY_REFRESH, InstanceId, LocalStream, PROTOCOL_VERSION,
};

mod scan;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum EndpointAddress {
    Unix { path: PathBuf },
    WindowsPipe { name: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryRecord {
    pub family: String,
    pub protocol_min: u16,
    pub protocol_max: u16,
    pub application: String,
    pub project_id: ProjectId,
    pub canonical_project_path: PathBuf,
    pub instance_id: InstanceId,
    pub binding_id: BindingId,
    pub binding_epoch: u64,
    pub endpoint: EndpointAddress,
    pub capability_id: CapabilityId,
    pub capability_path: PathBuf,
    pub process_id: u32,
    #[serde(default)]
    pub command_versions: BTreeMap<String, (u32, u32)>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub oldest_event_seq: u64,
    pub newest_event_seq: u64,
    pub created_unix_millis: u64,
    pub refreshed_unix_millis: u64,
    pub expires_unix_millis: u64,
}

impl DiscoveryRecord {
    pub fn validate(&self) -> BridgeResult<()> {
        if self.family != DISCOVERY_FAMILY {
            return Err(BridgeError::Protocol(
                "unknown discovery record family".into(),
            ));
        }
        if self.protocol_min == 0
            || self.protocol_min > PROTOCOL_VERSION
            || self.protocol_max < PROTOCOL_VERSION
            || self.protocol_min > self.protocol_max
        {
            return Err(BridgeError::Protocol(
                "discovery protocol range is incompatible".into(),
            ));
        }
        if self.application.is_empty() || self.application.len() > crate::MAX_STRING_BYTES {
            return Err(BridgeError::Protocol(
                "invalid discovery application identity".into(),
            ));
        }
        Ok(())
    }

    pub fn is_expired(&self, now_unix_millis: u64) -> bool {
        now_unix_millis >= self.expires_unix_millis
    }
}

#[derive(Clone, Debug)]
pub struct DiscoveryDirectory {
    root: PathBuf,
}

impl DiscoveryDirectory {
    pub fn open(root: impl Into<PathBuf>) -> BridgeResult<Self> {
        let root = root.into();
        secure_directory(&root)?;
        let directory = Self { root };
        directory.remove_stale(|endpoint| LocalStream::connect(endpoint).is_err())?;
        Ok(directory)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Return the authenticated local endpoint reserved for a binding.
    ///
    /// Applications must use this instead of reproducing platform-specific
    /// endpoint naming. [`DiscoveryDirectory::publish`] verifies the exact
    /// same mapping before making a binding discoverable.
    pub fn endpoint_for(&self, binding_id: BindingId) -> EndpointAddress {
        self.expected_endpoint(binding_id)
    }

    pub fn publish(
        &self,
        mut record: DiscoveryRecord,
        capability: &Capability,
    ) -> BridgeResult<PublishedBinding> {
        if record.capability_id != capability.id() {
            return Err(BridgeError::Authentication(
                "record and capability identities differ".into(),
            ));
        }
        record.validate()?;
        let capability_path = self.root.join(format!("{}.capability", record.binding_id));
        let record_path = self.root.join(format!("{}.json", record.binding_id));
        if record.capability_path != capability_path {
            return Err(BridgeError::Protocol(
                "capability path is outside the binding directory".into(),
            ));
        }
        if record.endpoint != self.expected_endpoint(record.binding_id) {
            return Err(BridgeError::Protocol(
                "endpoint target does not match binding identity".into(),
            ));
        }
        let secret = capability.copy_secret();
        atomic_publish(&capability_path, &secret)?;
        record.refresh()?;
        let bytes = serde_json::to_vec(&record)?;
        if let Err(error) = atomic_publish(&record_path, &bytes) {
            let _ = fs::remove_file(&capability_path);
            return Err(error);
        }
        Ok(PublishedBinding {
            record,
            record_path,
            capability_path,
            remove_on_drop: true,
        })
    }

    /// Publish a binding with a freshly generated capability.
    ///
    /// Lifecycle owners should use this for launch, replacement, move, close /
    /// reopen, and any binding epoch change so capability rotation is not
    /// optional at the call site.
    pub fn publish_rotated(
        &self,
        mut record: DiscoveryRecord,
    ) -> BridgeResult<(PublishedBinding, Capability)> {
        let capability = Capability::generate()?;
        record.capability_id = capability.id();
        record.capability_path = self.root.join(format!("{}.capability", record.binding_id));
        let published = self.publish(record, &capability)?;
        Ok((published, capability))
    }

    pub fn records(&self) -> BridgeResult<Vec<DiscoveryRecord>> {
        let mut records = self
            .validated_entries()?
            .into_iter()
            .map(|entry| entry.record)
            .collect::<Vec<_>>();
        records.sort_by_key(|record| (record.application.clone(), record.binding_id.to_string()));
        Ok(records)
    }

    pub fn remove_stale<F>(&self, mut endpoint_failed: F) -> BridgeResult<usize>
    where
        F: FnMut(&EndpointAddress) -> bool,
    {
        self.remove_stale_at(unix_millis()?, &mut endpoint_failed)
    }

    fn remove_stale_at<F>(&self, now: u64, endpoint_failed: &mut F) -> BridgeResult<usize>
    where
        F: FnMut(&EndpointAddress) -> bool,
    {
        let mut removed = 0;
        for entry in self.validated_entries()? {
            if entry.record.is_expired(now)
                && endpoint_failed(&entry.record.endpoint)
                && secure_owned_file(&entry.record_path).is_ok()
                && secure_owned_file(&entry.capability_path).is_ok()
            {
                remove_owned_endpoint(&entry.record.endpoint)?;
                fs::remove_file(entry.record_path)?;
                fs::remove_file(entry.capability_path)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub fn load_capability(&self, record: &DiscoveryRecord) -> BridgeResult<Capability> {
        let expected = self.root.join(format!("{}.capability", record.binding_id));
        if record.capability_path != expected {
            return Err(BridgeError::Authentication(
                "capability path does not match binding".into(),
            ));
        }
        let secret = secure_read(&expected, 32)?;
        let bytes = Zeroizing::new(
            <[u8; 32]>::try_from(secret.as_slice())
                .map_err(|_| BridgeError::Authentication("capability has wrong length".into()))?,
        );
        Ok(Capability::from_zeroizing(record.capability_id, bytes))
    }
}

pub struct PublishedBinding {
    record: DiscoveryRecord,
    record_path: PathBuf,
    capability_path: PathBuf,
    remove_on_drop: bool,
}

impl PublishedBinding {
    pub fn record(&self) -> &DiscoveryRecord {
        &self.record
    }

    pub fn refresh(&mut self, oldest_event_seq: u64, newest_event_seq: u64) -> BridgeResult<()> {
        self.record.oldest_event_seq = oldest_event_seq;
        self.record.newest_event_seq = newest_event_seq;
        self.record.refresh()?;
        atomic_publish(&self.record_path, &serde_json::to_vec(&self.record)?)
    }

    pub fn preserve_files(mut self) {
        self.remove_on_drop = false;
    }
}

impl Drop for PublishedBinding {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = fs::remove_file(&self.record_path);
            let _ = fs::remove_file(&self.capability_path);
        }
    }
}

pub struct DiscoveryLease {
    published: Arc<Mutex<PublishedBinding>>,
    stop: Arc<(Mutex<bool>, Condvar)>,
    worker: Option<thread::JoinHandle<()>>,
    refresh_error: Arc<Mutex<Option<String>>>,
}

impl DiscoveryLease {
    pub fn new(published: PublishedBinding) -> BridgeResult<Self> {
        Self::with_refresh_interval(published, DISCOVERY_REFRESH)
    }

    fn with_refresh_interval(
        published: PublishedBinding,
        refresh_interval: std::time::Duration,
    ) -> BridgeResult<Self> {
        let published = Arc::new(Mutex::new(published));
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let refresh_error = Arc::new(Mutex::new(None));
        let worker = {
            let published = Arc::clone(&published);
            let stop = Arc::clone(&stop);
            let refresh_error = Arc::clone(&refresh_error);
            thread::Builder::new()
                .name("spectrum-discovery-lease".into())
                .spawn(move || {
                    let (stopped, wake) = &*stop;
                    loop {
                        let guard = match stopped.lock() {
                            Ok(guard) => guard,
                            Err(_) => return,
                        };
                        let Ok((guard, _)) = wake.wait_timeout(guard, refresh_interval) else {
                            return;
                        };
                        if *guard {
                            return;
                        }
                        drop(guard);
                        let result = (|| -> BridgeResult<()> {
                            let mut binding = published.lock().map_err(|_| BridgeError::Closed)?;
                            let oldest = binding.record.oldest_event_seq;
                            let newest = binding.record.newest_event_seq;
                            binding.refresh(oldest, newest)
                        })();
                        if let Err(error) = result {
                            if let Ok(mut slot) = refresh_error.lock() {
                                *slot = Some(error.to_string());
                            }
                            return;
                        }
                    }
                })
                .map_err(BridgeError::Io)?
        };
        Ok(Self {
            published,
            stop,
            worker: Some(worker),
            refresh_error,
        })
    }

    pub fn refresh(&self, oldest_event_seq: u64, newest_event_seq: u64) -> BridgeResult<()> {
        if let Some(error) = self
            .refresh_error
            .lock()
            .map_err(|_| BridgeError::Closed)?
            .as_ref()
        {
            return Err(BridgeError::Protocol(format!(
                "automatic discovery refresh failed: {error}"
            )));
        }
        self.published
            .lock()
            .map_err(|_| BridgeError::Closed)?
            .refresh(oldest_event_seq, newest_event_seq)
    }

    pub fn record(&self) -> BridgeResult<DiscoveryRecord> {
        Ok(self
            .published
            .lock()
            .map_err(|_| BridgeError::Closed)?
            .record()
            .clone())
    }
}

impl Drop for DiscoveryLease {
    fn drop(&mut self) {
        let (stopped, wake) = &*self.stop;
        if let Ok(mut stopped) = stopped.lock() {
            *stopped = true;
            wake.notify_all();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl DiscoveryRecord {
    fn refresh(&mut self) -> BridgeResult<()> {
        self.refreshed_unix_millis = unix_millis()?;
        if self.created_unix_millis == 0 {
            self.created_unix_millis = self.refreshed_unix_millis;
        }
        self.expires_unix_millis = self.refreshed_unix_millis
            + u64::try_from(DISCOVERY_EXPIRY.as_millis())
                .map_err(|_| BridgeError::Protocol("lease duration overflow".into()))?;
        Ok(())
    }
}

fn atomic_publish(path: &Path, bytes: &[u8]) -> BridgeResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| BridgeError::Protocol("publication path has no parent".into()))?;
    secure_directory(parent)?;
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random)
        .map_err(|error| BridgeError::Io(std::io::Error::other(error.to_string())))?;
    let suffix = u64::from_ne_bytes(random);
    random.zeroize();
    let temporary = parent.join(format!(".bridge-{suffix:016x}.tmp"));
    let result = (|| -> BridgeResult<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        #[cfg(windows)]
        apply_private_acl(&temporary)?;
        durable_replace(&temporary, path)?;
        secure_owned_file(path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn durable_replace(source: &Path, destination: &Path) -> BridgeResult<()> {
    fs::rename(source, destination)?;
    let parent = destination
        .parent()
        .ok_or_else(|| BridgeError::Protocol("publication path has no parent".into()))?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(windows)]
fn durable_replace(source: &Path, destination: &Path) -> BridgeResult<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn secure_read(path: &Path, maximum: usize) -> BridgeResult<Zeroizing<Vec<u8>>> {
    secure_owned_file(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    verify_open_file(&file)?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(
        u64::try_from(maximum + 1).map_err(|_| BridgeError::Limit("read limit overflow".into()))?,
    )
    .read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(BridgeError::Limit("secure file is oversized".into()));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn remove_owned_endpoint(endpoint: &EndpointAddress) -> BridgeResult<()> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    let EndpointAddress::Unix { path } = endpoint else {
        return Ok(());
    };
    let Some(parent) = path.parent() else {
        return Err(BridgeError::Authentication(
            "stale endpoint has no private parent".into(),
        ));
    };
    secure_directory(parent)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_socket()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(BridgeError::Authentication(
            "stale endpoint failed ownership checks".into(),
        ));
    }
    fs::remove_file(path)?;
    Ok(())
}

#[cfg(windows)]
fn remove_owned_endpoint(_endpoint: &EndpointAddress) -> BridgeResult<()> {
    // Named pipes disappear with their final kernel handle; stale cleanup only
    // removes the authenticated discovery/capability records.
    Ok(())
}

#[cfg(unix)]
fn secure_directory(path: &Path) -> BridgeResult<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    if !path.exists() {
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700).recursive(true).create(path)?;
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(BridgeError::Authentication(
            "discovery directory must be owned by the user with mode 0700".into(),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn secure_directory(path: &Path) -> BridgeResult<()> {
    if path.exists() {
        verify_private_acl(path)?;
    } else {
        fs::create_dir_all(path)?;
        apply_private_acl(path)?;
    }
    Ok(())
}

#[cfg(unix)]
fn secure_owned_file(path: &Path) -> BridgeResult<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(BridgeError::Authentication(
            "binding file must be a user-owned, single-link regular file with mode 0600".into(),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn secure_owned_file(path: &Path) -> BridgeResult<()> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(BridgeError::Authentication(
            "binding file is not regular".into(),
        ));
    }
    verify_private_acl(path)?;
    Ok(())
}

#[cfg(unix)]
fn verify_open_file(file: &File) -> BridgeResult<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(BridgeError::Authentication(
            "opened binding file failed ownership checks".into(),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn apply_private_acl(path: &Path) -> BridgeResult<()> {
    crate::windows_security::apply_private_acl(path)
}

#[cfg(windows)]
fn verify_private_acl(path: &Path) -> BridgeResult<()> {
    crate::windows_security::verify_private_acl(path)
}

#[cfg(windows)]
fn verify_open_file(file: &File) -> BridgeResult<()> {
    if !file.metadata()?.is_file() {
        return Err(BridgeError::Authentication(
            "opened binding file is not regular".into(),
        ));
    }
    crate::windows_security::verify_private_handle(file)
}

fn unix_millis() -> BridgeResult<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| BridgeError::Protocol("system clock precedes epoch".into()))?
        .as_millis();
    u64::try_from(millis).map_err(|_| BridgeError::Protocol("system clock overflow".into()))
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use super::*;

    fn record(root: &Path, capability: &Capability) -> DiscoveryRecord {
        let binding_id = BindingId::new();
        DiscoveryRecord {
            family: DISCOVERY_FAMILY.into(),
            protocol_min: PROTOCOL_VERSION,
            protocol_max: PROTOCOL_VERSION,
            application: "test-host".into(),
            project_id: ProjectId::new(),
            canonical_project_path: root.join("project.test"),
            instance_id: InstanceId::new(),
            binding_id,
            binding_epoch: 1,
            endpoint: test_endpoint(root, binding_id),
            capability_id: capability.id(),
            capability_path: root.join(format!("{binding_id}.capability")),
            process_id: std::process::id(),
            command_versions: BTreeMap::new(),
            capabilities: Vec::new(),
            oldest_event_seq: 0,
            newest_event_seq: 0,
            created_unix_millis: 0,
            refreshed_unix_millis: 0,
            expires_unix_millis: 0,
        }
    }

    #[cfg(unix)]
    fn test_endpoint(root: &Path, binding_id: BindingId) -> EndpointAddress {
        let binding = binding_id.to_string();
        EndpointAddress::Unix {
            path: root.join(format!("{}.sock", &binding[..8])),
        }
    }

    #[cfg(windows)]
    fn test_endpoint(_root: &Path, binding_id: BindingId) -> EndpointAddress {
        EndpointAddress::WindowsPipe {
            name: format!(r"\\.\pipe\spectrum-live-{binding_id}"),
        }
    }

    #[test]
    fn publication_round_trips_and_removes_secrets() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("records");
        let directory = DiscoveryDirectory::open(&root).unwrap();
        let capability = Capability::generate().unwrap();
        let lease = directory
            .publish(record(&root, &capability), &capability)
            .unwrap();
        let records = directory.records().unwrap();
        assert_eq!(records.len(), 1);
        let loaded = directory.load_capability(&records[0]).unwrap();
        let challenge = crate::AuthChallenge::new(
            records[0].binding_id,
            1,
            records[0].instance_id,
            records[0].project_id,
            records[0].capability_id,
        )
        .unwrap();
        let proof = loaded.prove(&challenge).unwrap();
        crate::verify_proof(&capability, &challenge, &proof).unwrap();
        drop(lease);
        assert!(directory.records().unwrap().is_empty());
    }

    #[test]
    fn lease_refreshes_automatically_and_rotated_publication_changes_capability() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("records");
        let directory = DiscoveryDirectory::open(&root).unwrap();
        let seed = Capability::generate().unwrap();
        let published = directory.publish(record(&root, &seed), &seed).unwrap();
        let initial = published.record().clone();
        let lease =
            DiscoveryLease::with_refresh_interval(published, Duration::from_millis(10)).unwrap();
        thread::sleep(Duration::from_millis(40));
        let refreshed = lease.record().unwrap();
        assert!(refreshed.refreshed_unix_millis > initial.refreshed_unix_millis);
        assert!(refreshed.expires_unix_millis > initial.expires_unix_millis);
        drop(lease);

        let mut next = initial;
        next.binding_epoch += 1;
        next.created_unix_millis = 0;
        next.refreshed_unix_millis = 0;
        next.expires_unix_millis = 0;
        let (published, rotated) = directory.publish_rotated(next).unwrap();
        assert_ne!(rotated.id(), seed.id());
        assert_ne!(rotated.secret(), seed.secret());
        drop(published);
    }

    #[cfg(unix)]
    #[test]
    fn hardlinks_and_symlinks_are_rejected() {
        use std::os::unix::fs::symlink;
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("records");
        let directory = DiscoveryDirectory::open(&root).unwrap();
        let capability = Capability::generate().unwrap();
        let lease = directory
            .publish(record(&root, &capability), &capability)
            .unwrap();
        let original = lease.record().capability_path.clone();
        let hardlink = root.join("hardlink");
        fs::hard_link(&original, &hardlink).unwrap();
        assert!(directory.load_capability(lease.record()).is_err());
        fs::remove_file(hardlink).unwrap();
        fs::remove_file(&original).unwrap();
        symlink("/dev/null", &original).unwrap();
        assert!(directory.load_capability(lease.record()).is_err());
    }

    #[test]
    fn mismatched_filename_and_embedded_targets_abort_cleanup_before_any_deletion() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("records");
        let directory = DiscoveryDirectory::open(&root).unwrap();

        let capability_b = Capability::generate().unwrap();
        let published_b = directory
            .publish(record(&root, &capability_b), &capability_b)
            .unwrap();
        let binding_a = BindingId::new();
        let record_a_path = root.join(format!("{binding_a}.json"));
        let capability_a_path = root.join(format!("{binding_a}.capability"));
        atomic_publish(&capability_a_path, &[0xA5; 32]).unwrap();

        let mut mismatched_a = published_b.record().clone();
        mismatched_a.created_unix_millis = 1;
        mismatched_a.refreshed_unix_millis = 1;
        mismatched_a.expires_unix_millis = 2;
        atomic_publish(&record_a_path, &serde_json::to_vec(&mismatched_a).unwrap()).unwrap();

        let paths = [
            record_a_path,
            capability_a_path,
            published_b.record_path.clone(),
            published_b.capability_path.clone(),
        ];
        let bytes_before = paths
            .iter()
            .map(|path| fs::read(path).unwrap())
            .collect::<Vec<_>>();
        assert!(directory.records().is_err());
        let mut endpoint_checks = 0;
        assert!(
            directory
                .remove_stale_at(3, &mut |_| {
                    endpoint_checks += 1;
                    true
                })
                .is_err()
        );
        assert_eq!(endpoint_checks, 0);
        for (path, expected) in paths.iter().zip(bytes_before) {
            assert_eq!(fs::read(path).unwrap(), expected);
        }
    }

    #[cfg(unix)]
    #[test]
    fn mismatched_filename_identity_preserves_unix_endpoint_inodes() {
        use std::os::unix::fs::MetadataExt;

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("records");
        let directory = DiscoveryDirectory::open(&root).unwrap();

        let capability_b = Capability::generate().unwrap();
        let record_b = record(&root, &capability_b);
        let listener_b = crate::LocalListener::bind(&record_b.endpoint).unwrap();
        let published_b = directory.publish(record_b, &capability_b).unwrap();

        let binding_a = BindingId::new();
        let endpoint_a = test_endpoint(&root, binding_a);
        let listener_a = crate::LocalListener::bind(&endpoint_a).unwrap();
        let record_a_path = root.join(format!("{binding_a}.json"));
        let capability_a_path = root.join(format!("{binding_a}.capability"));
        atomic_publish(&capability_a_path, &[0xA5; 32]).unwrap();

        let mut mismatched_a = published_b.record().clone();
        mismatched_a.created_unix_millis = 1;
        mismatched_a.refreshed_unix_millis = 1;
        mismatched_a.expires_unix_millis = 2;
        atomic_publish(&record_a_path, &serde_json::to_vec(&mismatched_a).unwrap()).unwrap();

        let record_b_path = published_b.record_path.clone();
        let capability_b_path = published_b.capability_path.clone();
        let files_before = [
            fs::read(&record_a_path).unwrap(),
            fs::read(&capability_a_path).unwrap(),
            fs::read(&record_b_path).unwrap(),
            fs::read(&capability_b_path).unwrap(),
        ];
        let EndpointAddress::Unix {
            path: endpoint_a_path,
        } = &endpoint_a
        else {
            unreachable!()
        };
        let EndpointAddress::Unix {
            path: endpoint_b_path,
        } = &published_b.record().endpoint
        else {
            unreachable!()
        };
        let endpoint_a_identity = fs::metadata(endpoint_a_path).unwrap().ino();
        let endpoint_b_identity = fs::metadata(endpoint_b_path).unwrap().ino();

        assert!(directory.records().is_err());
        let mut endpoint_checks = 0;
        assert!(
            directory
                .remove_stale_at(3, &mut |_| {
                    endpoint_checks += 1;
                    true
                })
                .is_err()
        );
        assert_eq!(endpoint_checks, 0);
        assert_eq!(fs::read(&record_a_path).unwrap(), files_before[0]);
        assert_eq!(fs::read(&capability_a_path).unwrap(), files_before[1]);
        assert_eq!(fs::read(&record_b_path).unwrap(), files_before[2]);
        assert_eq!(fs::read(&capability_b_path).unwrap(), files_before[3]);
        assert_eq!(
            fs::metadata(endpoint_a_path).unwrap().ino(),
            endpoint_a_identity
        );
        assert_eq!(
            fs::metadata(endpoint_b_path).unwrap().ino(),
            endpoint_b_identity
        );
        std::hint::black_box((&listener_a, &listener_b));
    }

    #[cfg(unix)]
    #[test]
    fn killed_host_residuals_are_scavenged_only_after_endpoint_failure_and_expiry() {
        use std::process::Command;

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("records");
        let ready = temporary.path().join("ready");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("crashed_discovery_process_helper")
            .arg("--nocapture")
            .env("SPECTRUM_BRIDGE_CRASH_DISCOVERY", &root)
            .env("SPECTRUM_BRIDGE_CRASH_DISCOVERY_READY", &ready)
            .spawn()
            .unwrap();
        for _ in 0..200 {
            if ready.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            ready.exists(),
            "crash helper did not publish its ready marker"
        );
        child.kill().unwrap();
        child.wait().unwrap();

        let directory = DiscoveryDirectory::open(&root).unwrap();
        let residual = directory.records().unwrap().pop().unwrap();
        assert!(matches!(&residual.endpoint, EndpointAddress::Unix { .. }));
        let removed = directory
            .remove_stale_at(residual.expires_unix_millis + 1, &mut |endpoint| {
                LocalStream::connect(endpoint).is_err()
            })
            .unwrap();
        assert_eq!(removed, 1);
        assert!(directory.records().unwrap().is_empty());
        let EndpointAddress::Unix { path } = residual.endpoint else {
            unreachable!()
        };
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn crashed_discovery_process_helper() {
        let Ok(root) = std::env::var("SPECTRUM_BRIDGE_CRASH_DISCOVERY") else {
            return;
        };
        let ready = std::env::var("SPECTRUM_BRIDGE_CRASH_DISCOVERY_READY").unwrap();
        let root = PathBuf::from(root);
        let directory = DiscoveryDirectory::open(&root).unwrap();
        let capability = Capability::generate().unwrap();
        let record = record(&root, &capability);
        let listener = crate::LocalListener::bind(&record.endpoint).unwrap();
        directory
            .publish(record, &capability)
            .unwrap()
            .preserve_files();
        fs::write(ready, b"ready").unwrap();
        std::hint::black_box((&listener, &capability));
        loop {
            thread::park();
        }
    }

    #[cfg(windows)]
    #[test]
    fn discovery_files_have_current_logon_owner_and_exact_user_dacl() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("records");
        let directory = DiscoveryDirectory::open(&root).unwrap();
        let capability = Capability::generate().unwrap();
        let published = directory
            .publish(record(&root, &capability), &capability)
            .unwrap();
        crate::windows_security::verify_private_acl(&root).unwrap();
        crate::windows_security::verify_private_acl(&published.record_path).unwrap();
        crate::windows_security::verify_private_acl(&published.capability_path).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn write_through_refresh_preserves_acl_and_failed_replace_cleans_temporary() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("records");
        let directory = DiscoveryDirectory::open(&root).unwrap();
        let capability = Capability::generate().unwrap();
        let mut published = directory
            .publish(record(&root, &capability), &capability)
            .unwrap();

        published.refresh(7, 9).unwrap();
        let refreshed: DiscoveryRecord =
            serde_json::from_slice(&fs::read(&published.record_path).unwrap()).unwrap();
        assert_eq!(
            (refreshed.oldest_event_seq, refreshed.newest_event_seq),
            (7, 9)
        );
        crate::windows_security::verify_private_acl(&published.record_path).unwrap();

        let blocked = root.join("blocked.json");
        fs::create_dir(&blocked).unwrap();
        assert!(atomic_publish(&blocked, b"cannot replace a directory").is_err());
        assert!(fs::read_dir(&root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".bridge-")
        }));
    }

    #[test]
    fn public_endpoint_mapping_is_the_mapping_publication_accepts() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("records");
        let directory = DiscoveryDirectory::open(&root).unwrap();
        let capability = Capability::generate().unwrap();
        let record = record(&root, &capability);
        let endpoint = directory.endpoint_for(record.binding_id);
        assert_eq!(record.endpoint, endpoint);

        let published = directory.publish(record, &capability).unwrap();
        assert_eq!(published.record().endpoint, endpoint);
    }
}
