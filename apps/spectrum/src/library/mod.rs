//! Spectrum orchestration: editor engines adapt their content to the neutral library.
use anyhow::{Context, Result, bail};
use lumen_core::{DurableCatalog, Project};
use prism_core::{Document, LayerKind};
use sha2::{Digest, Sha256};
use spectrum_library::{Asset, AssetId, Library};
use spectrum_revisions::{Actor, ActorKind, SessionId};
use std::path::{Path, PathBuf};

pub use spectrum_library::default_root;
pub mod live;
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
    pub fn import(&self, paths: Vec<PathBuf>) -> Result<Vec<Asset>> {
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
    pub fn preview(&self, id: AssetId) -> Result<PathBuf> {
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
                    if let Some(source) = layer.image_asset {
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
