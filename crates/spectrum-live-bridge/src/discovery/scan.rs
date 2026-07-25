use std::{collections::BTreeSet, ffi::OsStr, fs, path::PathBuf};

use crate::{BindingId, BridgeError, BridgeResult};

use super::{DiscoveryDirectory, DiscoveryRecord, EndpointAddress, secure_read};

pub(super) struct ValidatedDiscoveryEntry {
    pub(super) record: DiscoveryRecord,
    pub(super) record_path: PathBuf,
    pub(super) capability_path: PathBuf,
}

impl DiscoveryDirectory {
    pub(super) fn validated_entries(&self) -> BridgeResult<Vec<ValidatedDiscoveryEntry>> {
        let mut entries = Vec::new();
        let mut binding_ids = BTreeSet::new();
        let mut capability_targets = BTreeSet::new();
        let mut endpoint_targets = BTreeSet::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let record_path = entry.path();
            let bytes = secure_read(&record_path, crate::MAX_FRAME_BYTES)?;
            let record: DiscoveryRecord = serde_json::from_slice(&bytes)?;
            record.validate()?;
            let expected_name = format!("{}.json", record.binding_id);
            if entry.file_name() != OsStr::new(&expected_name) {
                return Err(BridgeError::Authentication(
                    "discovery filename does not match embedded binding identity".into(),
                ));
            }
            let capability_path = self.root.join(format!("{}.capability", record.binding_id));
            if record.capability_path != capability_path {
                return Err(BridgeError::Authentication(
                    "discovery capability target does not match binding identity".into(),
                ));
            }
            if record.endpoint != self.expected_endpoint(record.binding_id) {
                return Err(BridgeError::Authentication(
                    "discovery endpoint target does not match binding identity".into(),
                ));
            }
            if !binding_ids.insert(record.binding_id.to_string())
                || !capability_targets.insert(capability_path.clone())
                || !endpoint_targets.insert(endpoint_identity(&record.endpoint))
            {
                return Err(BridgeError::Authentication(
                    "discovery directory contains duplicate or ambiguous binding targets".into(),
                ));
            }
            entries.push(ValidatedDiscoveryEntry {
                record,
                record_path,
                capability_path,
            });
        }
        Ok(entries)
    }

    pub(super) fn expected_endpoint(&self, binding_id: BindingId) -> EndpointAddress {
        #[cfg(unix)]
        {
            let binding = binding_id.to_string();
            EndpointAddress::Unix {
                path: self.root.join(format!("{}.sock", &binding[..8])),
            }
        }
        #[cfg(windows)]
        {
            EndpointAddress::WindowsPipe {
                name: format!(r"\\.\pipe\spectrum-live-{binding_id}"),
            }
        }
    }
}

fn endpoint_identity(endpoint: &EndpointAddress) -> String {
    match endpoint {
        EndpointAddress::Unix { path } => format!("unix:{}", path.display()),
        EndpointAddress::WindowsPipe { name } => format!("windows:{}", name.to_lowercase()),
    }
}
