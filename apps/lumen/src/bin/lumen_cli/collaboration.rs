use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use spectrum_revisions::{Actor, ActorKind, CollaborationMode, SessionId};

use super::{AgentCommand, Command, Workspace};

pub(super) fn agent_command(
    path: &Path,
    session: Option<SessionId>,
    command: AgentCommand,
) -> Result<Value> {
    match command {
        AgentCommand::Start {
            photo_id,
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
                Some(source) => {
                    Workspace::start_collaboration(path, Some(source), photo_id, actor, mode)?
                }
                None => match local_gui_session_id() {
                    Some(source) => Workspace::start_collaboration(
                        path,
                        Some(source),
                        photo_id,
                        actor.clone(),
                        mode,
                    )
                    .or_else(|_| {
                        Workspace::start_collaboration(path, None, photo_id, actor, mode)
                    })?,
                    None => Workspace::start_collaboration(path, None, photo_id, actor, mode)?,
                },
            };
            Ok(json!({
                "ok": true,
                "action": "agent_start",
                "project": path,
                "mode": collaboration.mode,
                "photo_id": photo_id,
                "track_id": collaboration.track_id,
                "session": collaboration.agent_session,
                "source_session": collaboration.source_session,
                "base_revision": collaboration.base_revision,
                "status": collaboration.status,
                "use_for_every_command": [
                    "--catalog", path,
                    "--session", collaboration.agent_session.to_string()
                ],
                "behavior": match collaboration.mode {
                    CollaborationMode::Together => "Lumen follows agent revisions for this photo until the human makes a competing edit on the same photo",
                    CollaborationMode::Separate => "the human project stays on its own session while the agent explores",
                }
            }))
        }
        AgentCommand::Status => {
            let session = session.context("agent status requires --session <SESSION_ID>")?;
            let collaboration = Workspace::collaboration(path, session)?;
            let workspace = Workspace::open_session(path, session)?;
            let history = workspace
                .project
                .photos
                .iter()
                .find_map(|photo| {
                    workspace
                        .history_for(photo.id)
                        .ok()
                        .flatten()
                        .filter(|history| history.track_id == collaboration.track_id)
                })
                .context("agent session's photo track is unavailable")?;
            Ok(json!({
                "ok": true,
                "action": "agent_status",
                "project": path,
                "session": session,
                "photo_id": history.photo_id,
                "track_id": history.track_id,
                "cursor": history.current,
                "collaboration": collaboration,
            }))
        }
    }
}

fn local_gui_session_id() -> Option<SessionId> {
    let directory = eframe::storage_dir("Spectrum")?;
    spectrum_revisions::local_session_id(&directory).ok()
}

pub(super) fn cli_actor() -> Actor {
    Actor {
        id: "local:lumen-cli".into(),
        display_name: "Lumen CLI".into(),
        kind: ActorKind::Agent,
    }
}

pub(super) fn run_commands(
    workspace: &mut Workspace,
    value: &str,
) -> Result<Vec<lumen_core::CommandOutput>> {
    if value.trim_start().starts_with('[') {
        workspace.execute_batch(serde_json::from_str::<Vec<Command>>(value)?)
    } else {
        Ok(vec![workspace.execute(serde_json::from_str(value)?)?])
    }
}
