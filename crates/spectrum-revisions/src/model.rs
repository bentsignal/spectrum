use serde::{Deserialize, Serialize};

use crate::{AssetId, ChangeSetId, ProjectId, RevisionId, SessionId, TrackId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Human,
    Agent,
    System,
}

impl ActorKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
            Self::System => "system",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "human" => Some(Self::Human),
            "agent" => Some(Self::Agent),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub id: String,
    pub display_name: String,
    pub kind: ActorKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Encoding {
    pub family: String,
    pub version: u32,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
}

impl Encoding {
    pub fn new(family: impl Into<String>, version: u32) -> Self {
        Self {
            family: family.into(),
            version,
            required_capabilities: Vec::new(),
        }
    }

    pub fn requiring(mut self, capability: impl Into<String>) -> Self {
        self.required_capabilities.push(capability.into());
        self.normalize();
        self
    }

    pub(crate) fn normalize(&mut self) {
        self.required_capabilities.sort();
        self.required_capabilities.dedup();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payload {
    pub encoding: Encoding,
    pub bytes: Vec<u8>,
}

impl Payload {
    pub fn new(encoding: Encoding, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            encoding,
            bytes: bytes.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct NewProject {
    pub application_id: String,
    pub application_version: String,
    pub actor: Actor,
    pub session_id: SessionId,
    pub root_label: Option<String>,
    pub track_kind: String,
    pub track_label: String,
    pub initial_snapshots: Vec<Payload>,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectInfo {
    pub project_id: ProjectId,
    pub application_id: String,
    pub created_at_ms: i64,
    pub container_format: u32,
    pub default_track_id: TrackId,
    pub root_revision: RevisionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub kind: String,
    pub label: String,
    pub root_revision: RevisionId,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug)]
pub struct NewTrack {
    pub kind: String,
    pub label: String,
    pub root_label: Option<String>,
    pub initial_snapshots: Vec<Payload>,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug)]
pub struct AppendRevision {
    pub track_id: TrackId,
    pub session_id: SessionId,
    pub expected_parent: RevisionId,
    pub application_version: String,
    pub label: Option<String>,
    pub command_count: u32,
    pub operation_payloads: Vec<Payload>,
    pub snapshots: Vec<Payload>,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    pub id: RevisionId,
    pub track_id: TrackId,
    pub change_set_id: ChangeSetId,
    pub parent_id: Option<RevisionId>,
    pub actor: Actor,
    pub session_id: SessionId,
    pub created_at_ms: i64,
    pub application_version: String,
    pub label: Option<String>,
    pub command_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub actor: Actor,
    pub cursor: RevisionId,
    pub updated_at_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationMode {
    Together,
    Separate,
}

impl CollaborationMode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Together => "together",
            Self::Separate => "separate",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "together" => Some(Self::Together),
            "separate" => Some(Self::Separate),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationStatus {
    Active,
    Split,
    Superseded,
}

impl CollaborationStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Split => "split",
            Self::Superseded => "superseded",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "split" => Some(Self::Split),
            "superseded" => Some(Self::Superseded),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collaboration {
    pub agent_session: SessionId,
    pub source_session: SessionId,
    pub track_id: TrackId,
    pub base_revision: RevisionId,
    pub followed_revision: RevisionId,
    pub mode: CollaborationMode,
    pub status: CollaborationStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollaborationSync {
    Idle,
    Waiting(Collaboration),
    Advanced {
        collaboration: Collaboration,
        from: RevisionId,
        to: RevisionId,
    },
    Split(Collaboration),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayStep {
    pub revision: Revision,
    pub operations: Payload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayPlan {
    pub target: RevisionId,
    pub snapshot_revision: RevisionId,
    pub snapshot: Payload,
    pub steps: Vec<ReplayStep>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub revision_id: RevisionId,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub id: AssetId,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

impl Asset {
    pub fn new(media_type: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        let bytes = bytes.into();
        Self {
            id: AssetId::for_bytes(&bytes),
            media_type: media_type.into(),
            bytes,
        }
    }
}
