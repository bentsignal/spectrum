use std::{path::Path, str::FromStr};

use anyhow::{Context, Result, bail};
use clap::{Subcommand, ValueEnum};
use lumen_core::{
    Command, LUMEN_COMMAND_OPERATIONS_VERSION, LUMEN_LIVE_ACTION_FAMILY, LUMEN_LIVE_ACTION_VERSION,
    LUMEN_LIVE_APPLICATION, LumenLiveAction, LumenLiveActionExpectation, Project, Workspace,
    lumen_live_discovery_root,
};
use serde_json::{Value, json};
use spectrum_live_bridge::{
    ActionEnvelope, BindingId, BridgeClient, ClientConfig, DiscoveryDirectory, DiscoveryRecord,
    ExpectedCursor, InteractionPolicy, PROTOCOL_FAMILY, PROTOCOL_VERSION, RequestEnvelope,
    RequestId, ResponseBody, ServerMessage,
};
use spectrum_revisions::{CollaborationMode, SessionId};

use super::{AdjustmentPatch, Cli, CliCommand, EditArgs};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(super) enum CliLiveMode {
    Off,
    Required,
}

#[derive(Clone, Subcommand)]
pub(super) enum LiveCommand {
    /// Inspect the authenticated live binding and collaboration state.
    Status,
    /// Apply one Command JSON object or an array as one live revision.
    Apply {
        json: String,
        #[arg(long, value_enum, default_value_t = CliInteractionPolicy::Immediate)]
        interaction: CliInteractionPolicy,
    },
    /// Move the agent session back one revision through the live GUI.
    Undo,
    /// Move the agent session forward one revision through the live GUI.
    Redo,
    /// Move the agent session to an exact compatible revision.
    MoveAgentCursor {
        revision: spectrum_revisions::RevisionId,
    },
    /// Subscribe to the live event stream and return a bounded event suffix.
    Subscribe {
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 1)]
        count: usize,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum CliInteractionPolicy {
    Immediate,
    Deferred,
}

impl From<CliInteractionPolicy> for InteractionPolicy {
    fn from(value: CliInteractionPolicy) -> Self {
        match value {
            CliInteractionPolicy::Immediate => Self::Immediate,
            CliInteractionPolicy::Deferred => Self::Deferred,
        }
    }
}

pub(super) fn resolved_live_mode(argument: Option<CliLiveMode>) -> Result<CliLiveMode> {
    match argument {
        Some(mode) => Ok(mode),
        None => match std::env::var("LUMEN_LIVE_MODE")
            .or_else(|_| std::env::var("SPECTRUM_LIVE_MODE"))
            .ok()
            .as_deref()
        {
            Some("required") => Ok(CliLiveMode::Required),
            Some("off") | None => Ok(CliLiveMode::Off),
            Some(value) => bail!("unsupported live mode {value:?}; expected off or required"),
        },
    }
}

pub(super) fn require_direct_mode(mode: CliLiveMode, command: &str) -> Result<()> {
    if mode == CliLiveMode::Required {
        bail!("{command} is not available with --live required");
    }
    Ok(())
}

