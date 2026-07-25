use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use spectrum_live_bridge::ActionEnvelope;
use spectrum_revisions::{
    Collaboration, CollaborationSync, ProjectId, Revision, RevisionId, SessionId, TrackId,
};

use crate::{Command, CommandOutput, LiveWorkspaceState};

pub const LUMEN_LIVE_ACTION_FAMILY: &str = "spectrum.lumen.live";
pub const LUMEN_LIVE_ACTION_VERSION: u32 = 1;
pub const LUMEN_LIVE_APPLICATION: &str = "spectrum.lumen";
pub const LUMEN_COMMAND_OPERATIONS_VERSION: u32 = 1;

pub fn lumen_live_discovery_root() -> Result<PathBuf> {
    Ok(lumen_live_discovery_root_in(
        &eframe::storage_dir("Spectrum")
            .context("Spectrum could not locate its per-user application directory")?,
    ))
}

fn lumen_live_discovery_root_in(spectrum_storage: &Path) -> PathBuf {
    spectrum_storage
        .join("LiveBridge")
        .join(format!("v{}", spectrum_live_bridge::PROTOCOL_VERSION))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LumenLiveActionExpectation {
    pub photo_id: u64,
    pub track_id: TrackId,
    pub agent_revision: RevisionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_revision: Option<RevisionId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum LumenLiveAction {
    State,
    ExecuteBatch {
        expectation: LumenLiveActionExpectation,
        command_version: u32,
        commands: Vec<Command>,
    },
    Undo {
        expectation: LumenLiveActionExpectation,
    },
    Redo {
        expectation: LumenLiveActionExpectation,
    },
    MoveAgentCursor {
        expectation: LumenLiveActionExpectation,
        target: RevisionId,
    },
}

impl LumenLiveAction {
    pub fn validate(&self) -> Result<()> {
        let Self::ExecuteBatch {
            command_version,
            commands,
            ..
        } = self
        else {
            return Ok(());
        };
        if commands.is_empty() {
            bail!("live command batch is empty");
        }
        if commands.len() > spectrum_live_bridge::MAX_BATCH_ITEMS {
            bail!(
                "live command batch exceeds {} commands",
                spectrum_live_bridge::MAX_BATCH_ITEMS
            );
        }
        if *command_version == 0 || *command_version > LUMEN_COMMAND_OPERATIONS_VERSION {
            bail!("unsupported Lumen command operation version {command_version}");
        }
        let expectation = self
            .expectation()
            .context("live command batch lost its exact photo expectation")?;
        for command in commands {
            validate_photo_command(command, expectation.photo_id)?;
        }
        Ok(())
    }

    pub fn expectation(&self) -> Option<&LumenLiveActionExpectation> {
        match self {
            Self::State => None,
            Self::ExecuteBatch { expectation, .. }
            | Self::Undo { expectation }
            | Self::Redo { expectation }
            | Self::MoveAgentCursor { expectation, .. } => Some(expectation),
        }
    }
}

fn validate_photo_command(command: &Command, photo_id: u64) -> Result<()> {
    let command_photo = match command {
        Command::Adjust { id, .. }
        | Command::SetAdjustments { id, .. }
        | Command::Rotate { id, .. }
        | Command::FlipHorizontal { id }
        | Command::FlipVertical { id } => *id,
        Command::Reset { ids } if ids.as_slice() == [photo_id] => photo_id,
        Command::ApplyPreset { ids, .. } if ids.as_slice() == [photo_id] => photo_id,
        Command::Reset { .. } | Command::ApplyPreset { .. } => {
            bail!("live reset and preset application must target exactly photo {photo_id}")
        }
        Command::CopyEdits { .. } | Command::PasteEdits { .. } => {
            bail!("live copy/paste must be lowered to an explicit SetAdjustments command")
        }
        Command::New { .. }
        | Command::Open { .. }
        | Command::Save { .. }
        | Command::Import { .. }
        | Command::Select { .. }
        | Command::SetPick { .. }
        | Command::RenameBatch { .. }
        | Command::HistoryBack { .. }
        | Command::HistoryForward { .. }
        | Command::HistoryJump { .. }
        | Command::SavePreset { .. }
        | Command::DeletePreset { .. }
        | Command::Remove { .. }
        | Command::Export { .. }
        | Command::ExportBatch { .. }
        | Command::Undo
        | Command::Redo => {
            bail!("command is not allowed in a photo-local Lumen live batch")
        }
    };
    if command_photo != photo_id {
        bail!(
            "live command targets photo {command_photo}, expected bound collaboration photo {photo_id}"
        );
    }
    Ok(())
}

pub fn decode_live_action(envelope: &ActionEnvelope) -> Result<LumenLiveAction> {
    envelope.validate().map_err(anyhow::Error::from)?;
    if envelope.family != LUMEN_LIVE_ACTION_FAMILY || envelope.version != LUMEN_LIVE_ACTION_VERSION
    {
        bail!(
            "unsupported Lumen live action family/version {}@{}",
            envelope.family,
            envelope.version
        );
    }
    if !envelope.capabilities.is_empty() {
        bail!(
            "unsupported Lumen live action capabilities: {}",
            envelope.capabilities.join(", ")
        );
    }
    let action: LumenLiveAction =
        serde_json::from_value(envelope.action.clone()).context("invalid Lumen live action")?;
    action.validate()?;
    Ok(action)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LumenLiveState {
    pub project_id: ProjectId,
    pub catalog_track_id: TrackId,
    pub catalog_cursor: RevisionId,
    pub photo_id: u64,
    pub photo_track_id: TrackId,
    pub human_session: SessionId,
    pub human_photo_cursor: RevisionId,
    pub agent_session: SessionId,
    pub agent_photo_cursor: RevisionId,
    pub collaboration: Collaboration,
    pub command_version: u32,
}

impl LumenLiveState {
    pub(crate) fn new(
        human: &LiveWorkspaceState,
        agent: &LiveWorkspaceState,
        collaboration: Collaboration,
    ) -> Self {
        Self {
            project_id: human.project_id,
            catalog_track_id: human.catalog_track_id,
            catalog_cursor: human.catalog_cursor,
            photo_id: human.photo_id,
            photo_track_id: human.photo_track_id,
            human_session: human.session_id,
            human_photo_cursor: human.photo_cursor,
            agent_session: agent.session_id,
            agent_photo_cursor: agent.photo_cursor,
            collaboration,
            command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LumenLiveApplied {
    pub before: LumenLiveState,
    pub after: LumenLiveState,
    pub outputs: Vec<CommandOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub committed_revision: Option<Revision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collaboration_sync: Option<LumenLiveCollaborationSync>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LumenLiveCollaborationSync {
    Waiting,
    Advanced { from: RevisionId, to: RevisionId },
    Split,
}

impl From<CollaborationSync> for LumenLiveCollaborationSync {
    fn from(value: CollaborationSync) -> Self {
        match value {
            CollaborationSync::Idle | CollaborationSync::Waiting(_) => Self::Waiting,
            CollaborationSync::Advanced { from, to, .. } => Self::Advanced { from, to },
            CollaborationSync::Split(_) => Self::Split,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LumenLiveResult {
    State(Box<LumenLiveState>),
    Applied(Box<LumenLiveApplied>),
}

#[cfg(test)]
mod tests {
    use std::fs;

    use spectrum_live_bridge::DiscoveryDirectory;

    use super::*;

    #[test]
    fn protocol_discovery_roots_isolate_crash_residuals_in_both_directions() {
        let temporary = tempfile::tempdir().unwrap();
        let storage = temporary.path().join("Spectrum");
        let bridge = storage.join("LiveBridge");
        let legacy = bridge.join("v1");
        let current = lumen_live_discovery_root_in(&storage);
        assert_eq!(current, bridge.join("v2"));

        fs::create_dir_all(&legacy).unwrap();
        let legacy_residual = legacy.join("legacy-crash.json");
        fs::write(
            &legacy_residual,
            br#"{"family":"spectrum.live.discovery","protocol_min":1,"protocol_max":1}"#,
        )
        .unwrap();
        let current_directory = DiscoveryDirectory::open(&current).unwrap();
        assert!(current_directory.records().unwrap().is_empty());
        assert!(legacy_residual.exists());

        let current_residual = current.join("current-crash.json");
        fs::write(&current_residual, b"v2 crash residual").unwrap();
        assert_eq!(
            fs::read_dir(&legacy)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>(),
            vec![legacy_residual]
        );
        assert!(current_residual.exists());
    }
}
