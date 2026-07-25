use std::{path::Path, str::FromStr};

use anyhow::{Context, Result, bail};
use clap::{Subcommand, ValueEnum};
use prism_core::{
    Command, PRISM_COMMAND_OPERATIONS_VERSION, PRISM_LIVE_ACTION_FAMILY, PRISM_LIVE_ACTION_VERSION,
    PRISM_LIVE_APPLICATION, PrismLiveAction, PrismLiveActionExpectation, Workspace,
    prism_live_discovery_root,
};
use serde_json::{Value, json};
use spectrum_live_bridge::{
    ActionEnvelope, BindingId, BridgeClient, ClientConfig, DiscoveryDirectory, DiscoveryRecord,
    ExpectedCursor, InteractionPolicy, PROTOCOL_FAMILY, PROTOCOL_VERSION, RequestEnvelope,
    RequestId, ResponseBody, ServerMessage,
};
use spectrum_revisions::{CollaborationMode, SessionId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(super) enum CliLiveMode {
    Off,
    Required,
}

#[derive(Subcommand)]
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
        None => match std::env::var("PRISM_LIVE_MODE")
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
            PrismLiveAction::State,
            InteractionPolicy::Immediate,
        ),
        LiveCommand::Apply { json, interaction } => {
            let commands = parse_commands(&json)?;
            let action = PrismLiveAction::ExecuteBatch {
                expectation: expectation(path, session, &binding)?,
                command_version: PRISM_COMMAND_OPERATIONS_VERSION,
                commands,
            };
            request(&binding, path, session, action, interaction)
        }
        LiveCommand::Undo => {
            let action = PrismLiveAction::Undo {
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
            let action = PrismLiveAction::Redo {
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
            let action = PrismLiveAction::MoveAgentCursor {
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
    pub(super) document: prism_core::Document,
    session: SessionId,
    binding: DiscoveredBinding,
    expectation: PrismLiveActionExpectation,
    expected_cursors: Vec<ExpectedCursor>,
    actor_label: String,
}

pub(super) fn prepare_live_semantic(
    path: &Path,
    session: Option<SessionId>,
) -> Result<PreparedLiveSemantic> {
    let session = session.context("live commands require --session <AGENT_SESSION_ID>")?;
    let binding = discover(path)?;
    let agent = Workspace::open_session(path, session)?;
    let agent_state = agent
        .live_state()?
        .context("agent session is not durable")?;
    let collaboration = Workspace::collaboration(path, session)?;
    let snapshot = current_snapshot(&binding)?;
    let human_cursor = binding_cursor(&snapshot, collaboration.track_id)?;
    let expectation = PrismLiveActionExpectation {
        agent_revision: agent_state.cursor,
        source_revision: (collaboration.mode == CollaborationMode::Together)
            .then_some(human_cursor),
    };
    Ok(PreparedLiveSemantic {
        document: agent.document,
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
        [Command::Undo] => PrismLiveAction::Undo {
            expectation: prepared.expectation.clone(),
        },
        [Command::Redo] => PrismLiveAction::Redo {
            expectation: prepared.expectation.clone(),
        },
        _ if commands
            .iter()
            .any(|command| matches!(command, Command::Undo | Command::Redo)) =>
        {
            bail!("undo or redo must be the only command in a live semantic batch")
        }
        _ => PrismLiveAction::ExecuteBatch {
            expectation: prepared.expectation,
            command_version: PRISM_COMMAND_OPERATIONS_VERSION,
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
    let directory = DiscoveryDirectory::open(prism_live_discovery_root()?)?;
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("could not resolve live project {}", path.display()))?;
    let selected = std::env::var("PRISM_LIVE_BINDING_ID")
        .or_else(|_| std::env::var("SPECTRUM_LIVE_BINDING_ID"))
        .ok()
        .map(|value| BindingId::from_str(&value))
        .transpose()
        .context("invalid live binding id")?;
    let matches = directory
        .records()?
        .into_iter()
        .filter(|record| {
            record.application == PRISM_LIVE_APPLICATION
                && record.canonical_project_path == canonical
                && selected.is_none_or(|binding| binding == record.binding_id)
        })
        .collect::<Vec<_>>();
    let record = match matches.as_slice() {
        [record] => record.clone(),
        [] => bail!(
            "no authenticated live Prism binding exists for {}",
            canonical.display()
        ),
        _ => bail!(
            "multiple live Prism bindings match {}; set PRISM_LIVE_BINDING_ID",
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
) -> Result<PrismLiveActionExpectation> {
    let agent = Workspace::open_session(path, session)?
        .live_state()?
        .context("agent session is not durable")?;
    let collaboration = Workspace::collaboration(path, session)?;
    let snapshot = current_snapshot(binding)?;
    let human_cursor = binding_cursor(&snapshot, collaboration.track_id)?;
    Ok(PrismLiveActionExpectation {
        agent_revision: agent.cursor,
        source_revision: (collaboration.mode == CollaborationMode::Together)
            .then_some(human_cursor),
    })
}

fn request(
    binding: &DiscoveredBinding,
    path: &Path,
    session: SessionId,
    action: PrismLiveAction,
    interaction: impl Into<InteractionPolicy>,
) -> Result<Value> {
    let agent = Workspace::open_session(path, session)?
        .live_state()?
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
    action: PrismLiveAction,
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
        application: PRISM_LIVE_APPLICATION.into(),
        session_id: session,
        expected_cursors,
        actor_label,
        interaction,
        action: ActionEnvelope {
            family: PRISM_LIVE_ACTION_FAMILY.into(),
            version: PRISM_LIVE_ACTION_VERSION,
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

fn binding_cursor(
    snapshot: &spectrum_live_bridge::StateSnapshot,
    track_id: spectrum_revisions::TrackId,
) -> Result<spectrum_revisions::RevisionId> {
    snapshot
        .cursors
        .iter()
        .find(|cursor| cursor.track_id == track_id)
        .map(|cursor| cursor.revision_id)
        .context("live snapshot does not include the Prism document track")
}

pub(super) fn parse_commands(value: &str) -> Result<Vec<Command>> {
    let commands = decode_commands(value)?;
    let action = PrismLiveAction::ExecuteBatch {
        expectation: PrismLiveActionExpectation {
            agent_revision: spectrum_revisions::RevisionId::new(),
            source_revision: None,
        },
        command_version: PRISM_COMMAND_OPERATIONS_VERSION,
        commands,
    };
    action.validate()?;
    let PrismLiveAction::ExecuteBatch { commands, .. } = action else {
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
    use super::*;

    #[test]
    fn live_apply_parser_preserves_one_object_or_one_atomic_array() {
        assert_eq!(
            parse_commands(r#"{"command":"rename_document","name":"One"}"#)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            parse_commands(
                r#"[{"command":"rename_document","name":"One"},{"command":"set_snapping","enabled":true}]"#
            )
            .unwrap()
            .len(),
            2
        );
        assert!(parse_commands(r#"[{"command":"undo"}]"#).is_err());
    }

    #[test]
    fn explicit_live_mode_overrides_environment_free_default() {
        assert_eq!(
            resolved_live_mode(Some(CliLiveMode::Required)).unwrap(),
            CliLiveMode::Required
        );
    }
}