pub(super) fn run_required_live(cli: &Cli) -> Result<Value> {
    let prepared = prepare_live_semantic(&cli.catalog, cli.session)?;
    let photo_id = prepared.photo_id;
    let commands = match &cli.command {
        CliCommand::Edit { id, adjustments } => vec![Command::Adjust {
            id: *id,
            patch: adjustment_patch(adjustments),
        }],
        CliCommand::Reset { ids } if ids.as_slice() == [photo_id] => {
            vec![Command::Reset { ids: ids.clone() }]
        }
        CliCommand::Rotate {
            id,
            counterclockwise,
        } => vec![Command::Rotate {
            id: *id,
            clockwise: !counterclockwise,
        }],
        CliCommand::Flip {
            id,
            horizontal,
            vertical,
        } => {
            if *horizontal == *vertical {
                bail!("choose exactly one of --horizontal or --vertical");
            }
            vec![if *horizontal {
                Command::FlipHorizontal { id: *id }
            } else {
                Command::FlipVertical { id: *id }
            }]
        }
        CliCommand::PresetApply { preset_id, ids } if ids.as_slice() == [photo_id] => {
            vec![Command::ApplyPreset {
                preset_id: *preset_id,
                ids: ids.clone(),
            }]
        }
        CliCommand::CopyEdits { from, to } if to.as_slice() == [photo_id] => {
            let adjustments = prepared.project.photo(*from)?.adjustments.clone();
            vec![Command::SetAdjustments {
                id: photo_id,
                adjustments,
            }]
        }
        CliCommand::HistoryBack { id } if *id == photo_id => vec![Command::Undo],
        CliCommand::HistoryForward { id } if *id == photo_id => vec![Command::Redo],
        CliCommand::Run { json } => decode_commands(json)?,
        _ => bail!(
            "this command is not allowed with --live required; use live apply for an explicit photo-local command"
        ),
    };
    live_execute_prepared(prepared, commands)
}

fn adjustment_patch(args: &EditArgs) -> AdjustmentPatch {
    AdjustmentPatch {
        exposure: args.exposure,
        temperature: args.temperature,
        tint: args.tint,
        contrast: args.contrast,
        highlights: args.highlights,
        shadows: args.shadows,
        whites: args.whites,
        blacks: args.blacks,
        texture: args.texture,
        clarity: args.clarity,
        dehaze: args.dehaze,
        vibrance: args.vibrance,
        saturation: args.saturation,
        vignette: args.vignette,
        sharpening: args.sharpening,
        noise_reduction: args.noise_reduction,
        ..Default::default()
    }
}

pub(super) fn live_command(
    path: &Path,
    session: Option<SessionId>,
    command: LiveCommand,
) -> Result<Value> {
    let session = session.context("live commands require --session <AGENT_SESSION_ID>")?;
    let binding = discover(path)?;
    match command {
        LiveCommand::Subscribe { after, count } => subscribe(&binding, after, count),
        LiveCommand::Status => request(
            &binding,
            path,
            session,
            LumenLiveAction::State,
            InteractionPolicy::Immediate,
        ),
        LiveCommand::Apply { json, interaction } => {
            let expectation = expectation(path, session, &binding)?;
            let commands = parse_commands(&json, expectation.photo_id)?;
            let action = LumenLiveAction::ExecuteBatch {
                expectation,
                command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
                commands,
            };
            request(&binding, path, session, action, interaction)
        }
        LiveCommand::Undo => {
            let action = LumenLiveAction::Undo {
                expectation: expectation(path, session, &binding)?,
            };
            request(
                &binding,
                path,
                session,
                action,
                InteractionPolicy::Immediate,
            )
        }
        LiveCommand::Redo => {
            let action = LumenLiveAction::Redo {
                expectation: expectation(path, session, &binding)?,
            };
            request(
                &binding,
                path,
                session,
                action,
                InteractionPolicy::Immediate,
            )
        }
        LiveCommand::MoveAgentCursor { revision } => {
            let action = LumenLiveAction::MoveAgentCursor {
                expectation: expectation(path, session, &binding)?,
                target: revision,
            };
            request(
                &binding,
                path,
                session,
                action,
                InteractionPolicy::Immediate,
            )
        }
    }
}

pub(super) struct PreparedLiveSemantic {
    pub(super) project: Project,
    pub(super) photo_id: u64,
    session: SessionId,
    binding: DiscoveredBinding,
    expectation: LumenLiveActionExpectation,
    expected_cursors: Vec<ExpectedCursor>,
    actor_label: String,
}

