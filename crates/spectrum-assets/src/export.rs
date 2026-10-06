//! Exports render assets to files the user chooses, outside the library.
use super::Service;
use anyhow::{Context, Result, bail};
use spectrum_canvas::Document;
use spectrum_library::{AssetId, AssetKind};
use std::path::Path;

/// Export quality and size. Quality applies to JPEG.
#[derive(Clone, Copy, Debug)]
pub struct ExportOptions {
    pub quality: u8,
    /// Longest edge in pixels; `None` exports at full size.
    pub max_size: Option<u32>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            quality: 92,
            max_size: None,
        }
    }
}

impl Service {
    /// Exports an image or canvas at full size to a path outside the library.
    /// The extension picks the format: .jpg, .jpeg, or .png.
    pub fn export(&self, id: AssetId, destination: &Path) -> Result<()> {
        self.export_with(id, destination, ExportOptions::default())
    }

    /// Exports at a size and quality; the file type follows the destination's extension.
    pub fn export_with(
        &self,
        id: AssetId,
        destination: &Path,
        options: ExportOptions,
    ) -> Result<()> {
        self.check_export(destination)?;
        match self.library.get(id)?.kind {
            AssetKind::Image => spectrum_image::engine::export(
                &self.image(id)?,
                destination,
                spectrum_imaging::RenderOptions {
                    max_size: options.max_size,
                },
                options.quality,
            ),
            AssetKind::Canvas => {
                self.export_canvas_with(&self.saved_canvas(id)?, destination, options)
            }
            kind => bail!("exporting is not implemented for {kind}"),
        }
    }

    /// The width and height an asset exports at full size.
    pub fn export_size(&self, id: AssetId) -> Result<(u32, u32)> {
        match self.library.get(id)?.kind {
            AssetKind::Image => {
                let image = self.image(id)?;
                spectrum_imaging::adjusted_image_dimensions(
                    image.width,
                    image.height,
                    &image.adjustments,
                )
                .context("the image has no size")
            }
            AssetKind::Canvas => {
                let document = self.saved_canvas(id)?;
                Ok((document.width, document.height))
            }
            kind => bail!("exporting is not implemented for {kind}"),
        }
    }

    /// Exports a canvas document, such as one open in an editor.
    pub fn export_canvas_with(
        &self,
        document: &Document,
        path: &Path,
        options: ExportOptions,
    ) -> Result<()> {
        self.check_export(path)?;
        let mut document = document.clone();
        self.resolve(&mut document)?;
        spectrum_canvas::export_document_sized(&document, path, options.quality, options.max_size)
    }

    /// Refuses destinations inside the managed library.
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
