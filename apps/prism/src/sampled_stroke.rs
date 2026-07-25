use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::SampledSourceSnapshot;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SampledBrushSource {
    /// Authoring-only marker resolved before durable apply or replay.
    CurrentClone,
    CloneStamp {
        source: Box<SampledSourceSnapshot>,
    },
}

impl SampledBrushSource {
    pub(crate) fn resolved_clone(source: SampledSourceSnapshot) -> Self {
        Self::CloneStamp {
            source: Box::new(source),
        }
    }

    pub(crate) fn source(&self) -> Option<&SampledSourceSnapshot> {
        match self {
            Self::CurrentClone => None,
            Self::CloneStamp { source } => Some(source),
        }
    }

    pub(crate) fn source_mut(&mut self) -> Option<&mut SampledSourceSnapshot> {
        match self {
            Self::CurrentClone => None,
            Self::CloneStamp { source } => Some(source),
        }
    }

    pub(crate) fn identity(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        match self {
            Self::CurrentClone => hash.update([0]),
            Self::CloneStamp { source } => {
                hash.update([1]);
                hash.update(
                    serde_json::to_vec(source).expect("sampled source serialization cannot fail"),
                );
            }
        }
        hash.finalize().into()
    }
}
