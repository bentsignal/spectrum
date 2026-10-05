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
    let directory = DiscoveryDirectory::open(spectrum_image::lumen_live_discovery_root()?)?;
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
        &spectrum_library::data_root().context("missing Spectrum data directory")?,
    )?)
}
pub fn image(path: &Path, id: u64, command: spectrum_image::Command) -> Result<serde_json::Value> {
    let Some((directory, record)) = discover(path, spectrum_image::LUMEN_LIVE_APPLICATION)? else {
        let mut workspace = spectrum_image::Workspace::open_as(path, actor(), SessionId::new())?;
        workspace.execute(spectrum_image::Command::Select { id })?;
        let output = workspace.execute(command)?;
        if let Some(error) = workspace.pending_publish_error() {
            bail!("the edit was saved but not published: {error}");
        }
        return Ok(serde_json::to_value(output)?);
    };
    let collaboration = spectrum_image::Workspace::start_collaboration(
        path,
        Some(human_session()?),
        id,
        actor(),
        CollaborationMode::Together,
    )?;
    let agent = spectrum_image::Workspace::open_session(path, collaboration.agent_session)?
        .live_state_for_track(collaboration.track_id)?
        .context("missing image state")?;
    let snapshot = connect(&directory, &record)?.subscribe(record.newest_event_seq)?;
    let source = spectrum_image::Workspace::open_session(path, collaboration.source_session)?
        .live_state_for_track(collaboration.track_id)?
        .context("missing source image state")?
        .photo_cursor;
    let expectation = spectrum_image::LumenLiveActionExpectation {
        photo_id: id,
        track_id: collaboration.track_id,
        agent_revision: agent.photo_cursor,
        source_revision: Some(source),
    };
    let action = match command {
        spectrum_image::Command::Undo => spectrum_image::LumenLiveAction::Undo { expectation },
        spectrum_image::Command::Redo => spectrum_image::LumenLiveAction::Redo { expectation },
        command => spectrum_image::LumenLiveAction::ExecuteBatch {
            expectation,
            command_version: spectrum_image::LUMEN_COMMAND_OPERATIONS_VERSION,
            commands: vec![command],
        },
    };
    send(
        &directory,
        &record,
        collaboration.agent_session,
        spectrum_image::LUMEN_LIVE_ACTION_FAMILY,
        spectrum_image::LUMEN_LIVE_ACTION_VERSION,
        serde_json::to_value(action)?,
        snapshot.cursors,
    )
}
pub fn canvas(path: &Path, commands: Vec<spectrum_canvas::Command>) -> Result<serde_json::Value> {
    let Some((directory, record)) = discover(path, spectrum_canvas::PRISM_LIVE_APPLICATION)? else {
        let mut workspace = spectrum_canvas::Workspace::open_as(path, actor(), SessionId::new())?;
        let output = if commands.len() == 1 {
            vec![workspace.execute(commands.into_iter().next().unwrap())?]
        } else {
            workspace.execute_batch(commands)?
        };
        if let Some(error) = workspace.pending_publish_error() {
            bail!("the edit was saved but not published: {error}");
        }
        return Ok(serde_json::to_value(output)?);
    };
    let collaboration = spectrum_canvas::Workspace::start_collaboration(
        path,
        Some(human_session()?),
        actor(),
        CollaborationMode::Together,
    )?;
    let agent = spectrum_canvas::Workspace::open_session(path, collaboration.agent_session)?
        .live_state()?
        .context("missing canvas state")?;
    let snapshot = connect(&directory, &record)?.subscribe(record.newest_event_seq)?;
    let source = snapshot
        .cursors
        .iter()
        .find(|c| c.track_id == collaboration.track_id)
        .context("missing desktop canvas cursor")?
        .revision_id;
    let expectation = spectrum_canvas::PrismLiveActionExpectation {
        agent_revision: agent.cursor,
        source_revision: Some(source),
    };
    let action = match commands.as_slice() {
        [spectrum_canvas::Command::Undo] => spectrum_canvas::PrismLiveAction::Undo { expectation },
        [spectrum_canvas::Command::Redo] => spectrum_canvas::PrismLiveAction::Redo { expectation },
        _ => spectrum_canvas::PrismLiveAction::ExecuteBatch {
            expectation,
            command_version: spectrum_canvas::PRISM_COMMAND_OPERATIONS_VERSION,
            commands,
        },
    };
    send(
        &directory,
        &record,
        collaboration.agent_session,
        spectrum_canvas::PRISM_LIVE_ACTION_FAMILY,
        spectrum_canvas::PRISM_LIVE_ACTION_VERSION,
        serde_json::to_value(action)?,
        snapshot.cursors,
    )
}
