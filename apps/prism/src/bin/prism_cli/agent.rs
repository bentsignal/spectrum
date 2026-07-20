use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::{Subcommand, ValueEnum};
use prism_core::Workspace;
use serde_json::{Value, json};
use spectrum_revisions::{Actor, ActorKind, CollaborationMode, SessionId};

#[derive(Subcommand)]
pub(super) enum AgentCommand {
    /// Start from the current human position and return a persistent agent session.
    Start {
        #[arg(long, value_enum)]
        mode: CliAgentMode,
        #[arg(long, default_value = "Agent")]
        name: String,
        /// Choose a specific human session instead of the most recently active one.
        #[arg(long)]
        from_session: Option<SessionId>,
    },
    /// Inspect this agent session's mode, cursor, and follow status.
    Status,
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum CliAgentMode {
    Together,
    Separate,
}

impl From<CliAgentMode> for CollaborationMode {
    fn from(value: CliAgentMode) -> Self {
        match value {
            CliAgentMode::Together => Self::Together,
            CliAgentMode::Separate => Self::Separate,
        }
    }
}

pub(super) fn agent_command(
    path: &Path,
    session: Option<SessionId>,
    command: AgentCommand,
) -> Result<Value> {
    match command {
        AgentCommand::Start {
            mode,
            name,
            from_session,
        } => {
            let name = name.trim();
            if name.is_empty() {
                bail!("agent name cannot be empty");
            }
            let actor = Actor {
                id: format!("external-agent:{}", SessionId::new()),
                display_name: name.into(),
                kind: ActorKind::Agent,
            };
            let mode = CollaborationMode::from(mode);
            let collaboration = match from_session {
                Some(source) => Workspace::start_collaboration(path, Some(source), actor, mode)?,
                None => match local_gui_session_id() {
                    Some(source) => {
                        Workspace::start_collaboration(path, Some(source), actor.clone(), mode)
                            .or_else(|_| Workspace::start_collaboration(path, None, actor, mode))?
                    }
                    None => Workspace::start_collaboration(path, None, actor, mode)?,
                },
            };
            Ok(json!({
                "ok": true,
                "action": "agent_start",
                "project": path,
                "mode": collaboration.mode,
                "session": collaboration.agent_session,
                "source_session": collaboration.source_session,
                "base_revision": collaboration.base_revision,
                "status": collaboration.status,
                "use_for_every_command": [
                    "--project", path,
                    "--session", collaboration.agent_session.to_string()
                ],
                "behavior": match collaboration.mode {
                    CollaborationMode::Together => "Prism follows agent revisions until the human makes a competing edit",
                    CollaborationMode::Separate => "the human canvas stays on its own session while the agent explores",
                }
            }))
        }
        AgentCommand::Status => {
            let session = session.context("agent status requires --session <SESSION_ID>")?;
            let collaboration = Workspace::collaboration(path, session)?;
            let workspace = Workspace::open_session(path, session)?;
            let cursor = workspace
                .history()?
                .context("agent session does not have durable history")?
                .current;
            Ok(json!({
                "ok": true,
                "action": "agent_status",
                "project": path,
                "session": session,
                "cursor": cursor,
                "collaboration": collaboration,
            }))
        }
    }
}

fn local_gui_session_id() -> Option<SessionId> {
    let directory = eframe::storage_dir("Spectrum")?;
    spectrum_revisions::local_session_id(&directory).ok()
}
