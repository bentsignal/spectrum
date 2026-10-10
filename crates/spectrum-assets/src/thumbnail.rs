//! Small cached renders for library grids.
use super::{Service, file_stamp, hex, save_atomically};
use anyhow::{Result, bail};
use spectrum_library::{AssetId, AssetKind};
use std::path::PathBuf;

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
        match asset.kind {
            AssetKind::Image => {
                // A thumbnail is a look at the asset: the person follows an
                // agent they work together with, as when opening it.
                let image = self.read::<spectrum_image::ImageModel>(&document, true)?.0;
                let key = hex(&serde_json::to_vec(&(
                    &image.path,
                    &image.adjustments,
                    max,
                ))?);
                let path = previews.join(format!("{id}-thumb-{}.png", &key[..24]));
                if !path.exists() {
                    let rendered = spectrum_image::engine::render_thumbnail(&image, max)?;
                    save_atomically(&rendered, &path)?;
                }
                Ok(path)
            }
            AssetKind::Canvas => {
                let (modified, length) = file_stamp(&document)?;
                let key = hex(format!("{modified:?}{length}{max}").as_bytes());
                let path = previews.join(format!("{id}-thumb-{}.png", &key[..24]));
                if !path.exists() {
                    let mut doc = self
                        .read::<spectrum_canvas::CanvasModel>(&document, true)?
                        .0;
                    // Small renders of its images, made in a fraction of the
                    // time their full renders would take.
                    self.resolve_small(&mut doc, Some((max * 2).max(256)))?;
                    spectrum_canvas::export_document_sized(&doc, &path, 90, Some(max))?;
                }
                Ok(path)
            }
            kind => bail!("no thumbnail for {kind} assets"),
        }
    }
}
