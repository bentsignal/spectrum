use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use spectrum_revisions::{AssetId, Revision, RevisionId, RevisionStore};

use crate::{Document, LayerKind, RenderRegion, render_document, render_document_region_scaled};

use super::{
    APPLICATION_ID, AssetReference, PrismCompatibility, decode_snapshot, operation_batch,
    readonly_font, replay_commands, validate_asset_identity,
};

pub(super) fn validate_copy_store(store: &RevisionStore, expected_revisions: usize) -> Result<()> {
    let info = store.project_info()?;
    if info.application_id != APPLICATION_ID {
        bail!("published optimized copy has an invalid application identity");
    }
    let revisions = store.revisions_for_track(info.default_track_id)?;
    if revisions.len() != expected_revisions {
        bail!("published optimized copy has an incomplete revision history");
    }
    let ordered = ordered_revisions(&revisions, info.root_revision)?;
    let (root, tail) = ordered
        .split_first()
        .context("published optimized copy has no root revision")?;
    let mut replayed = exact_snapshot_document(store, root.id)?;
    for revision in tail {
        let (_, commands) = operation_batch(store, revision)?;
        replay_commands(store, &mut replayed, commands)?;
        let exact = exact_snapshot_document(store, revision.id)?;
        if replayed != exact {
            bail!(
                "published optimized copy revision {} operations do not reproduce its exact snapshot",
                revision.id
            );
        }
    }
    Ok(())
}

fn ordered_revisions(revisions: &[Revision], root: RevisionId) -> Result<Vec<&Revision>> {
    let by_id = revisions
        .iter()
        .map(|revision| (revision.id, revision))
        .collect::<HashMap<_, _>>();
    let root_revision = by_id
        .get(&root)
        .context("published optimized copy root revision is missing")?;
    if root_revision.parent_id.is_some() {
        bail!("published optimized copy root revision has a parent");
    }
    let mut child_for_parent = HashMap::new();
    for revision in revisions {
        if revision.id == root {
            continue;
        }
        let parent = revision
            .parent_id
            .context("published optimized copy contains a parentless non-root revision")?;
        if !by_id.contains_key(&parent) {
            bail!("published optimized copy revision has an external parent");
        }
        if child_for_parent.insert(parent, revision.id).is_some() {
            bail!("published optimized copy revision history is branched");
        }
    }
    let mut ordered = Vec::with_capacity(revisions.len());
    let mut cursor = root;
    loop {
        ordered.push(*by_id.get(&cursor).unwrap());
        let Some(next) = child_for_parent.get(&cursor) else {
            break;
        };
        cursor = *next;
    }
    if ordered.len() != revisions.len() {
        bail!("published optimized copy revision history is disconnected");
    }
    Ok(ordered)
}

fn exact_snapshot_document(store: &RevisionStore, revision: RevisionId) -> Result<Document> {
    let plan = store.replay_plan(revision, &PrismCompatibility)?;
    if plan.snapshot_revision != revision || !plan.steps.is_empty() {
        bail!("published optimized copy is missing an exact revision snapshot");
    }
    let mut document: Document = serde_json::from_slice(&decode_snapshot(&plan.snapshot)?)
        .context("published optimized copy has an invalid exact revision snapshot")?;
    readonly_font::hydrate_read_only_legacy_font_permissions(store, &mut document)?;
    document.migrate()?;
    Ok(document)
}

pub(super) fn validate_render_parity(
    source_store: &RevisionStore,
    source_document: &Document,
    destination_store: &RevisionStore,
    destination_document: &Document,
    output: &Path,
) -> Result<()> {
    let materialization = RenderMaterialization::new(output)?;
    let source = materialization.materialize(source_store, source_document, "source")?;
    let destination =
        materialization.materialize(destination_store, destination_document, "destination")?;
    let source_full = render_document(&source, None)?.to_rgba8();
    let destination_full = render_document(&destination, None)?.to_rgba8();
    if source_full != destination_full {
        bail!("optimized copy does not preserve exact full-document rendering");
    }
    let region = RenderRegion {
        x: 0,
        y: 0,
        width: source.width.min(256),
        height: source.height.min(256),
    };
    let source_region = render_document_region_scaled(&source, 1.0, region)?.to_rgba8();
    let destination_region = render_document_region_scaled(&destination, 1.0, region)?.to_rgba8();
    if source_region != destination_region {
        bail!("optimized copy does not preserve exact region rendering");
    }
    Ok(())
}

struct RenderMaterialization {
    directory: PathBuf,
}

impl RenderMaterialization {
    fn new(output: &Path) -> Result<Self> {
        let directory = output.with_file_name(format!(
            ".prism-optimized-render-{}",
            spectrum_revisions::SessionId::new()
        ));
        fs::create_dir(&directory)?;
        Ok(Self { directory })
    }

    fn materialize(
        &self,
        store: &RevisionStore,
        document: &Document,
        label: &str,
    ) -> Result<Document> {
        let mut document = document.clone();
        let mut paths = HashMap::new();
        for layer in &mut document.layers {
            if let LayerKind::Raster {
                path,
                original_path,
            } = &mut layer.kind
            {
                *path = self.materialize_asset(store, path, label, &mut paths)?;
                *original_path = None;
            }
        }
        for font in &mut document.font_assets {
            font.path = self.materialize_asset(store, &font.path, label, &mut paths)?;
            font.original_path = None;
        }
        Ok(document)
    }

    fn materialize_asset(
        &self,
        store: &RevisionStore,
        reference_path: &Path,
        label: &str,
        paths: &mut HashMap<AssetId, PathBuf>,
    ) -> Result<PathBuf> {
        let reference =
            AssetReference::parse(reference_path).context("render asset is not embedded")?;
        if let Some(path) = paths.get(&reference.id) {
            return Ok(path.clone());
        }
        let asset = store
            .asset_record(reference.id)?
            .with_context(|| format!("render asset {} is missing", reference.id))?;
        validate_asset_identity(&asset, reference.id, &reference.extension)?;
        let path = self
            .directory
            .join(format!("{label}-{}.{}", reference.id, reference.extension));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(&asset.bytes)?;
        file.sync_all()?;
        paths.insert(reference.id, path.clone());
        Ok(path)
    }
}

impl Drop for RenderMaterialization {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
