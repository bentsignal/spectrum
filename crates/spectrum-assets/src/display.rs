//! Canvases shown before their images' full renders are made. An image
//! edited since its canvas last showed it needs a new full-size render (a
//! second or more for a large, heavily edited photo); until it is made, the
//! canvas shows a smaller render made in a fraction of that time, scaled up
//! to the same size on the canvas.
use super::{Preview, Service, hex, save_atomically};
use anyhow::{Result, bail};
use spectrum_canvas::{Document, LayerKind};
use spectrum_library::AssetId;
use std::path::PathBuf;

/// Long edge of a stand-in render.
const STAND_IN: u32 = 2048;

impl Service {
    /// Points a canvas drawn at `density` device pixels per canvas unit at
    /// renders it can show now: each image's full render when it is made, or
    /// else a stand-in scaled to the same size. Layers with masks, which are
    /// sized to the full render, wait for it. Returns the images shown larger
    /// than their stand-ins, whose full renders are still to make with
    /// [`Service::preview`].
    pub fn resolve_for_display(
        &self,
        document: &mut Document,
        density: f32,
    ) -> Result<Vec<AssetId>> {
        self.resolve_small(document, None, density)
    }

    /// As [`Service::resolve_for_display`], or with `small`, every image as
    /// a stand-in no larger than `small`, for a canvas drawn small (its
    /// thumbnail), which needs no image's full render.
    pub(crate) fn resolve_small(
        &self,
        document: &mut Document,
        small: Option<u32>,
        density: f32,
    ) -> Result<Vec<AssetId>> {
        let mut pending = Vec::new();
        for layer in &mut document.layers {
            let Some(id) = layer.image_asset else {
                continue;
            };
            let LayerKind::Raster { path } = &mut layer.kind else {
                bail!("image reference on non-raster layer");
            };
            let masked =
                layer.pixel_mask.is_some() || layer.vector_mask.is_some() || layer.mask.enabled;
            let target = match (small, masked) {
                (Some(_), false) => Preview::Missing {
                    path: PathBuf::new(),
                    image: Box::new(self.image(id)?),
                    stamp: (std::time::SystemTime::UNIX_EPOCH, 0),
                },
                _ => self.preview_target(id)?,
            };
            match target {
                Preview::Ready(ready) => *path = ready,
                Preview::Missing { .. } if masked => *path = self.preview(id)?,
                Preview::Missing { image, .. } => {
                    let (stand_in, scale) = self.stand_in(id, &image, small.unwrap_or(STAND_IN))?;
                    if small.is_some() {
                        *path = stand_in;
                        layer.transform.scale_x *= scale.0;
                        layer.transform.scale_y *= scale.1;
                        continue;
                    }
                    *path = stand_in;
                    layer.transform.scale_x *= scale.0;
                    layer.transform.scale_y *= scale.1;
                    // Drawn no larger than the stand-in, it is all that shows.
                    let shown = layer
                        .transform
                        .scale_x
                        .abs()
                        .max(layer.transform.scale_y.abs())
                        * density;
                    if shown > 1.0 && !pending.contains(&id) {
                        pending.push(id);
                    }
                }
            }
        }
        Ok(pending)
    }

    /// A small render of `image`, made once per set of edits, and how much
    /// larger its full render is on each axis.
    fn stand_in(
        &self,
        id: AssetId,
        image: &spectrum_image::Image,
        size: u32,
    ) -> Result<(PathBuf, (f32, f32))> {
        let key = hex(&serde_json::to_vec(&(
            &image.path,
            &image.adjustments,
            size,
        ))?);
        let path = self
            .library
            .root()
            .join("previews")
            .join(format!("{id}-{}-stand-in.png", &key[..24]));
        let size = if path.exists() {
            image::image_dimensions(&path)?
        } else {
            let rendered = spectrum_image::engine::render_thumbnail(image, size)?;
            save_atomically(&rendered, &path)?;
            (rendered.width(), rendered.height())
        };
        let full = spectrum_image::render::adjusted_image_dimensions(
            image.width,
            image.height,
            &image.adjustments,
        )
        .unwrap_or(size);
        Ok((
            path,
            (
                full.0 as f32 / size.0.max(1) as f32,
                full.1 as f32 / size.1.max(1) as f32,
            ),
        ))
    }
}