pub(super) fn prepare_live_semantic(
    path: &Path,
    session: Option<SessionId>,
) -> Result<PreparedLiveSemantic> {
    let session = session.context("live commands require --session <AGENT_SESSION_ID>")?;
    let binding = discover(path)?;
    let collaboration = Workspace::collaboration(path, session)?;
    let agent = Workspace::open_session(path, session)?;
    let agent_state = agent
        .live_state_for_track(collaboration.track_id)?
        .context("agent session is not durable")?;
    let snapshot = current_snapshot(&binding)?;
    let source = Workspace::open_session(path, collaboration.source_session)?;
    let source_state = source
        .live_state_for_track(collaboration.track_id)?
        .context("collaboration source session is not durable")?;
    let expectation = LumenLiveActionExpectation {
        photo_id: agent_state.photo_id,
        track_id: collaboration.track_id,
        agent_revision: agent_state.photo_cursor,
        source_revision: (collaboration.mode == CollaborationMode::Together)
            .then_some(source_state.photo_cursor),
    };
    Ok(PreparedLiveSemantic {
        project: agent.project,
        photo_id: agent_state.photo_id,
        session,
        binding,
        expectation,
        expected_cursors: snapshot.cursors,
        actor_label: agent_state.actor.display_name,
    })
}

pub(super) fn live_execute_prepared(
    prepared: PreparedLiveSemantic,
    commands: Vec<Command>,
) -> Result<Value> {
    let action = match commands.as_slice() {
        [Command::Undo] => LumenLiveAction::Undo {
            expectation: prepared.expectation.clone(),
        },
        [Command::Redo] => LumenLiveAction::Redo {
            expectation: prepared.expectation.clone(),
        },
        _ if commands
            .iter()
            .any(|command| matches!(command, Command::Undo | Command::Redo)) =>
        {
            bail!("undo or redo must be the only command in a live semantic batch")
        }
        _ => LumenLiveAction::ExecuteBatch {
            expectation: prepared.expectation,
            command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
            commands,
        },
    };
    send_request(
        &prepared.binding,
        prepared.session,
        action,
        InteractionPolicy::Immediate,
        prepared.actor_label,
        prepared.expected_cursors,
    )
}

struct DiscoveredBinding {
    directory: DiscoveryDirectory,
    record: DiscoveryRecord,
}

fn discover(path: &Path) -> Result<DiscoveredBinding> {
    let directory = DiscoveryDirectory::open(lumen_live_discovery_root()?)?;
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("could not resolve live project {}", path.display()))?;
    let selected = std::env::var("LUMEN_LIVE_BINDING_ID")
        .or_else(|_| std::env::var("SPECTRUM_LIVE_BINDING_ID"))
        .ok()
        .map(|value| BindingId::from_str(&value))
        .transpose()
        .context("invalid live binding id")?;
    let matches = directory
        .records()?
        .into_iter()
        .filter(|record| {
            record.application == LUMEN_LIVE_APPLICATION
                && record.canonical_project_path == canonical
                && selected.is_none_or(|binding| binding == record.binding_id)
        })
        .collect::<Vec<_>>();
    let record = match matches.as_slice() {
        [record] => record.clone(),
        [] => bail!(
            "no authenticated live Lumen binding exists for {}",
            canonical.display()
        ),
        _ => bail!(
            "multiple live Lumen bindings match {}; set LUMEN_LIVE_BINDING_ID",
            canonical.display()
        ),
    };
    Ok(DiscoveredBinding { directory, record })
}

fn connect(binding: &DiscoveredBinding) -> Result<BridgeClient> {
    let capability = binding.directory.load_capability(&binding.record)?;
    BridgeClient::connect(
        &ClientConfig::local(binding.record.endpoint.clone()),
        &capability,
    )
    .map_err(Into::into)
}

fn current_snapshot(binding: &DiscoveredBinding) -> Result<spectrum_live_bridge::StateSnapshot> {
    let mut client = connect(binding)?;
    client
        .subscribe(binding.record.newest_event_seq)
        .map_err(Into::into)
}

fn expectation(
    path: &Path,
    session: SessionId,
    binding: &DiscoveredBinding,
) -> Result<LumenLiveActionExpectation> {
    let collaboration = Workspace::collaboration(path, session)?;
    let agent = Workspace::open_session(path, session)?
        .live_state_for_track(collaboration.track_id)?
        .context("agent session is not durable")?;
    let source = Workspace::open_session(path, collaboration.source_session)?
        .live_state_for_track(collaboration.track_id)?
        .context("collaboration source session is not durable")?;
    let snapshot = current_snapshot(binding)?;
    ensure_catalog_snapshot_matches(&snapshot, &agent)?;
    Ok(LumenLiveActionExpectation {
        photo_id: agent.photo_id,
        track_id: collaboration.track_id,
        agent_revision: agent.photo_cursor,
        source_revision: (collaboration.mode == CollaborationMode::Together)
            .then_some(source.photo_cursor),
    })
}

