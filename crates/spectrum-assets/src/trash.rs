//! Deleting moves assets to the trash; purging removes their documents.
use super::Service;
use anyhow::Result;
use serde::Serialize;
use spectrum_library::{Asset, AssetId, AssetKind, Project, Trashed};
use std::path::PathBuf;

/// What a deletion affects, for confirmation and reporting.
#[derive(Serialize)]
pub struct Usage {
    pub projects: Vec<Project>,
    /// Live assets, such as canvases, that show a placeholder while it is gone.
    pub dependents: Vec<Asset>,
}

#[derive(Serialize)]
pub struct Deleted {
    pub trashed: Trashed,
    #[serde(flatten)]
    pub usage: Usage,
}

impl Service {
    pub fn usage(&self, id: AssetId) -> Result<Usage> {
        Ok(Usage {
            projects: self.library.asset_projects(id)?,
            dependents: self
                .library
                .dependents(id)?
                .into_iter()
                .filter_map(|d| self.library.get(d).ok())
                .collect(),
        })
    }

    /// Moves an asset to the trash. Canvases that use it draw a placeholder of
    /// the same size until it is restored.
    pub fn delete(&mut self, id: AssetId) -> Result<Deleted> {
        let asset = self.library.get(id)?;
        let size = if asset.kind == AssetKind::Image {
            Some(image::image_dimensions(self.preview(id)?)?)
        } else {
            None
        };
        let usage = self.usage(id)?;
        let trashed = self.library.trash(id, size)?;
        self.previews.borrow_mut().remove(&id);
        Ok(Deleted { trashed, usage })
    }

    pub fn restore(&mut self, id: AssetId) -> Result<Asset> {
        self.previews.borrow_mut().remove(&id);
        self.library.restore(id)
    }

    /// Permanently removes a trashed asset's document and index entry.
    pub fn purge(&mut self, id: AssetId) -> Result<()> {
        let asset = self.library.lookup(id)?;
        let path = self.library.root().join(&asset.document);
        self.library.forget(id)?;
        if !self.library.document_in_use(&asset.document)? {
            spectrum_document::remove(&path)?;
        }
        Ok(())
    }

    /// Purges trashed assets older than 30 days. Returns the purged IDs.
    pub fn purge_expired(&mut self) -> Result<Vec<AssetId>> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;
        let expired = self.library.expired(now)?;
        for asset in &expired {
            self.purge(asset.id)?;
        }
        Ok(expired.into_iter().map(|a| a.id).collect())
    }

    /// A neutral image with a cross, sized like the missing image.
    pub(super) fn placeholder(&self, size: Option<(u32, u32)>) -> Result<PathBuf> {
        let (width, height) = size.unwrap_or((1024, 768));
        let path = self
            .library
            .root()
            .join("previews")
            .join(format!("missing-{width}x{height}.png"));
        if path.exists() {
            return Ok(path);
        }
        let stroke = (width.min(height) / 160).max(1) as f32;
        let (w, h) = (width as f32, height as f32);
        let length = (w * w + h * h).sqrt();
        let image = image::RgbaImage::from_fn(width, height, |x, y| {
            let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
            let down = (h * x - w * y).abs() / length;
            let up = (h * x + w * y - w * h).abs() / length;
            let edge =
                x < stroke * 2. || y < stroke * 2. || x > w - stroke * 2. || y > h - stroke * 2.;
            if down < stroke || up < stroke || edge {
                image::Rgba([72, 72, 72, 255])
            } else {
                image::Rgba([38, 38, 38, 255])
            }
        });
        let temporary = path.with_file_name(format!("{}.png", AssetId::new_v4()));
        image.save(&temporary)?;
        std::fs::rename(&temporary, &path)?;
        Ok(path)
    }
}
