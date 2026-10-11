//! History commands shared by images and canvases: listing (with search),
//! naming revisions, and previews of any revision.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use spectrum_assets::Service;
use spectrum_document::{History, RevisionId};
use spectrum_library::AssetId;

/// A history as JSON: every revision, every session, and the names. With a
/// search, `matches` lists the revisions whose name, description, or author
/// contains it (ignoring case).
pub(super) fn output(history: &History, search: Option<&str>) -> Value {
    let names: serde_json::Map<String, Value> = history
        .names
        .iter()
        .map(|(revision, name)| (revision.to_string(), json!(name)))
        .collect();
    let mut value = json!({
        "ok": true,
        "root": history.root,
        "current": history.current,
        "revisions": history.revisions,
        "sessions": history.sessions,
        "names": names,
    });
    if let Some(search) = search {
        let wanted = search.to_lowercase();
        let found = |text: &str| text.to_lowercase().contains(&wanted);
        let matches: Vec<RevisionId> = history
            .revisions
            .iter()
            .filter(|revision| {
                history
                    .names
                    .iter()
                    .any(|(id, name)| *id == revision.id && found(name))
                    || revision.label.as_deref().is_some_and(found)
                    || found(&revision.actor.display_name)
            })
            .map(|revision| revision.id)
            .collect();
        value["matches"] = json!(matches);
    }
    value
}

/// `history-name <revision> [name] [--clear]`: names the branch holding
/// the revision.
pub(super) fn name(service: &mut Service, id: AssetId, args: &clap::ArgMatches) -> Result<Value> {
    let revision = *args
        .get_one::<RevisionId>("revision")
        .context("choose a revision from `history`")?;
    let name = args.get_one::<String>("name").map(String::as_str);
    let name = if args.get_flag("clear") { None } else { name };
    if name.is_none() && !args.get_flag("clear") {
        anyhow::bail!("give a name, or --clear to remove one");
    }
    let start = service.name_branch(id, revision, name)?;
    Ok(json!({"ok": true, "action": "history_name", "branch_start": start, "name": name}))
}

/// `history-preview <revision> [--size N]`: renders it once and prints
/// where the preview is.
pub(super) fn preview(service: &Service, id: AssetId, args: &clap::ArgMatches) -> Result<Value> {
    let revision = *args
        .get_one::<RevisionId>("revision")
        .context("choose a revision from `history`")?;
    let size = args.get_one::<u32>("size").copied().unwrap_or(256);
    let path = service.revision_preview(id, revision, size)?;
    Ok(json!({"ok": true, "revision": revision, "path": path}))
}