fn request(
    binding: &DiscoveredBinding,
    path: &Path,
    session: SessionId,
    action: LumenLiveAction,
    interaction: impl Into<InteractionPolicy>,
) -> Result<Value> {
    let collaboration = Workspace::collaboration(path, session)?;
    let agent = Workspace::open_session(path, session)?
        .live_state_for_track(collaboration.track_id)?
        .context("agent session is not durable")?;
    let snapshot = current_snapshot(binding)?;
    send_request(
        binding,
        session,
        action,
        interaction.into(),
        agent.actor.display_name,
        snapshot.cursors,
    )
}

fn send_request(
    binding: &DiscoveredBinding,
    session: SessionId,
    action: LumenLiveAction,
    interaction: InteractionPolicy,
    actor_label: String,
    expected_cursors: Vec<ExpectedCursor>,
) -> Result<Value> {
    let mut client = connect(binding)?;
    let response = client.request(RequestEnvelope {
        protocol: PROTOCOL_FAMILY.into(),
        version: PROTOCOL_VERSION,
        request_id: RequestId::new(),
        binding_id: binding.record.binding_id,
        binding_epoch: binding.record.binding_epoch,
        project_id: binding.record.project_id,
        application: LUMEN_LIVE_APPLICATION.into(),
        session_id: session,
        expected_cursors,
        actor_label,
        interaction,
        action: ActionEnvelope {
            family: LUMEN_LIVE_ACTION_FAMILY.into(),
            version: LUMEN_LIVE_ACTION_VERSION,
            capabilities: Vec::new(),
            action: serde_json::to_value(action)?,
        },
    })?;
    match response.body {
        ResponseBody::Applied { result, cursors } => {
            Ok(json!({"ok": true, "result": result, "cursors": cursors}))
        }
        ResponseBody::Deferred => Ok(json!({
            "ok": true,
            "deferred": true,
            "request_id": response.request_id,
        })),
        ResponseBody::Refused { reason } => bail!("live action refused: {reason}"),
        ResponseBody::Conflict { current } => {
            bail!(
                "live cursor conflict; current cursors: {}",
                serde_json::to_string(&current)?
            )
        }
        ResponseBody::OutcomeUnknown => {
            bail!("live action outcome is unknown; inspect state and event history before retrying")
        }
        ResponseBody::Error { code, message } => bail!("live bridge error {code}: {message}"),
    }
}

fn subscribe(binding: &DiscoveredBinding, after: u64, count: usize) -> Result<Value> {
    if count == 0 || count > spectrum_live_bridge::MAX_SUBSCRIBER_EVENTS {
        bail!(
            "--count must be between 1 and {}",
            spectrum_live_bridge::MAX_SUBSCRIBER_EVENTS
        );
    }
    let mut client = connect(binding)?;
    let snapshot = client.subscribe(after)?;
    let mut events = Vec::with_capacity(count);
    while events.len() < count {
        match client.read_subscription_message()? {
            ServerMessage::Event(event) => events.push(event),
            ServerMessage::ResyncRequired {
                oldest_seq,
                newest_seq,
            } => {
                return Ok(json!({
                    "ok": false,
                    "resync_required": {
                        "oldest_seq": oldest_seq,
                        "newest_seq": newest_seq,
                    },
                    "snapshot": snapshot,
                }));
            }
            message => bail!("unexpected live subscription message: {message:?}"),
        }
    }
    Ok(json!({"ok": true, "snapshot": snapshot, "events": events}))
}

