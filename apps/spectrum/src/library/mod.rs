//! Spectrum orchestration: editor engines adapt their content to the neutral library.
use anyhow::{Context, Result, bail};
use lumen_core::{DurableCatalog, Project};
use prism_core::{Document, LayerKind};
use sha2::{Digest, Sha256};
use spectrum_library::{Asset, AssetId, ImportBatch, Library, ProjectId};
use spectrum_revisions::{Actor, ActorKind, SessionId};
use std::path::{Path, PathBuf};

pub use spectrum_library::default_root;
pub mod live;
pub mod thumbnail;
pub mod trash;
pub fn actor() -> Actor {
    Actor {
        id: "spectrum:library".into(),
        display_name: "Spectrum".into(),
        kind: ActorKind::Agent,
    }
}
type PreviewCache = std::collections::HashMap<AssetId, ((std::time::SystemTime, u64), PathBuf)>;
pub struct Service {
    previews: std::cell::RefCell<PreviewCache>,
    pub library: Library,
    indexed: std::collections::HashMap<PathBuf, (std::time::SystemTime, u64)>,
}
impl Service {
    pub fn open(root: &Path) -> Result<Self> {
        let library = Library::open(root)?;
        std::fs::create_dir_all(library.root().join("canvases"))?;
        std::fs::create_dir_all(library.root().join("images"))?;
        std::fs::create_dir_all(library.root().join("previews"))?;
        Ok(Self {
            library,
            indexed: Default::default(),
            previews: Default::default(),
        })
    }
    pub fn catalog(&self) -> PathBuf {
        self.library.root().join("images/library.spectrum")
    }
    pub fn ensure_catalog(&self) -> Result<PathBuf> {
        let path = self.catalog();
        if !path.exists() {
            lumen_core::Workspace::create_durable(
                Project::new("Images"),
                &path,
                actor(),
                SessionId::new(),
            )?;
        }
        Ok(path)
    }
    pub fn index_catalog(&self, path: &Path) -> Result<Vec<Asset>> {
        DurableCatalog::library_entries(path)?
            .into_iter()
            .map(|(id, name)| self.library.register("image", &name, path, Some(id)))
            .collect()
    }
    /// Open editor summaries avoid rereading embedded raster bytes during interaction.
    pub fn index_canvas(
        &mut self,
        path: &Path,
        name: &str,
        links: &[(String, AssetId)],
    ) -> Result<()> {
        let asset = self.library.register("canvas", name, path, None)?;
        self.library.references(asset.id, links)?;
        self.indexed.insert(path.to_path_buf(), file_stamp(path)?);
        Ok(())
    }
    pub fn scan(&mut self) -> Result<Vec<Asset>> {
        for entry in std::fs::read_dir(self.library.root().join("images"))? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "spectrum") {
                let stamp = file_stamp(&path)?;
                if self.indexed.get(&path) != Some(&stamp) {
                    self.index_catalog(&path)?;
                    self.indexed.insert(path, stamp);
                }
            }
        }
        for entry in std::fs::read_dir(self.library.root().join("canvases"))? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "spectrum") {
                let stamp = file_stamp(&path)?;
                if self.indexed.get(&path) == Some(&stamp) {
                    continue;
                }
                let doc = prism_core::Workspace::load_read_only(&path)?;
                let asset = self.library.register("canvas", &doc.name, &path, None)?;
                let links = doc
                    .layers
                    .iter()
                    .filter_map(|l| l.image_asset.map(|id| (l.id.to_string(), id)))
                    .collect::<Vec<_>>();
                self.library.references(asset.id, &links)?;
                self.indexed.insert(path, stamp);
            }
        }
        self.library.list()
    }
    pub fn import(&mut self, paths: Vec<PathBuf>) -> Result<Vec<Asset>> {
        Ok(self.import_into(paths, None)?.1)
    }
    /// Imports files as one batch and optionally adds them to a project.
    pub fn import_into(
        &mut self,
        paths: Vec<PathBuf>,
        project: Option<ProjectId>,
    ) -> Result<(ImportBatch, Vec<Asset>)> {
        if let Some(project) = project {
            self.library.project(project)?;
        }
        let assets = self.import_documents(paths)?;
        let ids = assets.iter().map(|a| a.id).collect::<Vec<_>>();
        let batch = self.library.record_import(&ids)?;
        if let Some(project) = project {
            self.library.add_to_project(project, &ids)?;
        }
        Ok((batch, assets))
    }
    fn import_documents(&self, paths: Vec<PathBuf>) -> Result<Vec<Asset>> {
        let path = self
            .library
            .root()
            .join("images")
            .join(format!("{}.spectrum", AssetId::new_v4()));
        let mut workspace = lumen_core::Workspace::create_durable(
            Project::new("Imported images"),
            &path,
            actor(),
            SessionId::new(),
        )?;
        let ids = workspace
            .execute(lumen_core::Command::Import { paths })?
            .photo_ids;
        self.index_catalog(&path).map(|a| {
            a.into_iter()
                .filter(|a| a.item.is_some_and(|i| ids.contains(&i)))
                .collect()
        })
    }
    /// Renames an image or canvas in its engine document and the index, so a
    /// later scan keeps the new name.
    pub fn rename(&mut self, id: AssetId, name: &str) -> Result<Asset> {
        let name = name.trim();
        if name.is_empty() {
            bail!("asset name cannot be empty");
        }
        let asset = self.library.get(id)?;
        let path = self.library.path(&asset)?;
        match asset.kind.as_str() {
            "image" => {
                let item = asset.item.context("image missing item")?;
                live::image(
                    &path,
                    item,
                    lumen_core::Command::RenamePhoto {
                        id: item,
                        name: name.into(),
                    },
                )?;
            }
            "canvas" => {
                live::canvas(
                    &path,
                    vec![prism_core::Command::RenameDocument { name: name.into() }],
                )?;
            }
            kind => bail!("renaming is not implemented for {kind}"),
        }
        self.library.register(&asset.kind, name, &path, asset.item)
    }
    /// Applies an adjustment patch to an image, through the desktop host when
    /// it has the document open.
    pub fn adjust(&self, id: AssetId, patch: lumen_core::AdjustmentPatch) -> Result<()> {
        let asset = self.library.get(id)?;
        if asset.kind != "image" {
            bail!("expected image asset");
        }
        let item = asset.item.context("image missing item")?;
        live::image(
            &self.library.path(&asset)?,
            item,
            lumen_core::Command::Adjust { id: item, patch },
        )?;
        Ok(())
    }
    /// Replaces an image's whole adjustment set, including curves, HSL, and
    /// color grading, which patches do not cover.
    pub fn set_adjustments(&self, id: AssetId, adjustments: lumen_core::Adjustments) -> Result<()> {
        let asset = self.library.get(id)?;
        if asset.kind != "image" {
            bail!("expected image asset");
        }
        let item = asset.item.context("image missing item")?;
        live::image(
            &self.library.path(&asset)?,
            item,
            lumen_core::Command::SetAdjustments {
                id: item,
                adjustments,
            },
        )?;
        Ok(())
    }
    /// Gives each target image all of the source image's edits, crop included.
    pub fn apply_edits(&self, from: AssetId, to: &[AssetId]) -> Result<()> {
        let adjustments = self.image(from)?.adjustments;
        for id in to.iter().filter(|id| **id != from) {
            self.set_adjustments(*id, adjustments.clone())?;
        }
        Ok(())
    }
    /// Steps an image's edit history back or forward.
    pub fn step_history(&self, id: AssetId, forward: bool) -> Result<()> {
        let asset = self.library.get(id)?;
        let item = asset.item.context("image missing item")?;
        let command = if forward {
            lumen_core::Command::Redo
        } else {
            lumen_core::Command::Undo
        };
        live::image(&self.library.path(&asset)?, item, command)?;
        Ok(())
    }
    /// Exports an image or canvas at full size to a path outside the library.
    /// The extension picks the format: .jpg, .jpeg, or .png.
    pub fn export(&self, id: AssetId, destination: &Path) -> Result<()> {
        self.check_export(destination)?;
        let asset = self.library.get(id)?;
        match asset.kind.as_str() {
            "image" => lumen_core::engine::export_photo(
                &self.image(id)?,
                destination,
                spectrum_imaging::RenderOptions { max_size: None },
                92,
            ),
            "canvas" => {
                let doc = prism_core::Workspace::load_read_only(&self.library.path(&asset)?)?;
                self.export_canvas(&doc, destination)
            }
            kind => bail!("exporting is not implemented for {kind}"),
        }
    }
    pub fn create_canvas(&self, name: String, width: u32, height: u32) -> Result<Asset> {
        let path = self
            .library
            .root()
            .join("canvases")
            .join(format!("{}.spectrum", AssetId::new_v4()));
        let doc = Document::new(&name, width, height);
        prism_core::Workspace::create_durable(doc, &path, actor(), SessionId::new())?;
        self.library.register("canvas", &name, &path, None)
    }
    pub fn image(&self, id: AssetId) -> Result<lumen_core::Photo> {
        let asset = self.library.get(id)?;
        if asset.kind != "image" {
            bail!("expected image asset");
        }
        DurableCatalog::load_photo_current(
            &self.library.path(&asset)?,
            asset.item.context("image missing item")?,
        )
    }
    /// Immutable render key; full resolution is retained for export and canvas zoom.
    /// Trashed and purged images resolve to a same-sized placeholder.
    pub fn preview(&self, id: AssetId) -> Result<PathBuf> {
        if let Some(removed) = self.library.removed(id)? {
            return self.placeholder(removed.size);
        }
        let asset = self.library.get(id)?;
        let stamp = file_stamp(&self.library.path(&asset)?)?;
        if let Some((cached_stamp, path)) = self.previews.borrow().get(&id)
            && *cached_stamp == stamp
            && path.exists()
        {
            return Ok(path.clone());
        }
        let photo = self.image(id)?;
        let key = Sha256::digest(serde_json::to_vec(&(&photo.path, &photo.adjustments))?)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let path = self
            .library
            .root()
            .join("previews")
            .join(format!("{id}-{key}.png"));
        if !path.exists() {
            let temporary = path.with_file_name(format!("{}.png", AssetId::new_v4()));
            let result = (|| -> Result<()> {
                lumen_core::engine::render_photo(&photo, Default::default())?.save(&temporary)?;
                std::fs::rename(&temporary, &path)?;
                Ok(())
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(temporary);
            }
            result?;
        }
        self.previews.borrow_mut().insert(id, (stamp, path.clone()));
        Ok(path)
    }
    pub fn resolve(&self, doc: &mut Document) -> Result<()> {
        for layer in &mut doc.layers {
            if let Some(id) = layer.image_asset {
                let LayerKind::Raster {
                    path,
                    original_path,
                } = &mut layer.kind
                else {
                    bail!("image reference on non-raster layer");
                };
                *path = self.preview(id)?;
                *original_path = None;
            }
        }
        Ok(())
    }
    pub fn place(&mut self, canvas: AssetId, image: AssetId) -> Result<u64> {
        let asset = self.library.get(canvas)?;
        if asset.kind != "canvas" {
            bail!("expected canvas asset");
        }
        let source = self.library.get(image)?;
        let path = self.preview(image)?;
        let result = live::canvas(
            &self.library.path(&asset)?,
            vec![prism_core::Command::AddLinkedImage {
                path,
                name: source.name,
                asset: image,
            }],
        )?;
        self.scan()?;
        let outputs = result
            .as_array()
            .or_else(|| result.get("outputs").and_then(|v| v.as_array()))
            .context("missing placed layer result")?;
        outputs
            .first()
            .and_then(|v| v.get("layer_ids"))
            .and_then(|v| v.get(0))
            .and_then(|v| v.as_u64())
            .context("missing placed layer id")
    }

    pub fn copy(&mut self, id: AssetId) -> Result<Asset> {
        let asset = self.library.get(id)?;
        match asset.kind.as_str() {
            "image" => {
                let mut photo = self.image(id)?;
                photo.id = 1;
                photo.name = format!("{} copy", photo.name);
                let mut project = Project::new(&photo.name);
                project.photos = vec![photo];
                project.next_id = 2;
                project.selected = Some(1);
                let path = self
                    .library
                    .root()
                    .join("images")
                    .join(format!("{}.spectrum", AssetId::new_v4()));
                lumen_core::Workspace::create_durable(project, &path, actor(), SessionId::new())?;
                self.index_catalog(&path)?
                    .pop()
                    .context("copy was not indexed")
            }
            "canvas" => {
                let mut doc = prism_core::Workspace::load_read_only(&self.library.path(&asset)?)?;
                let mut copied = std::collections::HashMap::new();
                for layer in &mut doc.layers {
                    if let Some(source) = layer.image_asset
                        && self.library.removed(source)?.is_none()
                    {
                        let id = if let Some(id) = copied.get(&source) {
                            *id
                        } else {
                            let id = self.copy(source)?.id;
                            copied.insert(source, id);
                            id
                        };
                        layer.image_asset = Some(id);
                    }
                }
                doc.name = format!("{} copy", doc.name);
                self.resolve(&mut doc)?;
                let path = self
                    .library
                    .root()
                    .join("canvases")
                    .join(format!("{}.spectrum", AssetId::new_v4()));
                prism_core::Workspace::create_durable(
                    doc.clone(),
                    &path,
                    actor(),
                    SessionId::new(),
                )?;
                let asset = self.library.register("canvas", &doc.name, &path, None)?;
                self.scan()?;
                Ok(asset)
            }
            _ => bail!("copy is not implemented for {}", asset.kind),
        }
    }
}

fn file_stamp(path: &Path) -> Result<(std::time::SystemTime, u64)> {
    let m = std::fs::metadata(path)?;
    Ok((m.modified()?, m.len()))
}

pub fn export_canvas(document: &Document, path: &Path) -> Result<()> {
    let service = Service::open(&default_root()?)?;
    service.export_canvas(document, path)
}
impl Service {
    pub fn export_canvas(&self, document: &Document, path: &Path) -> Result<()> {
        self.check_export(path)?;
        let mut document = document.clone();
        self.resolve(&mut document)?;
        prism_core::export_document(&document, path, 92)
    }
    pub fn check_export(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let resolved = if path.exists() {
            std::fs::canonicalize(path)?
        } else {
            std::fs::canonicalize(parent)?
                .join(path.file_name().context("missing export filename")?)
        };
        if resolved.starts_with(self.library.root()) {
            bail!("exports must be outside Spectrum's managed library");
        }
        Ok(())
    }
}
