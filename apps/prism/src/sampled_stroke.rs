use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{SampledSourceId, SampledSourceMapping};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SampledBrushSource {
    /// Authoring-only marker resolved before durable apply or replay.
    CurrentClone,
    CloneStamp {
        source_id: SampledSourceId,
        mapping: SampledSourceMapping,
    },
}

impl SampledBrushSource {
    pub(crate) fn resolved_clone(
        source_id: SampledSourceId,
        mapping: SampledSourceMapping,
    ) -> Self {
        Self::CloneStamp { source_id, mapping }
    }

    pub(crate) fn source_id(&self) -> Option<&SampledSourceId> {
        match self {
            Self::CurrentClone => None,
            Self::CloneStamp { source_id, .. } => Some(source_id),
        }
    }

    pub(crate) fn mapping(&self) -> Option<SampledSourceMapping> {
        match self {
            Self::CurrentClone => None,
            Self::CloneStamp { mapping, .. } => Some(*mapping),
        }
    }

    pub(crate) fn identity(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        match self {
            Self::CurrentClone => hash.update([0]),
            Self::CloneStamp { source_id, mapping } => {
                hash.update([1]);
                hash.update(source_id.as_str().as_bytes());
                hash.update(
                    serde_json::to_vec(mapping).expect("sampled mapping serialization cannot fail"),
                );
            }
        }
        hash.finalize().into()
    }
}
