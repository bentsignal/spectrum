//! The library service: what Spectrum does with each kind of asset. Every
//! asset is one durable document in the library; the library index names
//! it, files it in projects, and records which assets use which.
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use spectrum_canvas::{Document, LayerKind};
use spectrum_document::{Actor, ActorKind, SessionId};
use spectrum_image::Image;
use spectrum_library::{Asset, AssetId, AssetKind, ImportBatch, Library, ProjectId};
use std::path::{Path, PathBuf};

pub use spectrum_library::default_root;
mod export;
pub mod thumbnail;
pub mod trash;
pub use export::ExportOptions;

type PreviewCache = std::collections::HashMap<AssetId, ((std::time::SystemTime, u64), PathBuf)>;

/// A library opened by someone: the person at the desktop, or an agent.
/// Their edits land in one lasting session per library, so history shows
/// who made each change.
pub struct Service {
    previews: std::cell::RefCell<PreviewCache>,
    pub library: Library,
    actor: Actor,
    session: SessionId,
}

impl Service {
    /// Opens a library as the person using Spectrum.
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_as(
            root,
            Actor {
                id: "person:local".into(),
                display_name: "You".into(),
                kind: ActorKind::Human,
            },
        )
    }

    /// Opens a library as an agent working through the command line.
    pub fn agent(root: &Path) -> Result<Self> {
        Self::open_as(
            root,
            Actor {
                id: "agent:spectrum-cli".into(),
                display_name: "Spectrum CLI".into(),
                kind: ActorKind::Agent,
            },
        )
    }

    fn open_as(root: &Path, actor: Actor) -> Result<Self> {
        let library = Library::open(root)?;
        for directory in ["canvases", "images", "previews"] {
            std::fs::create_dir_all(library.root().join(directory))?;
        }
        let kind = match actor.kind {
            ActorKind::Human => "person",
            _ => "agent",
        };
        let session =
            spectrum_document::local_session_id(&library.root().join("sessions").join(kind))?;
        Ok(Self {
            library,
            previews: Default::default(),
            actor,
            session,
        })
    }

    /// A new document path in the library's folder for `kind`.
    fn new_document(&self, kind: AssetKind) -> PathBuf {
        let folder = match kind {
            AssetKind::Image => "images",
            AssetKind::Canvas => "canvases",
            other => other.as_str(),
        };
        self.library
            .root()
            .join(folder)
            .join(format!("{}.spectrum", AssetId::new_v4()))
    }

    fn document(&self, id: AssetId, kind: AssetKind) -> Result<PathBuf> {
        let asset = self.library.get(id)?;
        if asset.kind != kind {
            bail!("{} is a {}, not a {kind}", asset.name, asset.kind);
        }
        self.library.path(&asset)
    }

    pub fn import(&mut self, paths: Vec<PathBuf>) -> Result<Vec<Asset>> {
        Ok(self.import_into(paths, None)?.1)
    }

    /// Imports files as one batch, one image asset per file, and optionally
    /// adds them to a project.
    pub fn import_into(
        &mut self,
        paths: Vec<PathBuf>,
        project: Option<ProjectId>,
    ) -> Result<(ImportBatch, Vec<Asset>)> {
        if let Some(project) = project {
            self.library.project(project)?;
        }
        if paths.is_empty() {
            bail!("choose at least one image to import");
        }
        let images = paths
            .iter()
            .map(|path| Image::import(path))
            .collect::<Result<Vec<_>>>()?;
        let mut assets = Vec::with_capacity(images.len());
        for (path, image) in paths.iter().zip(images) {
            let name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Image".into());
            assets.push(self.create_image(&name, image)?);
        }
        let ids = assets.iter().map(|a| a.id).collect::<Vec<_>>();
        let batch = self.library.record_import(&ids)?;
        if let Some(project) = project {
            self.library.add_to_project(project, &ids)?;
        }
        Ok((batch, assets))
    }

    fn create_image(&self, name: &str, image: Image) -> Result<Asset> {
        let path = self.new_document(AssetKind::Image);
        spectrum_image::Workspace::create(&path, image, self.actor.clone(), self.session)?;
        self.library.register(AssetKind::Image, name, &path)
    }

    pub fn create_canvas(&self, name: String, width: u32, height: u32) -> Result<Asset> {
        self.create_canvas_from(Document::new(&name, width, height))
    }

    /// Adds a canvas asset holding `document`, named after it.
    pub fn create_canvas_from(&self, document: Document) -> Result<Asset> {
        let path = self.new_document(AssetKind::Canvas);
        let name = document.name.clone();
        spectrum_canvas::Workspace::create(&path, document, self.actor.clone(), self.session)?;
        self.library.register(AssetKind::Canvas, &name, &path)
    }

    /// Renames an asset.
    pub fn rename(&mut self, id: AssetId, name: &str) -> Result<Asset> {
        self.library.rename(id, name)
    }

    /// An image as last saved.
    pub fn image(&self, id: AssetId) -> Result<Image> {
        spectrum_image::Workspace::read(&self.document(id, AssetKind::Image)?)
    }

    /// A canvas as last saved, with its linked images resolved for drawing.
    pub fn canvas(&self, id: AssetId) -> Result<Document> {
        let mut document = self.saved_canvas(id)?;
        self.resolve(&mut document)?;
        Ok(document)
    }

    /// A canvas as last saved, its linked images as they were stored.
    pub fn saved_canvas(&self, id: AssetId) -> Result<Document> {
        spectrum_canvas::Workspace::read(&self.document(id, AssetKind::Canvas)?)
    }

    /// Edits an image. Each edit is one revision in its history.
    pub fn edit_image(
        &self,
        id: AssetId,
        commands: Vec<spectrum_image::Command>,
    ) -> Result<Vec<spectrum_image::CommandOutput>> {
        let path = self.document(id, AssetKind::Image)?;
        let mut workspace =
            spectrum_image::Workspace::open_newest(&path, self.actor.clone(), self.session)?;
        let outputs = commands
            .into_iter()
            .map(|command| workspace.execute(command))
            .collect::<Result<Vec<_>>>()?;
        if let Some(error) = workspace.pending_publish_error() {
            bail!("the edit was saved but not published: {error}");
        }
        self.previews.borrow_mut().remove(&id);
        Ok(outputs)
    }

    /// Edits a canvas. Consecutive edits apply together as one revision;
    /// undo and redo step on their own.
    pub fn edit_canvas(
        &mut self,
        id: AssetId,
        commands: Vec<spectrum_canvas::Command>,
    ) -> Result<Vec<spectrum_canvas::CommandOutput>> {
        use spectrum_canvas::{CanvasModel, Command};
        use spectrum_document::Model;
        let path = self.document(id, AssetKind::Canvas)?;
        let mut workspace =
            spectrum_canvas::Workspace::open_newest(&path, self.actor.clone(), self.session)?;
        let mut outputs = Vec::with_capacity(commands.len());
        let mut batch: Vec<Command> = Vec::new();
        for command in commands {
            if CanvasModel::step(&command).is_some() || CanvasModel::transient(&command) {
                if !batch.is_empty() {
                    outputs.extend(workspace.execute_batch(std::mem::take(&mut batch))?);
                }
                outputs.push(workspace.execute(command)?);
            } else {
                batch.push(command);
            }
        }
        if !batch.is_empty() {
            outputs.extend(workspace.execute_batch(batch)?);
        }
        if let Some(error) = workspace.pending_publish_error() {
            bail!("the edit was saved but not published: {error}");
        }
        self.library.references(id, &links(&workspace.document))?;
        Ok(outputs)
    }

    /// An asset's history: every revision, and where each session is.
    pub fn history(&self, id: AssetId) -> Result<spectrum_document::History> {
        let asset = self.library.get(id)?;
        let path = self.library.path(&asset)?;
        let history = match asset.kind {
            AssetKind::Image => {
                spectrum_image::Workspace::open_newest(&path, self.actor.clone(), self.session)?
                    .history()?
            }
            AssetKind::Canvas => {
                spectrum_canvas::Workspace::open_newest(&path, self.actor.clone(), self.session)?
                    .history()?
            }
            kind => bail!("{kind} assets have no history yet"),
        };
        history.context("the asset has no saved history")
    }

    /// Returns an asset to an earlier (or later) revision in its history.
    pub fn move_to(&mut self, id: AssetId, revision: spectrum_document::RevisionId) -> Result<()> {
        let asset = self.library.get(id)?;
        let path = self.library.path(&asset)?;
        match asset.kind {
            AssetKind::Image => {
                spectrum_image::Workspace::open_newest(&path, self.actor.clone(), self.session)?
                    .move_to(revision)?;
                self.previews.borrow_mut().remove(&id);
            }
            AssetKind::Canvas => {
                let mut workspace = spectrum_canvas::Workspace::open_newest(
                    &path,
                    self.actor.clone(),
                    self.session,
                )?;
                workspace.move_to(revision)?;
                self.library.references(id, &links(&workspace.document))?;
            }
            kind => bail!("{kind} assets have no history yet"),
        }
        Ok(())
    }

    /// The document an asset of `kind` is stored in, and the session this
    /// service edits in, for editors that work on documents directly.
    pub fn editor_target(&self, id: AssetId, kind: AssetKind) -> Result<(PathBuf, SessionId)> {
        Ok((self.document(id, kind)?, self.session))
    }

    /// Records which images a canvas uses after it was edited directly.
    pub fn index_canvas(&mut self, id: AssetId) -> Result<()> {
        let document = self.saved_canvas(id)?;
        self.library.references(id, &links(&document))
    }

    /// Changes some of an image's adjustments.
    pub fn adjust(&self, id: AssetId, patch: spectrum_image::AdjustmentPatch) -> Result<()> {
        self.edit_image(id, vec![spectrum_image::Command::Adjust { patch }])
            .map(drop)
    }

    /// Replaces an image's whole adjustment set, including curves, HSL, and
    /// color grading, which patches do not cover.
    pub fn set_adjustments(
        &self,
        id: AssetId,
        adjustments: spectrum_image::Adjustments,
    ) -> Result<()> {
        self.edit_image(
            id,
            vec![spectrum_image::Command::SetAdjustments { adjustments }],
        )
        .map(drop)
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
        let command = if forward {
            spectrum_image::Command::Redo
        } else {
            spectrum_image::Command::Undo
        };
        self.edit_image(id, vec![command]).map(drop)
    }

    /// The rendered image a canvas draws for `id`. Trashed and purged images
    /// resolve to a same-sized placeholder.
    pub fn preview(&self, id: AssetId) -> Result<PathBuf> {
        if let Some(removed) = self.library.removed(id)? {
            return self.placeholder(removed.size);
        }
        let document = self.document(id, AssetKind::Image)?;
        let stamp = file_stamp(&document)?;
        if let Some((cached_stamp, path)) = self.previews.borrow().get(&id)
            && *cached_stamp == stamp
            && path.exists()
        {
            return Ok(path.clone());
        }
        let image = self.image(id)?;
        let key = hex(&serde_json::to_vec(&(&image.path, &image.adjustments))?);
        let path = self
            .library
            .root()
            .join("previews")
            .join(format!("{id}-{key}.png"));
        if !path.exists() {
            let rendered = spectrum_image::engine::render(&image, Default::default())?;
            save_atomically(&rendered, &path)?;
        }
        self.previews.borrow_mut().insert(id, (stamp, path.clone()));
        Ok(path)
    }

    /// Points a canvas's linked image layers at their images' current renders.
    pub fn resolve(&self, document: &mut Document) -> Result<()> {
        for layer in &mut document.layers {
            if let Some(id) = layer.image_asset {
                let LayerKind::Raster { path } = &mut layer.kind else {
                    bail!("image reference on non-raster layer");
                };
                *path = self.preview(id)?;
            }
        }
        Ok(())
    }

    /// Places an image on a canvas as a linked layer, returning its layer ID.
    pub fn place(&mut self, canvas: AssetId, image: AssetId) -> Result<u64> {
        let name = self.library.get(image)?.name;
        let path = self.preview(image)?;
        let outputs = self.edit_canvas(
            canvas,
            vec![spectrum_canvas::Command::AddLinkedImage {
                path,
                name,
                asset: image,
            }],
        )?;
        outputs
            .first()
            .and_then(|output| output.layer_ids.first().copied())
            .context("missing placed layer id")
    }

    /// Duplicates an asset. A canvas copy gets its own copies of its images.
    pub fn copy(&mut self, id: AssetId) -> Result<Asset> {
        let asset = self.library.get(id)?;
        let name = format!("{} copy", asset.name);
        match asset.kind {
            AssetKind::Image => self.create_image(&name, self.image(id)?),
            AssetKind::Canvas => {
                let mut document = self.saved_canvas(id)?;
                let mut copied = std::collections::HashMap::new();
                for layer in &mut document.layers {
                    if let Some(source) = layer.image_asset
                        && self.library.removed(source)?.is_none()
                    {
                        let id = match copied.get(&source) {
                            Some(id) => *id,
                            None => {
                                let id = self.copy(source)?.id;
                                copied.insert(source, id);
                                id
                            }
                        };
                        layer.image_asset = Some(id);
                    }
                }
                document.name = name.clone();
                self.resolve(&mut document)?;
                let path = self.new_document(AssetKind::Canvas);
                spectrum_canvas::Workspace::create(
                    &path,
                    document.clone(),
                    self.actor.clone(),
                    self.session,
                )?;
                let copy = self.library.register(AssetKind::Canvas, &name, &path)?;
                self.library.references(copy.id, &links(&document))?;
                Ok(copy)
            }
            kind => bail!("copying is not implemented for {kind}"),
        }
    }
}

/// The images a canvas uses, by layer.
fn links(document: &Document) -> Vec<(String, AssetId)> {
    document
        .layers
        .iter()
        .filter_map(|layer| layer.image_asset.map(|id| (layer.id.to_string(), id)))
        .collect()
}

fn file_stamp(path: &Path) -> Result<(std::time::SystemTime, u64)> {
    let m = std::fs::metadata(path)?;
    Ok((m.modified()?, m.len()))
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Writes a render beside its final name, then moves it into place, so
/// readers never see a partial file.
fn save_atomically(image: &image::DynamicImage, path: &Path) -> Result<()> {
    let temporary = path.with_file_name(format!("{}.png", AssetId::new_v4()));
    let result = image
        .save(&temporary)
        .map_err(anyhow::Error::from)
        .and_then(|()| Ok(std::fs::rename(&temporary, path)?));
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
