//! Small cached renders for library grids.
use super::{Service, file_stamp};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use spectrum_library::AssetId;
use std::path::PathBuf;

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Service {
    /// Renders an asset no larger than `max` pixels on its long edge. Trashed
    /// assets still render; purged ones return a placeholder.
    pub fn thumbnail(&self, id: AssetId, max: u32) -> Result<PathBuf> {
        if let Some(removed) = self.library.removed(id)?
            && removed.purged
        {
            return self.placeholder(removed.size);
        }
        let asset = self.library.lookup(id)?;
        let document = self.library.path(&asset)?;
        let previews = self.library.root().join("previews");
        match asset.kind.as_str() {
            "image" => {
                let photo = lumen_core::DurableCatalog::load_photo_current(
                    &document,
                    asset.item.context("image missing item")?,
                )?;
                let key = hex(&serde_json::to_vec(&(
                    &photo.path,
                    &photo.adjustments,
                    max,
                ))?);
                let path = previews.join(format!("{id}-thumb-{key}.png"));
                if !path.exists() {
                    let image = lumen_core::engine::render_photo(
                        &photo,
                        lumen_core::engine::RenderOptions {
                            max_size: Some(max),
                        },
                    )?;
                    let temporary = path.with_file_name(format!("{}.png", AssetId::new_v4()));
                    image.save(&temporary)?;
                    std::fs::rename(&temporary, &path)?;
                }
                Ok(path)
            }
            "canvas" => {
                let (modified, length) = file_stamp(&document)?;
                let key = hex(format!("{modified:?}{length}").as_bytes());
                let path = previews.join(format!("{id}-thumb-{key}.png"));
                if !path.exists() {
                    let mut doc = prism_core::Workspace::load_read_only(&document)?;
                    self.resolve(&mut doc)?;
                    prism_core::export_document(&doc, &path, 90)?;
                }
                Ok(path)
            }
            kind => bail!("no thumbnail for {kind} assets"),
        }
    }
}
