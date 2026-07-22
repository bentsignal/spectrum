use std::{collections::BTreeMap, fmt, sync::Arc};

use anyhow::{Context, Result, bail};
use image::{Rgba, RgbaImage};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::PixelMask;

pub const BRUSH_PROGRAM_VERSION: u32 = 1;
pub const MAX_BRUSH_SAMPLES_PER_STROKE: usize = 4_096;
pub const MAX_BRUSH_STROKES_PER_LAYER: usize = 1_024;
pub const MAX_BRUSH_SAMPLES_PER_DOCUMENT: usize = 131_072;
pub const MAX_BRUSH_DABS_PER_STROKE: usize = 32_768;
pub const MAX_BRUSH_DABS_PER_PROGRAM: usize = 262_144;
pub const MAX_PAINT_REGION_PIXELS: u64 = 4_096 * 4_096;
pub const MAX_BRUSH_CLIP_BYTES_PER_PROGRAM: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrushMode {
    Paint,
    Erase,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushStyle {
    pub mode: BrushMode,
    pub color: [u8; 4],
    pub size: f32,
    pub hardness: f32,
    pub opacity: f32,
    pub spacing: f32,
}

impl Default for BrushStyle {
    fn default() -> Self {
        Self {
            mode: BrushMode::Paint,
            color: [255, 255, 255, 255],
            size: 32.0,
            hardness: 0.8,
            opacity: 1.0,
            spacing: 0.15,
        }
    }
}

impl BrushStyle {
    pub fn validate(self) -> Result<Self> {
        for (name, value) in [
            ("brush size", self.size),
            ("brush hardness", self.hardness),
            ("brush opacity", self.opacity),
            ("brush spacing", self.spacing),
        ] {
            if !value.is_finite() {
                bail!("{name} must be finite");
            }
        }
        if !(1.0..=2_048.0).contains(&self.size) {
            bail!("brush size must be between 1 and 2048 pixels");
        }
        if !(0.0..=1.0).contains(&self.hardness) {
            bail!("brush hardness must be between 0 and 1");
        }
        if !(0.0..=1.0).contains(&self.opacity) {
            bail!("brush opacity must be between 0 and 1");
        }
        if !(0.01..=2.0).contains(&self.spacing) {
            bail!("brush spacing must be between 0.01 and 2");
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushSample {
    pub x: f32,
    pub y: f32,
    /// Version 1 freezes pressure as both diameter and coverage multiplier.
    pub pressure: f32,
}

impl BrushSample {
    fn validate(self, width: u32, height: u32) -> Result<Self> {
        if !self.x.is_finite() || !self.y.is_finite() || !self.pressure.is_finite() {
            bail!("brush samples must contain finite coordinates and pressure");
        }
        if self.x < 0.0 || self.y < 0.0 || self.x > width as f32 || self.y > height as f32 {
            bail!("brush samples must stay inside the Paint viewport");
        }
        if !(0.0..=1.0).contains(&self.pressure) {
            bail!("brush pressure must be between 0 and 1");
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrushClip {
    Rectangle {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    Alpha {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        #[serde(with = "clip_alpha")]
        alpha: Arc<[u8]>,
    },
}

impl BrushClip {
    fn validate(&self, viewport: (u32, u32)) -> Result<()> {
        let (x, y, width, height, alpha_len) = match self {
            Self::Rectangle {
                x,
                y,
                width,
                height,
            } => (*x, *y, *width, *height, None),
            Self::Alpha {
                x,
                y,
                width,
                height,
                alpha,
            } => (*x, *y, *width, *height, Some(alpha.len())),
        };
        if width == 0 || height == 0 {
            bail!("brush clip dimensions must be nonzero");
        }
        let right = x.checked_add(width).context("brush clip overflows")?;
        let bottom = y.checked_add(height).context("brush clip overflows")?;
        if right > viewport.0 || bottom > viewport.1 {
            bail!("brush clip exceeds its Paint viewport");
        }
        let pixels = u64::from(width) * u64::from(height);
        if alpha_len.is_some_and(|alpha_len| {
            pixels > MAX_PAINT_REGION_PIXELS || alpha_len != pixels as usize
        }) {
            bail!("brush clip exceeds its bounded alpha region");
        }
        Ok(())
    }

    pub(crate) fn byte_len(&self) -> usize {
        match self {
            Self::Rectangle { .. } => 0,
            Self::Alpha { alpha, .. } => alpha.len(),
        }
    }

    fn alpha_at(&self, x: u32, y: u32) -> u8 {
        match self {
            Self::Rectangle {
                x: left,
                y: top,
                width,
                height,
            } => u8::from(x >= *left && y >= *top && x < left + width && y < top + height) * 255,
            Self::Alpha {
                x: left,
                y: top,
                width,
                height,
                alpha,
            } if x >= *left && y >= *top && x < left + width && y < top + height => {
                alpha[((y - top) * width + (x - left)) as usize]
            }
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BrushStroke {
    pub style: BrushStyle,
    pub samples: Arc<[BrushSample]>,
    pub clip: Option<BrushClip>,
    content_hash: [u8; 32],
}

#[derive(Serialize, Deserialize)]
struct BrushStrokeWire {
    style: BrushStyle,
    #[serde(with = "stroke_samples")]
    samples: Arc<[BrushSample]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clip: Option<BrushClip>,
}

impl BrushStroke {
    pub fn new(style: BrushStyle, samples: impl Into<Arc<[BrushSample]>>) -> Result<Self> {
        Self::from_parts(style, samples.into(), None, None)
    }

    fn from_parts(
        style: BrushStyle,
        samples: Arc<[BrushSample]>,
        clip: Option<BrushClip>,
        viewport: Option<(u32, u32)>,
    ) -> Result<Self> {
        let style = style.validate()?;
        if samples.is_empty() || samples.len() > MAX_BRUSH_SAMPLES_PER_STROKE {
            bail!("a brush stroke must contain 1 through {MAX_BRUSH_SAMPLES_PER_STROKE} samples");
        }
        if let Some((width, height)) = viewport {
            for sample in samples.iter().copied() {
                sample.validate(width, height)?;
            }
            if let Some(clip) = &clip {
                clip.validate((width, height))?;
            }
        } else if samples.iter().any(|sample| {
            !sample.x.is_finite()
                || !sample.y.is_finite()
                || !sample.pressure.is_finite()
                || !(0.0..=1.0).contains(&sample.pressure)
        }) {
            bail!("brush samples must contain finite coordinates and bounded pressure");
        }
        let mut stroke = Self {
            style,
            samples,
            clip,
            content_hash: [0; 32],
        };
        if stroke.estimated_dab_count()? > MAX_BRUSH_DABS_PER_STROKE {
            bail!("brush stroke exceeds the {MAX_BRUSH_DABS_PER_STROKE}-dab limit");
        }
        stroke.content_hash = stroke.compute_identity();
        Ok(stroke)
    }

    pub fn identity(&self) -> [u8; 32] {
        self.content_hash
    }

    pub(crate) fn validated_for_viewport(&self, width: u32, height: u32) -> Result<Self> {
        Self::from_parts(
            self.style,
            Arc::clone(&self.samples),
            self.clip.clone(),
            Some((width, height)),
        )
    }

    pub(crate) fn with_clip(&self, clip: Option<BrushClip>, viewport: (u32, u32)) -> Result<Self> {
        Self::from_parts(self.style, Arc::clone(&self.samples), clip, Some(viewport))
    }

    fn interval(&self) -> f32 {
        (self.style.size * self.style.spacing).max(0.5)
    }

    fn estimated_dab_count(&self) -> Result<usize> {
        let distance = self.samples.windows(2).try_fold(0.0_f64, |total, pair| {
            let dx = f64::from(pair[1].x - pair[0].x);
            let dy = f64::from(pair[1].y - pair[0].y);
            let segment = dx.hypot(dy);
            if !segment.is_finite() {
                bail!("brush stroke distance overflowed");
            }
            Ok(total + segment)
        })?;
        let quotient = (distance / f64::from(self.interval())).ceil();
        if !quotient.is_finite() || quotient > (MAX_BRUSH_DABS_PER_STROKE - 2) as f64 {
            bail!("brush stroke exceeds the {MAX_BRUSH_DABS_PER_STROKE}-dab limit");
        }
        Ok(quotient as usize + 2)
    }

    fn compute_identity(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(BRUSH_PROGRAM_VERSION.to_le_bytes());
        hash.update([self.style.mode as u8]);
        hash.update(self.style.color);
        for value in [
            self.style.size,
            self.style.hardness,
            self.style.opacity,
            self.style.spacing,
        ] {
            hash.update(value.to_bits().to_le_bytes());
        }
        hash.update((self.samples.len() as u64).to_le_bytes());
        for sample in self.samples.iter() {
            hash.update(sample.x.to_bits().to_le_bytes());
            hash.update(sample.y.to_bits().to_le_bytes());
            hash.update(sample.pressure.to_bits().to_le_bytes());
        }
        match &self.clip {
            Some(BrushClip::Rectangle {
                x,
                y,
                width,
                height,
            }) => {
                hash.update([1]);
                for value in [*x, *y, *width, *height] {
                    hash.update(value.to_le_bytes());
                }
            }
            Some(BrushClip::Alpha {
                x,
                y,
                width,
                height,
                alpha,
            }) => {
                hash.update([2]);
                for value in [*x, *y, *width, *height] {
                    hash.update(value.to_le_bytes());
                }
                hash.update(alpha.as_ref());
            }
            None => hash.update([0]),
        }
        hash.finalize().into()
    }
}

impl Serialize for BrushStroke {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        BrushStrokeWire {
            style: self.style,
            samples: Arc::clone(&self.samples),
            clip: self.clip.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BrushStroke {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = BrushStrokeWire::deserialize(deserializer)?;
        Self::from_parts(wire.style, wire.samples, wire.clip, None)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug)]
pub struct BrushProgram {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub strokes: Arc<[BrushStroke]>,
    content_hash: [u8; 32],
}

impl PartialEq for BrushProgram {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
            && self.width == other.width
            && self.height == other.height
            && self.content_hash == other.content_hash
            && (Arc::ptr_eq(&self.strokes, &other.strokes)
                || self.strokes.as_ref() == other.strokes.as_ref())
    }
}

#[derive(Serialize, Deserialize)]
struct BrushProgramWire {
    version: u32,
    width: u32,
    height: u32,
    #[serde(with = "program_strokes")]
    strokes: Vec<BrushStroke>,
}

impl BrushProgram {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        Self::from_parts(BRUSH_PROGRAM_VERSION, width, height, Arc::from([]))
    }

    fn from_parts(
        version: u32,
        width: u32,
        height: u32,
        strokes: Arc<[BrushStroke]>,
    ) -> Result<Self> {
        if version != BRUSH_PROGRAM_VERSION {
            bail!("unsupported BrushProgram version {version}");
        }
        if width == 0
            || height == 0
            || width > crate::MAX_CANVAS_DIMENSION
            || height > crate::MAX_CANVAS_DIMENSION
        {
            bail!("Paint viewport dimensions are outside Prism limits");
        }
        if strokes.len() > MAX_BRUSH_STROKES_PER_LAYER {
            bail!("Paint layer exceeds the {MAX_BRUSH_STROKES_PER_LAYER}-stroke limit");
        }
        let mut sample_count = 0usize;
        let mut dab_count = 0usize;
        let mut validated = Vec::with_capacity(strokes.len());
        for stroke in strokes.iter() {
            let stroke = stroke.validated_for_viewport(width, height)?;
            sample_count = sample_count
                .checked_add(stroke.samples.len())
                .context("Paint sample count overflowed")?;
            dab_count = dab_count
                .checked_add(stroke.estimated_dab_count()?)
                .context("Paint dab count overflowed")?;
            validated.push(stroke);
        }
        if sample_count > MAX_BRUSH_SAMPLES_PER_DOCUMENT {
            bail!("Paint program exceeds the aggregate sample limit");
        }
        if dab_count > MAX_BRUSH_DABS_PER_PROGRAM {
            bail!("Paint program exceeds the aggregate dab limit");
        }
        let clip_bytes = validated
            .iter()
            .filter_map(|stroke| stroke.clip.as_ref())
            .try_fold(0usize, |total, clip| total.checked_add(clip.byte_len()))
            .context("Paint clip byte count overflowed")?;
        if clip_bytes > MAX_BRUSH_CLIP_BYTES_PER_PROGRAM {
            bail!("Paint program exceeds the aggregate clip-byte limit");
        }
        let strokes: Arc<[BrushStroke]> = validated.into();
        let mut program = Self {
            version,
            width,
            height,
            strokes,
            content_hash: [0; 32],
        };
        program.content_hash = program.compute_identity();
        Ok(program)
    }

    pub fn append(&self, stroke: BrushStroke) -> Result<Self> {
        let mut strokes = self.strokes.to_vec();
        strokes.push(stroke);
        Self::from_parts(self.version, self.width, self.height, strokes.into())
    }

    pub fn identity(&self) -> [u8; 32] {
        self.content_hash
    }

    pub(crate) fn sample_count(&self) -> usize {
        self.strokes.iter().map(|stroke| stroke.samples.len()).sum()
    }

    pub(crate) fn clip_bytes(&self) -> usize {
        self.strokes
            .iter()
            .filter_map(|stroke| stroke.clip.as_ref())
            .map(BrushClip::byte_len)
            .sum()
    }

    pub(crate) fn dab_count(&self) -> Result<usize> {
        self.strokes.iter().try_fold(0usize, |total, stroke| {
            total
                .checked_add(stroke.estimated_dab_count()?)
                .context("Paint dab count overflowed")
        })
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let rebuilt = Self::from_parts(
            self.version,
            self.width,
            self.height,
            Arc::clone(&self.strokes),
        )?;
        if rebuilt.identity() != self.identity() {
            bail!("BrushProgram identity does not match its contents");
        }
        Ok(())
    }

    fn compute_identity(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(self.version.to_le_bytes());
        hash.update(self.width.to_le_bytes());
        hash.update(self.height.to_le_bytes());
        hash.update((self.strokes.len() as u64).to_le_bytes());
        for stroke in self.strokes.iter() {
            hash.update(stroke.identity());
        }
        hash.finalize().into()
    }
}

impl Serialize for BrushProgram {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        BrushProgramWire {
            version: self.version,
            width: self.width,
            height: self.height,
            strokes: self.strokes.to_vec(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BrushProgram {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = BrushProgramWire::deserialize(deserializer)?;
        Self::from_parts(wire.version, wire.width, wire.height, wire.strokes.into())
            .map_err(serde::de::Error::custom)
    }
}

pub(crate) fn render_paint_region(
    program: &BrushProgram,
    pixel_mask: Option<&PixelMask>,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<RgbaImage> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || pixels > MAX_PAINT_REGION_PIXELS {
        bail!("Paint render region exceeds the bounded 4096-square limit");
    }
    if x.checked_add(width)
        .is_none_or(|right| right > program.width)
        || y.checked_add(height)
            .is_none_or(|bottom| bottom > program.height)
    {
        bail!("Paint render region exceeds the Paint viewport");
    }
    let mut output = RgbaImage::new(width, height);
    for stroke in program.strokes.iter() {
        let requested = (x, y, width, height);
        let mut tiles = BTreeMap::<(u32, u32), Vec<u8>>::new();
        for_each_dab(stroke, |dab| {
            for_dab_tiles(dab, stroke.style.size, requested, |key, tile| {
                let coverage = tiles
                    .entry(key)
                    .or_insert_with(|| vec![0; (tile.2 * tile.3) as usize]);
                accumulate_dab(coverage, tile, stroke, dab);
            });
        });
        for (key, coverage) in tiles {
            let tile = paint_tile_region(key, requested);
            for source_y in tile.1..tile.1 + tile.3 {
                for source_x in tile.0..tile.0 + tile.2 {
                    let index = ((source_y - tile.1) * tile.2 + (source_x - tile.0)) as usize;
                    let mut alpha = u32::from(coverage[index]);
                    if let Some(clip) = &stroke.clip {
                        alpha = (alpha * u32::from(clip.alpha_at(source_x, source_y)) + 127) / 255;
                    }
                    alpha = (alpha * (stroke.style.opacity * 255.0).round() as u32 + 127) / 255;
                    if stroke.style.mode == BrushMode::Paint {
                        alpha = (alpha * u32::from(stroke.style.color[3]) + 127) / 255;
                    }
                    if alpha == 0 {
                        continue;
                    }
                    let destination = output.get_pixel_mut(source_x - x, source_y - y);
                    match stroke.style.mode {
                        BrushMode::Paint => {
                            source_over(destination, stroke.style.color, alpha as u8)
                        }
                        BrushMode::Erase => destination_out(destination, alpha as u8),
                    }
                }
            }
        }
    }
    if let Some(mask) = pixel_mask {
        if (mask.width, mask.height) != (program.width, program.height) {
            bail!("Paint pixel mask dimensions do not match its viewport");
        }
        for local_y in 0..height {
            for local_x in 0..width {
                let mask_alpha =
                    u16::from(mask.alpha[((y + local_y) * mask.width + x + local_x) as usize]);
                let pixel = output.get_pixel_mut(local_x, local_y);
                pixel[3] = ((u16::from(pixel[3]) * mask_alpha + 127) / 255) as u8;
                if pixel[3] == 0 {
                    *pixel = Rgba([0; 4]);
                }
            }
        }
    }
    Ok(output)
}

const PAINT_TILE_SIZE: u32 = 64;

fn for_dab_tiles(
    dab: Dab,
    brush_size: f32,
    requested: (u32, u32, u32, u32),
    mut visit: impl FnMut((u32, u32), (u32, u32, u32, u32)),
) {
    let radius = brush_size * dab.pressure * 0.5 + 0.5;
    if radius <= 0.5 {
        return;
    }
    let left = (dab.x - radius).floor().max(requested.0 as f32) as u32;
    let top = (dab.y - radius).floor().max(requested.1 as f32) as u32;
    let right = (dab.x + radius)
        .ceil()
        .min((requested.0 + requested.2) as f32) as u32;
    let bottom = (dab.y + radius)
        .ceil()
        .min((requested.1 + requested.3) as f32) as u32;
    if right <= left || bottom <= top {
        return;
    }
    for tile_y in top / PAINT_TILE_SIZE..=(bottom - 1) / PAINT_TILE_SIZE {
        for tile_x in left / PAINT_TILE_SIZE..=(right - 1) / PAINT_TILE_SIZE {
            let key = (tile_x, tile_y);
            visit(key, paint_tile_region(key, requested));
        }
    }
}

fn paint_tile_region(key: (u32, u32), requested: (u32, u32, u32, u32)) -> (u32, u32, u32, u32) {
    let left = (key.0 * PAINT_TILE_SIZE).max(requested.0);
    let top = (key.1 * PAINT_TILE_SIZE).max(requested.1);
    let right = ((key.0 + 1) * PAINT_TILE_SIZE).min(requested.0 + requested.2);
    let bottom = ((key.1 + 1) * PAINT_TILE_SIZE).min(requested.1 + requested.3);
    (left, top, right - left, bottom - top)
}

#[derive(Clone, Copy)]
struct Dab {
    x: f32,
    y: f32,
    pressure: f32,
}

fn for_each_dab(stroke: &BrushStroke, mut visit: impl FnMut(Dab)) {
    let first = stroke.samples[0];
    visit(Dab {
        x: first.x,
        y: first.y,
        pressure: first.pressure,
    });
    let interval = stroke.interval();
    let mut distance_to_next = interval;
    let mut last_emitted = (first.x, first.y);
    for pair in stroke.samples.windows(2) {
        let start = pair[0];
        let end = pair[1];
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        let length = dx.hypot(dy);
        if length <= f32::EPSILON {
            continue;
        }
        let mut traveled = distance_to_next;
        while traveled <= length {
            let t = traveled / length;
            let dab = Dab {
                x: start.x + dx * t,
                y: start.y + dy * t,
                pressure: start.pressure + (end.pressure - start.pressure) * t,
            };
            visit(dab);
            last_emitted = (dab.x, dab.y);
            traveled += interval;
        }
        distance_to_next = traveled - length;
    }
    let last = *stroke.samples.last().expect("validated stroke is nonempty");
    if (last.x - last_emitted.0).hypot(last.y - last_emitted.1) > 0.0001 {
        visit(Dab {
            x: last.x,
            y: last.y,
            pressure: last.pressure,
        });
    }
}

fn accumulate_dab(
    coverage: &mut [u8],
    region: (u32, u32, u32, u32),
    stroke: &BrushStroke,
    dab: Dab,
) {
    let radius = stroke.style.size * dab.pressure * 0.5;
    if radius <= 0.0 {
        return;
    }
    let extent = radius + 0.5;
    let left = (dab.x - extent).floor().max(region.0 as f32) as u32;
    let top = (dab.y - extent).floor().max(region.1 as f32) as u32;
    let right = (dab.x + extent).ceil().min((region.0 + region.2) as f32) as u32;
    let bottom = (dab.y + extent).ceil().min((region.1 + region.3) as f32) as u32;
    let hard_radius = radius * stroke.style.hardness;
    for y in top..bottom {
        for x in left..right {
            let distance =
                ((x as f32 + 0.5 - dab.x).powi(2) + (y as f32 + 0.5 - dab.y).powi(2)).sqrt();
            let edge = radius + 0.5;
            let radial = if distance <= hard_radius {
                1.0
            } else {
                ((edge - distance) / (edge - hard_radius).max(0.0001)).clamp(0.0, 1.0)
            };
            let value = (radial * dab.pressure * 255.0).round() as u8;
            let index = ((y - region.1) * region.2 + (x - region.0)) as usize;
            coverage[index] = coverage[index].max(value);
        }
    }
}

fn source_over(destination: &mut Rgba<u8>, color: [u8; 4], source_alpha: u8) {
    let source_alpha = u32::from(source_alpha);
    let destination_alpha = u32::from(destination[3]);
    let retained = (destination_alpha * (255 - source_alpha) + 127) / 255;
    let output_alpha = source_alpha + retained;
    if output_alpha == 0 {
        *destination = Rgba([0; 4]);
        return;
    }
    for channel in 0..3 {
        destination[channel] = ((u32::from(color[channel]) * source_alpha
            + u32::from(destination[channel]) * retained
            + output_alpha / 2)
            / output_alpha) as u8;
    }
    destination[3] = output_alpha.min(255) as u8;
}

fn destination_out(destination: &mut Rgba<u8>, source_alpha: u8) {
    let alpha = (u32::from(destination[3]) * (255 - u32::from(source_alpha)) + 127) / 255;
    destination[3] = alpha as u8;
    if alpha == 0 {
        *destination = Rgba([0; 4]);
    }
}

mod stroke_samples {
    use super::*;
    use serde::de::{SeqAccess, Visitor};

    pub fn serialize<S: Serializer>(
        samples: &Arc<[BrushSample]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        samples.as_ref().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<[BrushSample]>, D::Error> {
        struct SamplesVisitor;

        impl<'de> Visitor<'de> for SamplesVisitor {
            type Value = Arc<[BrushSample]>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    formatter,
                    "at most {MAX_BRUSH_SAMPLES_PER_STROKE} brush samples"
                )
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let capacity = sequence
                    .size_hint()
                    .unwrap_or(0)
                    .min(MAX_BRUSH_SAMPLES_PER_STROKE);
                let mut samples = Vec::with_capacity(capacity);
                while let Some(sample) = sequence.next_element::<BrushSample>()? {
                    if samples.len() == MAX_BRUSH_SAMPLES_PER_STROKE {
                        return Err(serde::de::Error::custom(
                            "brush sample count exceeds its bound",
                        ));
                    }
                    samples.push(sample);
                }
                Ok(samples.into())
            }
        }

        deserializer.deserialize_seq(SamplesVisitor)
    }
}

mod program_strokes {
    use super::*;
    use serde::de::{SeqAccess, Visitor};

    pub fn serialize<S: Serializer>(
        strokes: &[BrushStroke],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        strokes.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<BrushStroke>, D::Error> {
        struct StrokesVisitor;

        impl<'de> Visitor<'de> for StrokesVisitor {
            type Value = Vec<BrushStroke>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    formatter,
                    "at most {MAX_BRUSH_STROKES_PER_LAYER} brush strokes"
                )
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let capacity = sequence
                    .size_hint()
                    .unwrap_or(0)
                    .min(MAX_BRUSH_STROKES_PER_LAYER);
                let mut strokes = Vec::with_capacity(capacity);
                let mut samples = 0usize;
                let mut dabs = 0usize;
                let mut clip_bytes = 0usize;
                while let Some(stroke) = sequence.next_element::<BrushStroke>()? {
                    if strokes.len() == MAX_BRUSH_STROKES_PER_LAYER {
                        return Err(serde::de::Error::custom(
                            "Paint stroke count exceeds its bound",
                        ));
                    }
                    samples = samples
                        .checked_add(stroke.samples.len())
                        .ok_or_else(|| serde::de::Error::custom("Paint sample count overflowed"))?;
                    dabs = dabs
                        .checked_add(
                            stroke
                                .estimated_dab_count()
                                .map_err(serde::de::Error::custom)?,
                        )
                        .ok_or_else(|| serde::de::Error::custom("Paint dab count overflowed"))?;
                    clip_bytes = clip_bytes
                        .checked_add(stroke.clip.as_ref().map_or(0, BrushClip::byte_len))
                        .ok_or_else(|| serde::de::Error::custom("Paint clip bytes overflowed"))?;
                    if samples > MAX_BRUSH_SAMPLES_PER_DOCUMENT
                        || dabs > MAX_BRUSH_DABS_PER_PROGRAM
                        || clip_bytes > MAX_BRUSH_CLIP_BYTES_PER_PROGRAM
                    {
                        return Err(serde::de::Error::custom(
                            "Paint program exceeds its aggregate bounds",
                        ));
                    }
                    strokes.push(stroke);
                }
                Ok(strokes)
            }
        }

        deserializer.deserialize_seq(StrokesVisitor)
    }
}

mod clip_alpha {
    use base64::{Engine, engine::general_purpose::STANDARD};

    use super::*;
    use serde::de::Visitor;

    const MAX_ENCODED_BYTES: usize = (MAX_PAINT_REGION_PIXELS as usize).div_ceil(3) * 4;

    pub fn serialize<S: Serializer>(bytes: &Arc<[u8]>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes.as_ref()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Arc<[u8]>, D::Error> {
        struct AlphaVisitor;

        impl<'de> Visitor<'de> for AlphaVisitor {
            type Value = Arc<[u8]>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a bounded base64 brush clip")
            }

            fn visit_borrowed_str<E: serde::de::Error>(
                self,
                value: &str,
            ) -> Result<Self::Value, E> {
                self.visit_str(value)
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                if value.len() > MAX_ENCODED_BYTES {
                    return Err(E::custom("brush clip exceeds its encoded limit"));
                }
                STANDARD.decode(value).map(Arc::from).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(AlphaVisitor)
    }
}
