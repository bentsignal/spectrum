//! Route edits to the authenticated desktop host when it owns the document.
use super::actor;
use anyhow::{Context, Result, bail};
use spectrum_live_bridge::*;
use spectrum_revisions::{CollaborationMode, SessionId};
use std::path::Path;

fn discover(
    path: &Path,
    application: &str,
) -> Result<Option<(DiscoveryDirectory, DiscoveryRecord)>> {
    let directory = DiscoveryDirectory::open(lumen_core::lumen_live_discovery_root()?)?;
    let path = std::fs::canonicalize(path)?;
    let matches = directory
        .records()?
        .into_iter()
        .filter(|r| r.application == application && r.canonical_project_path == path)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [r] => Ok(Some((directory, r.clone()))),
        _ => bail!("multiple desktop bindings own this document"),
    }
}
fn connect(directory: &DiscoveryDirectory, record: &DiscoveryRecord) -> Result<BridgeClient> {
    Ok(BridgeClient::connect(
        &ClientConfig::local(record.endpoint.clone()),
        &directory.load_capability(record)?,
    )?)
}
fn send(
    directory: &DiscoveryDirectory,
    record: &DiscoveryRecord,
    session: SessionId,
    family: &str,
    version: u32,
    action: serde_json::Value,
    cursors: Vec<ExpectedCursor>,
) -> Result<serde_json::Value> {
    let response = connect(directory, record)?.request(RequestEnvelope {
        protocol: PROTOCOL_FAMILY.into(),
        version: PROTOCOL_VERSION,
        request_id: RequestId::new(),
        binding_id: record.binding_id,
        binding_epoch: record.binding_epoch,
        project_id: record.project_id,
        application: record.application.clone(),
        session_id: session,
        expected_cursors: cursors,
        actor_label: "Spectrum CLI".into(),
        interaction: InteractionPolicy::Immediate,
        action: ActionEnvelope {
            family: family.into(),
            version,
            capabilities: vec![],
            action,
        },
    })?;
    match response.body {
        ResponseBody::Applied { result, .. } => Ok(result),
        other => bail!("desktop edit did not complete: {other:?}"),
    }
}
fn human_session() -> Result<SessionId> {
    Ok(spectrum_revisions::local_session_id(
        &eframe::storage_dir("Spectrum").context("missing Spectrum data directory")?,
    )?)
}
pub fn image(path: &Path, id: u64, command: lumen_core::Command) -> Result<serde_json::Value> {
    let Some((directory, record)) = discover(path, lumen_core::LUMEN_LIVE_APPLICATION)? else {
        let mut workspace = lumen_core::Workspace::open_as(path, actor(), SessionId::new())?;
        workspace.execute(lumen_core::Command::Select { id })?;
        return Ok(serde_json::to_value(workspace.execute(command)?)?);
    };
    let collaboration = lumen_core::Workspace::start_collaboration(
        path,
        Some(human_session()?),
        id,
        actor(),
        CollaborationMode::Together,
    )?;
    let agent = lumen_core::Workspace::open_session(path, collaboration.agent_session)?
        .live_state_for_track(collaboration.track_id)?
        .context("missing image state")?;
    let snapshot = connect(&directory, &record)?.subscribe(record.newest_event_seq)?;
    let source = lumen_core::Workspace::open_session(path, collaboration.source_session)?
        .live_state_for_track(collaboration.track_id)?
        .context("missing source image state")?
        .photo_cursor;
    let expectation = lumen_core::LumenLiveActionExpectation {
        photo_id: id,
        track_id: collaboration.track_id,
        agent_revision: agent.photo_cursor,
        source_revision: Some(source),
    };
    let action = match command {
        lumen_core::Command::Undo => lumen_core::LumenLiveAction::Undo { expectation },
        lumen_core::Command::Redo => lumen_core::LumenLiveAction::Redo { expectation },
        command => lumen_core::LumenLiveAction::ExecuteBatch {
            expectation,
            command_version: lumen_core::LUMEN_COMMAND_OPERATIONS_VERSION,
            commands: vec![command],
        },
    };
    send(
        &directory,
        &record,
        collaboration.agent_session,
        lumen_core::LUMEN_LIVE_ACTION_FAMILY,
        lumen_core::LUMEN_LIVE_ACTION_VERSION,
        serde_json::to_value(action)?,
        snapshot.cursors,
    )
}
pub fn canvas(path: &Path, commands: Vec<prism_core::Command>) -> Result<serde_json::Value> {
    let Some((directory, record)) = discover(path, prism_core::PRISM_LIVE_APPLICATION)? else {
        let mut workspace = prism_core::Workspace::open_as(path, actor(), SessionId::new())?;
        let output = if commands.len() == 1 {
            vec![workspace.execute(commands.into_iter().next().unwrap())?]
        } else {
            workspace.execute_batch(commands)?
        };
        return Ok(serde_json::to_value(output)?);
    };
    let collaboration = prism_core::Workspace::start_collaboration(
        path,
        Some(human_session()?),
        actor(),
        CollaborationMode::Together,
    )?;
    let agent = prism_core::Workspace::open_session(path, collaboration.agent_session)?
        .live_state()?
        .context("missing canvas state")?;
    let snapshot = connect(&directory, &record)?.subscribe(record.newest_event_seq)?;
    let source = snapshot
        .cursors
        .iter()
        .find(|c| c.track_id == collaboration.track_id)
        .context("missing desktop canvas cursor")?
        .revision_id;
    let expectation = prism_core::PrismLiveActionExpectation {
        agent_revision: agent.cursor,
        source_revision: Some(source),
    };
    let action = match commands.as_slice() {
        [prism_core::Command::Undo] => prism_core::PrismLiveAction::Undo { expectation },
        [prism_core::Command::Redo] => prism_core::PrismLiveAction::Redo { expectation },
        _ => prism_core::PrismLiveAction::ExecuteBatch {
            expectation,
            command_version: prism_core::PRISM_COMMAND_OPERATIONS_VERSION,
            commands,
        },
    };
    send(
        &directory,
        &record,
        collaboration.agent_session,
        prism_core::PRISM_LIVE_ACTION_FAMILY,
        prism_core::PRISM_LIVE_ACTION_VERSION,
        serde_json::to_value(action)?,
        snapshot.cursors,
    )
}
