//! Rendering one image again and again as its adjustments change, as a
//! slider drag does. Geometry and noise reduction rarely change during a
//! drag, so their result is kept and only color, spots, and sharpening run
//! for each frame.
use std::sync::{Arc, Mutex};

use image::{DynamicImage, RgbaImage};

use super::{apply_geometry, develop, has_pixel_adjustments, reduce_noise};
use crate::{Adjustments, CropRect};

/// The adjustments that shape the pixels color starts from.
#[derive(Clone, PartialEq)]
struct Stage {
    rotation: i32,
    flip_horizontal: bool,
    flip_vertical: bool,
    straighten: f32,
    crop: Option<CropRect>,
    noise_reduction: f32,
}

impl Stage {
    fn of(adjustments: &Adjustments) -> Self {
        Self {
            rotation: adjustments.rotation,
            flip_horizontal: adjustments.flip_horizontal,
            flip_vertical: adjustments.flip_vertical,
            straighten: adjustments.straighten,
            crop: adjustments.crop,
            noise_reduction: adjustments.noise_reduction,
        }
    }
}

/// An image rendered repeatedly with changing adjustments.
pub struct StagedRender {
    source: DynamicImage,
    prepared: Mutex<Option<(Stage, Arc<RgbaImage>)>>,
    /// How many times geometry and noise reduction ran.
    preparations: std::sync::atomic::AtomicU64,
}

impl StagedRender {
    pub fn new(source: DynamicImage) -> Self {
        Self {
            source,
            prepared: Mutex::new(None),
            preparations: Default::default(),
        }
    }

    pub fn source(&self) -> &DynamicImage {
        &self.source
    }

    /// How many times geometry and noise reduction ran: once per change to
    /// them, never per frame of a drag on another slider.
    pub fn preparations(&self) -> u64 {
        self.preparations.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The pixels `render_image(source, adjustments, RenderOptions::default())`
    /// gives, as RGBA.
    pub fn render(&self, adjustments: Adjustments) -> RgbaImage {
        let adjustments = adjustments.sanitized();
        let mut pixels = RgbaImage::clone(&self.prepared(&adjustments));
        if has_pixel_adjustments(&adjustments) {
            develop(&mut pixels, &adjustments);
        }
        pixels
    }

    fn prepared(&self, adjustments: &Adjustments) -> Arc<RgbaImage> {
        let stage = Stage::of(adjustments);
        let mut prepared = self
            .prepared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((kept, pixels)) = prepared.as_ref()
            && *kept == stage
        {
            return pixels.clone();
        }
        self.preparations
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let pixels = Arc::new(reduce_noise(
            super::filters::to_rgba(apply_geometry(self.source.clone(), adjustments)),
            adjustments,
        ));
        *prepared = Some((stage, pixels.clone()));
        pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ColorGrade, CurvePoint, RenderOptions, SpotRemoval, ToneCurve, render::render_image,
    };
    use image::{Rgb, RgbImage};

    /// A full editing pass: every tab.
    fn full_pass() -> Adjustments {
        let mut adjustments = Adjustments {
            exposure: 0.4,
            temperature: 10.0,
            tint: 5.0,
            contrast: 20.0,
            highlights: -30.0,
            shadows: 25.0,
            whites: 10.0,
            blacks: -10.0,
            texture: 15.0,
            clarity: 20.0,
            dehaze: 10.0,
            vibrance: 15.0,
            saturation: 5.0,
            vignette: -20.0,
            sharpening: 40.0,
            noise_reduction: 20.0,
            rotation: 90,
            straighten: 1.5,
            crop: Some(CropRect {
                x: 0.05,
                y: 0.05,
                width: 0.9,
                height: 0.9,
            }),
            spots: vec![SpotRemoval {
                x: 0.4,
                y: 0.5,
                radius: 0.05,
                opacity: 1.0,
            }],
            ..Default::default()
        };
        adjustments.hsl.blue.saturation = -20.0;
        adjustments.color_grading.shadows = ColorGrade {
            hue: 200.0,
            saturation: 20.0,
            luminance: 0.0,
        };
        adjustments.curves.master = ToneCurve {
            points: vec![
                CurvePoint { x: 0.0, y: 0.05 },
                CurvePoint { x: 0.5, y: 0.55 },
                CurvePoint { x: 1.0, y: 1.0 },
            ],
        };
        adjustments
    }

    #[test]
    fn a_drag_on_a_fully_edited_photo_repeats_only_color_work() {
        let source = DynamicImage::ImageRgb8(RgbImage::from_fn(120, 80, |x, y| {
            Rgb([(x * 2) as u8, (y * 3) as u8, (x + y) as u8])
        }));
        let staged = StagedRender::new(source);
        for step in 0..30 {
            let mut adjustments = full_pass();
            adjustments.exposure = step as f32 * 0.05;
            adjustments.hsl.blue.saturation = -(step as f32);
            staged.render(adjustments);
        }
        assert_eq!(staged.preparations(), 1);
        staged.render(Adjustments {
            straighten: 3.0,
            ..full_pass()
        });
        assert_eq!(staged.preparations(), 2);
    }

    #[test]
    fn staged_frames_match_whole_renders_as_adjustments_change() {
        let source = DynamicImage::ImageRgb8(RgbImage::from_fn(301, 203, |x, y| {
            Rgb([(x * 3) as u8, (y * 5) as u8, (x ^ y) as u8])
        }));
        let staged = StagedRender::new(source.clone());
        let full = full_pass();
        let frames = [
            Adjustments::default(),
            full.clone(),
            Adjustments {
                exposure: -0.7,
                ..full.clone()
            },
            Adjustments {
                straighten: -3.0,
                noise_reduction: 0.0,
                ..full.clone()
            },
            Adjustments {
                rotation: 0,
                exposure: 0.2,
                ..Default::default()
            },
        ];
        for adjustments in frames {
            let whole = render_image(
                source.clone(),
                adjustments.clone(),
                RenderOptions::default(),
            );
            assert!(staged.render(adjustments) == whole.to_rgba8());
        }
    }
}
