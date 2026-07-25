use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use spectrum_revisions::ProjectId;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    BindingId, BridgeError, BridgeResult, Capability, CapabilityId, DISCOVERY_EXPIRY,
    DISCOVERY_FAMILY, InstanceId, PROTOCOL_VERSION,
};

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
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
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

    pub fn records(&self) -> BridgeResult<Vec<DiscoveryRecord>> {
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let bytes = secure_read(&entry.path(), crate::MAX_FRAME_BYTES)?;
            let record: DiscoveryRecord = serde_json::from_slice(&bytes)?;
            record.validate()?;
            records.push(record);
        }
        records.sort_by_key(|record| (record.application.clone(), record.binding_id.to_string()));
        Ok(records)
    }

    pub fn remove_stale<F>(&self, mut endpoint_failed: F) -> BridgeResult<usize>
    where
        F: FnMut(&EndpointAddress) -> bool,
    {
        let now = unix_millis()?;
        let mut removed = 0;
        for record in self.records()? {
            if record.is_expired(now) && endpoint_failed(&record.endpoint) {
                let record_path = self.root.join(format!("{}.json", record.binding_id));
                let capability_path = self.root.join(format!("{}.capability", record.binding_id));
                if secure_owned_file(&record_path).is_ok()
                    && secure_owned_file(&capability_path).is_ok()
                {
                    fs::remove_file(record_path)?;
                    fs::remove_file(capability_path)?;
                    removed += 1;
                }
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
        let mut secret = secure_read(&expected, 32)?;
        let bytes: [u8; 32] = secret
            .as_slice()
            .try_into()
            .map_err(|_| BridgeError::Authentication("capability has wrong length".into()))?;
        secret.zeroize();
        Ok(Capability::from_secret(record.capability_id, bytes))
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
    published: PublishedBinding,
}

impl DiscoveryLease {
    pub fn new(published: PublishedBinding) -> Self {
        Self { published }
    }

    pub fn refresh(&mut self, oldest_event_seq: u64, newest_event_seq: u64) -> BridgeResult<()> {
        self.published.refresh(oldest_event_seq, newest_event_seq)
    }

    pub fn record(&self) -> &DiscoveryRecord {
        self.published.record()
    }
}

impl DiscoveryRecord {
    fn refresh(&mut self) -> BridgeResult<()> {
        self.refreshed_unix_millis = unix_millis()?;
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
        fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        secure_owned_file(path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
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
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SetFileSecurityW,
        },
        System::SystemServices::SECURITY_DESCRIPTOR_REVISION,
    };

    let sddl: Vec<u16> = "D:P(A;;GA;;;OW)\0".encode_utf16().collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SECURITY_DESCRIPTOR_REVISION,
            &raw mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let applied = unsafe { SetFileSecurityW(wide.as_ptr(), DACL_SECURITY_INFORMATION, descriptor) };
    unsafe {
        LocalFree(descriptor);
    }
    if applied == 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        verify_private_acl(path)
    }
}

#[cfg(windows)]
fn verify_private_acl(path: &Path) -> BridgeResult<()> {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            ACL, Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            DACL_SECURITY_INFORMATION, GetFileSecurityW, PSECURITY_DESCRIPTOR,
        },
        System::SystemServices::SECURITY_DESCRIPTOR_REVISION,
    };

    fn dacl(descriptor: PSECURITY_DESCRIPTOR) -> BridgeResult<(*mut ACL, usize)> {
        use windows_sys::Win32::Security::{
            ACL, ACL_SIZE_INFORMATION, AclSizeInformation, GetAclInformation,
            GetSecurityDescriptorDacl,
        };
        let mut present = 0;
        let mut defaulted = 0;
        let mut acl: *mut ACL = std::ptr::null_mut();
        if unsafe {
            GetSecurityDescriptorDacl(
                descriptor,
                &raw mut present,
                &raw mut acl,
                &raw mut defaulted,
            )
        } == 0
            || present == 0
            || acl.is_null()
        {
            return Err(BridgeError::Authentication("private DACL is absent".into()));
        }
        let mut size = ACL_SIZE_INFORMATION {
            AceCount: 0,
            AclBytesInUse: 0,
            AclBytesFree: 0,
        };
        if unsafe {
            GetAclInformation(
                acl,
                (&raw mut size).cast::<c_void>(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok((acl, size.AclBytesInUse as usize))
    }

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut needed = 0;
    unsafe {
        GetFileSecurityW(
            wide.as_ptr(),
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            0,
            &raw mut needed,
        );
    }
    if needed == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut actual = vec![0_u8; needed as usize];
    if unsafe {
        GetFileSecurityW(
            wide.as_ptr(),
            DACL_SECURITY_INFORMATION,
            actual.as_mut_ptr().cast(),
            needed,
            &raw mut needed,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }

    let sddl: Vec<u16> = "D:P(A;;GA;;;OW)\0".encode_utf16().collect();
    let mut expected: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SECURITY_DESCRIPTOR_REVISION,
            &raw mut expected,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let comparison = (|| -> BridgeResult<bool> {
        let (actual_acl, actual_len) = dacl(actual.as_mut_ptr().cast())?;
        let (expected_acl, expected_len) = dacl(expected)?;
        if actual_len != expected_len {
            return Ok(false);
        }
        let actual_bytes =
            unsafe { std::slice::from_raw_parts(actual_acl.cast::<u8>(), actual_len) };
        let expected_bytes =
            unsafe { std::slice::from_raw_parts(expected_acl.cast::<u8>(), expected_len) };
        Ok(actual_bytes == expected_bytes)
    })();
    unsafe {
        LocalFree(expected);
    }
    if comparison? {
        Ok(())
    } else {
        Err(BridgeError::Authentication(
            "binding path does not have the user-only protected DACL".into(),
        ))
    }
}

#[cfg(windows)]
fn verify_open_file(file: &File) -> BridgeResult<()> {
    if !file.metadata()?.is_file() {
        return Err(BridgeError::Authentication(
            "opened binding file is not regular".into(),
        ));
    }
    Ok(())
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
    use super::*;

    fn record(root: &Path, capability: &Capability) -> DiscoveryRecord {
        let binding_id = BindingId::new();
        DiscoveryRecord {
            family: DISCOVERY_FAMILY.into(),
            protocol_min: 1,
            protocol_max: 1,
            application: "test-host".into(),
            project_id: ProjectId::new(),
            canonical_project_path: root.join("project.test"),
            instance_id: InstanceId::new(),
            binding_id,
            binding_epoch: 1,
            endpoint: EndpointAddress::Unix {
                path: root.join("bridge.sock"),
            },
            capability_id: capability.id(),
            capability_path: root.join(format!("{binding_id}.capability")),
            process_id: std::process::id(),
            command_versions: BTreeMap::new(),
            capabilities: Vec::new(),
            oldest_event_seq: 0,
            newest_event_seq: 0,
            refreshed_unix_millis: 0,
            expires_unix_millis: 0,
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
}