fn ensure_catalog_snapshot_matches(
    snapshot: &spectrum_live_bridge::StateSnapshot,
    state: &lumen_core::LiveWorkspaceState,
) -> Result<()> {
    let cursor = snapshot
        .cursors
        .iter()
        .find(|cursor| cursor.track_id == state.catalog_track_id)
        .context("live snapshot does not include the Lumen catalog track")?;
    if cursor.revision_id != state.catalog_cursor {
        bail!("live binding catalog cursor does not match this collaboration session");
    }
    Ok(())
}

pub(super) fn parse_commands(value: &str, photo_id: u64) -> Result<Vec<Command>> {
    let commands = decode_commands(value)?;
    let action = LumenLiveAction::ExecuteBatch {
        expectation: LumenLiveActionExpectation {
            photo_id,
            track_id: spectrum_revisions::TrackId::new(),
            agent_revision: spectrum_revisions::RevisionId::new(),
            source_revision: None,
        },
        command_version: LUMEN_COMMAND_OPERATIONS_VERSION,
        commands,
    };
    action.validate()?;
    let LumenLiveAction::ExecuteBatch { commands, .. } = action else {
        unreachable!()
    };
    Ok(commands)
}

pub(super) fn decode_commands(value: &str) -> Result<Vec<Command>> {
    if value.trim_start().starts_with('[') {
        Ok(serde_json::from_str(value)?)
    } else {
        Ok(vec![serde_json::from_str(value)?])
    }
}

#[cfg(test)]
mod tests {
    use crate::run;
    use image::{Rgba, RgbaImage};
    use spectrum_revisions::{Actor, ActorKind};

    use super::*;

    #[test]
    fn live_apply_parser_preserves_one_object_or_one_atomic_array() {
        assert_eq!(
            parse_commands(r#"{"command":"rotate","id":7,"clockwise":true}"#, 7)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            parse_commands(
                r#"[{"command":"flip-horizontal","id":7},{"command":"flip-vertical","id":7}]"#,
                7
            )
            .unwrap()
            .len(),
            2
        );
        assert!(parse_commands(r#"[{"command":"undo"}]"#, 7).is_err());
    }

    #[test]
    fn explicit_live_mode_overrides_environment_free_default() {
        assert_eq!(
            resolved_live_mode(Some(CliLiveMode::Required)).unwrap(),
            CliLiveMode::Required
        );
    }

    #[test]
    fn required_live_without_an_open_binding_never_falls_back_to_direct_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.png");
        RgbaImage::from_pixel(2, 2, Rgba([20, 40, 60, 255]))
            .save(&source)
            .unwrap();
        let mut project = Project::new("No fallback");
        project.import(&[source]).unwrap();
        let path = directory.path().join("no-fallback.lumen");
        let human_session = SessionId::new();
        let human = Workspace::create_durable(
            project,
            &path,
            Actor {
                id: "person:no-fallback".into(),
                display_name: "No Fallback User".into(),
                kind: ActorKind::Human,
            },
            human_session,
        )
        .unwrap();
        let photo_id = human.project.photos[0].id;
        let collaboration = Workspace::start_collaboration(
            &path,
            Some(human_session),
            photo_id,
            Actor {
                id: "agent:no-fallback".into(),
                display_name: "No Fallback Agent".into(),
                kind: ActorKind::Agent,
            },
            CollaborationMode::Together,
        )
        .unwrap();
        let before = Workspace::open_session(&path, collaboration.agent_session)
            .unwrap()
            .live_state_for_track(collaboration.track_id)
            .unwrap()
            .unwrap()
            .photo_cursor;
        let error = run(Cli {
            catalog: path.clone(),
            session: Some(collaboration.agent_session),
            live: Some(CliLiveMode::Required),
            command: CliCommand::Run {
                json: format!(r#"{{"command":"flip-horizontal","id":{photo_id}}}"#),
            },
        })
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no authenticated live Lumen binding")
        );
        let after = Workspace::open_session(&path, collaboration.agent_session)
            .unwrap()
            .live_state_for_track(collaboration.track_id)
            .unwrap()
            .unwrap()
            .photo_cursor;
        assert_eq!(after, before);
    }
}
