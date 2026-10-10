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
mod display;
mod export;
mod sessions;
pub mod thumbnail;
pub mod trash;
pub use export::ExportOptions;

type PreviewCache = std::collections::HashMap<AssetId, ((std::time::SystemTime, u64), PathBuf)>;

/// A library opened by someone: the person at the desktop, or an agent.
/// Each asset's history is a tree; the person and each agent move through it
/// in their own session, so an agent never changes what the person sees
/// unless the person follows it.
pub struct Service {
    previews: std::cell::RefCell<PreviewCache>,
    pub library: Library,
    actor: Actor,
    /// This person's or agent's lasting session in the library.
    session: SessionId,
    /// The person's session, which agents start from.
    person: SessionId,
    /// The agent session chosen for this service's edits, if any; otherwise
    /// an agent works together with the person.
    chosen: Option<SessionId>,
}

impl Service {
    /// Opens a library as the person using Spectrum.
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_as(root, person())
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
        let sessions = library.root().join("sessions");
        let person = spectrum_document::local_session_id(&sessions.join("person"))?;
        let session = match actor.kind {
            ActorKind::Human => person,
            _ => spectrum_document::local_session_id(&sessions.join("agent"))?,
        };
        Ok(Self {
            library,
            previews: Default::default(),
            actor,
            session,
            person,
            chosen: None,
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

    /// The rendered image a canvas draws for `id`. Trashed and purged images
    /// resolve to a same-sized placeholder.
    pub fn preview(&self, id: AssetId) -> Result<PathBuf> {
        match self.preview_target(id)? {
            Preview::Ready(path) => Ok(path),
            Preview::Missing { path, image, stamp } => {
                let rendered = spectrum_image::engine::render(&image, Default::default())?;
                save_atomically(&rendered, &path)?;
                self.previews.borrow_mut().insert(id, (stamp, path.clone()));
                Ok(path)
            }
        }
    }

    /// Where `id`'s render is, and whether it still has to be made.
    fn preview_target(&self, id: AssetId) -> Result<Preview> {
        if let Some(removed) = self.library.removed(id)? {
            return self.placeholder(removed.size).map(Preview::Ready);
        }
        let document = self.document(id, AssetKind::Image)?;
        let stamp = file_stamp(&document)?;
        if let Some((cached_stamp, path)) = self.previews.borrow().get(&id)
            && *cached_stamp == stamp
            && path.exists()
        {
            return Ok(Preview::Ready(path.clone()));
        }
        let image = self.image(id)?;
        // An unedited photo the canvas can read is its own render: placing
        // or showing it renders nothing.
        if image.adjustments.is_identity() && !spectrum_image::is_raw_image(&image.path) {
            self.previews
                .borrow_mut()
                .insert(id, (stamp, image.path.clone()));
            return Ok(Preview::Ready(image.path));
        }
        let key = hex(&serde_json::to_vec(&(&image.path, &image.adjustments))?);
        let path = self
            .library
            .root()
            .join("previews")
            .join(format!("{id}-{key}.png"));
        if path.exists() {
            self.previews.borrow_mut().insert(id, (stamp, path.clone()));
            return Ok(Preview::Ready(path));
        }
        Ok(Preview::Missing {
            path,
            image: Box::new(image),
            stamp,
        })
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

fn person() -> Actor {
    Actor {
        id: "person:local".into(),
        display_name: "You".into(),
        kind: ActorKind::Human,
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
/// An image's render for canvases: on disk, or still to make.
enum Preview {
    Ready(PathBuf),
    Missing {
        path: PathBuf,
        image: Box<spectrum_image::Image>,
        stamp: (std::time::SystemTime, u64),
    },
}

/// Saves a render as a quickly written PNG: a canvas reads it back once into
/// its own cache, so speed matters more than size.
fn save_atomically(image: &image::DynamicImage, path: &Path) -> Result<()> {
    use image::{
        ImageEncoder,
        codecs::png::{CompressionType, FilterType, PngEncoder},
    };
    let temporary = path.with_file_name(format!("{}.png", AssetId::new_v4()));
    let write = || -> Result<()> {
        let file = std::io::BufWriter::new(std::fs::File::create(&temporary)?);
        let encoder =
            PngEncoder::new_with_quality(file, CompressionType::Fast, FilterType::Adaptive);
        let image = match image {
            image::DynamicImage::ImageRgb8(_) | image::DynamicImage::ImageRgba8(_) => {
                std::borrow::Cow::Borrowed(image)
            }
            other if other.color().has_alpha() => {
                std::borrow::Cow::Owned(image::DynamicImage::ImageRgba8(other.to_rgba8()))
            }
            other => std::borrow::Cow::Owned(image::DynamicImage::ImageRgb8(other.to_rgb8())),
        };
        encoder.write_image(
            image.as_bytes(),
            image.width(),
            image.height(),
            image.color().into(),
        )?;
        Ok(std::fs::rename(&temporary, path)?)
    };
    let result = write();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
