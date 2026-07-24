use super::*;

pub(super) fn from_lumen(catalog: &Path, photo_id: u64, output: &Path) -> Result<Value> {
    let project = if LumenDurableCatalog::looks_durable(catalog)? {
        LumenDurableCatalog::load_current(catalog)?
    } else {
        LumenProject::load(catalog)?
    };
    let photo = project.photo(photo_id)?;
    let rendered = render_photo(photo, RenderOptions::default())?;
    let session = SessionId::new();
    let import_directory = std::env::temp_dir().join("spectrum-prism-imports");
    std::fs::create_dir_all(&import_directory)?;
    let asset = import_directory.join(format!("{session}.png"));
    rendered.save(&asset)?;

    let mut workspace = Workspace::new(
        Document::new(
            format!("{} — {}", project.name, photo.name),
            rendered.width(),
            rendered.height(),
        ),
        None,
    );
    workspace.document.background = [0, 0, 0, 0];
    workspace.execute(Command::AddRaster {
        path: asset.clone(),
        name: Some(photo.name.clone()),
        x: 0.0,
        y: 0.0,
    })?;
    let durable = Workspace::create_durable(workspace.document, output, cli_actor(), session);
    let _ = std::fs::remove_file(asset);
    let mut workspace = durable?;
    workspace.save(None)?;
    Ok(json!({
        "ok": true,
        "action": "from_lumen",
        "catalog": catalog,
        "photo_id": photo_id,
        "project": output
    }))
}
