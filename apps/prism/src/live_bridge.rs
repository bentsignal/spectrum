use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use spectrum_live_bridge::ActionEnvelope;
use spectrum_revisions::{
    Collaboration, CollaborationSync, ProjectId, Revision, RevisionId, SessionId, TrackId,
};

use crate::{
    Command, CommandOutput, LiveWorkspaceState, PRISM_COMMAND_OPERATIONS_VERSION,
    required_command_operations_version,
};

pub const PRISM_LIVE_ACTION_FAMILY: &str = "spectrum.prism.live";
pub const PRISM_LIVE_ACTION_VERSION: u32 = 1;
pub const PRISM_LIVE_APPLICATION: &str = "spectrum.prism";

pub fn prism_live_discovery_root() -> Result<PathBuf> {
    Ok(prism_live_discovery_root_in(
        &eframe::storage_dir("Spectrum")
            .context("Spectrum could not locate its per-user application directory")?,
    ))
}

fn prism_live_discovery_root_in(spectrum_storage: &Path) -> PathBuf {
    spectrum_storage
        .join("LiveBridge")
        .join(format!("v{}", spectrum_live_bridge::PROTOCOL_VERSION))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrismLiveActionExpectation {
    pub agent_revision: RevisionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_revision: Option<RevisionId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrismLiveAction {
    State,
    ExecuteBatch {
        expectation: PrismLiveActionExpectation,
        command_version: u32,
        commands: Vec<Command>,
    },
    Undo {
        expectation: PrismLiveActionExpectation,
    },
    Redo {
        expectation: PrismLiveActionExpectation,
    },
    MoveAgentCursor {
        expectation: PrismLiveActionExpectation,
        target: RevisionId,
    },
}

impl PrismLiveAction {
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
        if *command_version == 0 || *command_version > PRISM_COMMAND_OPERATIONS_VERSION {
            bail!("unsupported Prism command operation version {command_version}");
        }
        if commands.iter().any(|command| {
            matches!(
                command,
                Command::Undo | Command::Redo | Command::SelectLayer { .. }
            )
        }) {
            bail!("history and UI-local selection commands cannot be nested in a live batch");
        }
        let required = required_command_operations_version(commands);
        if required > *command_version {
            bail!(
                "live batch declares command version {command_version} but requires version {required}"
            );
        }
        Ok(())
    }

    pub fn expectation(&self) -> Option<&PrismLiveActionExpectation> {
        match self {
            Self::State => None,
            Self::ExecuteBatch { expectation, .. }
            | Self::Undo { expectation }
            | Self::Redo { expectation }
            | Self::MoveAgentCursor { expectation, .. } => Some(expectation),
        }
    }
}

pub fn decode_live_action(envelope: &ActionEnvelope) -> Result<PrismLiveAction> {
    envelope.validate().map_err(anyhow::Error::from)?;
    if envelope.family != PRISM_LIVE_ACTION_FAMILY || envelope.version != PRISM_LIVE_ACTION_VERSION
    {
        bail!(
            "unsupported Prism live action family/version {}@{}",
            envelope.family,
            envelope.version
        );
    }
    if !envelope.capabilities.is_empty() {
        bail!(
            "unsupported Prism live action capabilities: {}",
            envelope.capabilities.join(", ")
        );
    }
    let action: PrismLiveAction =
        serde_json::from_value(envelope.action.clone()).context("invalid Prism live action")?;
    action.validate()?;
    Ok(action)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PrismLiveState {
    pub project_id: ProjectId,
    pub track_id: TrackId,
    pub human_session: SessionId,
    pub human_cursor: RevisionId,
    pub agent_session: SessionId,
    pub agent_cursor: RevisionId,
    pub collaboration: Collaboration,
    pub command_version: u32,
}

impl PrismLiveState {
    pub(crate) fn new(
        human: &LiveWorkspaceState,
        agent: &LiveWorkspaceState,
        collaboration: Collaboration,
    ) -> Self {
        Self {
            project_id: human.project_id,
            track_id: human.track_id,
            human_session: human.session_id,
            human_cursor: human.cursor,
            agent_session: agent.session_id,
            agent_cursor: agent.cursor,
            collaboration,
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PrismLiveApplied {
    pub before: PrismLiveState,
    pub after: PrismLiveState,
    pub outputs: Vec<CommandOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub committed_revision: Option<Revision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collaboration_sync: Option<PrismLiveCollaborationSync>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PrismLiveCollaborationSync {
    Waiting,
    Advanced { from: RevisionId, to: RevisionId },
    Split,
}

impl From<CollaborationSync> for PrismLiveCollaborationSync {
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
pub enum PrismLiveResult {
    State(PrismLiveState),
    Applied(Box<PrismLiveApplied>),
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
        let current = prism_live_discovery_root_in(&storage);
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
