use std::{fmt, sync::Arc};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::{
    SampledBrushSource, SampledSourceId, SampledSourceMapping, SampledSourceSnapshot, Transform,
};

#[cfg(test)]
pub(crate) use crate::paint_render::render_paint_region;

pub const BRUSH_PROGRAM_VERSION: u32 = 2;
const LEGACY_BRUSH_PROGRAM_VERSION: u32 = 1;
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
    CloneStamp,
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

    pub(crate) fn alpha_at(&self, x: u32, y: u32) -> u8 {
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
    pub source: Option<SampledBrushSource>,
    content_hash: [u8; 32],
}

#[derive(Serialize, Deserialize)]
struct BrushStrokeWire {
    style: BrushStyle,
    #[serde(with = "stroke_samples")]
    samples: Arc<[BrushSample]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clip: Option<BrushClip>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<SampledBrushSource>,
}

impl BrushStroke {
    pub fn new(style: BrushStyle, samples: impl Into<Arc<[BrushSample]>>) -> Result<Self> {
        Self::from_parts(style, samples.into(), None, None, None, true)
    }

    pub fn new_clone_stamp(
        mut style: BrushStyle,
        samples: impl Into<Arc<[BrushSample]>>,
        source: SampledSourceSnapshot,
    ) -> Result<Self> {
        style.mode = BrushMode::CloneStamp;
        let samples = samples.into();
        let first = samples
            .first()
            .context("a Clone Stamp stroke requires a destination anchor")?;
        let mapping = SampledSourceMapping::capture(
            &source,
            [first.x, first.y],
            (source.width, source.height),
            Transform::default(),
        )?;
        let source_id = source.stable_id()?;
        Self::from_parts(
            style,
            samples,
            None,
            Some(SampledBrushSource::resolved_clone(source_id, mapping)),
            None,
            false,
        )
    }

    fn from_parts(
        style: BrushStyle,
        samples: Arc<[BrushSample]>,
        clip: Option<BrushClip>,
        source: Option<SampledBrushSource>,
        viewport: Option<(u32, u32)>,
        allow_current_source: bool,
    ) -> Result<Self> {
        let style = style.validate()?;
        match (style.mode, &source) {
            (BrushMode::CloneStamp, Some(SampledBrushSource::CurrentClone))
                if allow_current_source => {}
            (BrushMode::CloneStamp, Some(SampledBrushSource::CloneStamp { mapping, .. })) => {
                mapping.validate()?;
            }
            (BrushMode::CloneStamp, _) => {
                bail!("Clone Stamp strokes require one immutable sampled source")
            }
            (_, None) => {}
            (_, Some(_)) => bail!("only Clone Stamp strokes can carry a sampled source"),
        }
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
            source,
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
            self.source.clone(),
            Some((width, height)),
            false,
        )
    }

    pub(crate) fn with_clip(&self, clip: Option<BrushClip>, viewport: (u32, u32)) -> Result<Self> {
        Self::from_parts(
            self.style,
            Arc::clone(&self.samples),
            clip,
            self.source.clone(),
            Some(viewport),
            false,
        )
    }

    pub(crate) fn resolve_current_clone(
        &self,
        current: Option<(&SampledSourceId, &SampledSourceSnapshot)>,
        destination_dimensions: (u32, u32),
        destination_transform: Transform,
    ) -> Result<Self> {
        if self.style.mode != BrushMode::CloneStamp {
            return Ok(self.clone());
        }
        let source = match &self.source {
            Some(SampledBrushSource::CurrentClone) => {
                let (source_id, current) =
                    current.context("set a Clone Stamp source before painting")?;
                let first = self
                    .samples
                    .first()
                    .context("a Clone Stamp stroke requires a destination anchor")?;
                Some(SampledBrushSource::resolved_clone(
                    source_id.clone(),
                    SampledSourceMapping::capture(
                        current,
                        [first.x, first.y],
                        destination_dimensions,
                        destination_transform,
                    )?,
                ))
            }
            Some(source @ SampledBrushSource::CloneStamp { .. }) => Some(source.clone()),
            None => bail!("Clone Stamp strokes require one immutable sampled source"),
        };
        Self::from_parts(
            self.style,
            Arc::clone(&self.samples),
            self.clip.clone(),
            source,
            None,
            false,
        )
    }

    pub fn as_current_clone(&self) -> Result<Self> {
        let mut style = self.style;
        style.mode = BrushMode::CloneStamp;
        Self::from_parts(
            style,
            Arc::clone(&self.samples),
            self.clip.clone(),
            Some(SampledBrushSource::CurrentClone),
            None,
            true,
        )
    }

    pub(crate) fn sampled_source_id(&self) -> Option<&SampledSourceId> {
        self.source.as_ref().and_then(SampledBrushSource::source_id)
    }

    pub fn sampled_source_identity(&self) -> Option<[u8; 32]> {
        self.source.as_ref().map(SampledBrushSource::identity)
    }

    pub(crate) fn sampled_source_mapping(&self) -> Option<SampledSourceMapping> {
        self.source.as_ref().and_then(SampledBrushSource::mapping)
    }

    pub(crate) fn interval(&self) -> f32 {
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
        match &self.source {
            Some(source) => {
                hash.update([1]);
                hash.update(source.identity());
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
            source: self.source.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BrushStroke {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = BrushStrokeWire::deserialize(deserializer)?;
        Self::from_parts(wire.style, wire.samples, wire.clip, wire.source, None, true)
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
        Self::from_parts(LEGACY_BRUSH_PROGRAM_VERSION, width, height, Arc::from([]))
    }

    fn from_parts(
        version: u32,
        width: u32,
        height: u32,
        strokes: Arc<[BrushStroke]>,
    ) -> Result<Self> {
        if !(LEGACY_BRUSH_PROGRAM_VERSION..=BRUSH_PROGRAM_VERSION).contains(&version) {
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
            if version == LEGACY_BRUSH_PROGRAM_VERSION && stroke.style.mode == BrushMode::CloneStamp
            {
                bail!("BrushProgram version 1 cannot contain Clone Stamp strokes");
            }
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
        let version = if stroke.style.mode == BrushMode::CloneStamp {
            BRUSH_PROGRAM_VERSION
        } else {
            self.version
        };
        strokes.push(stroke);
        Self::from_parts(version, self.width, self.height, strokes.into())
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

    pub(crate) fn contains_sampled_sources(&self) -> bool {
        self.strokes
            .iter()
            .any(|stroke| stroke.sampled_source_id().is_some())
    }

    pub(crate) fn for_each_sampled_source_id(&self, mut visit: impl FnMut(&SampledSourceId)) {
        for stroke in self.strokes.iter() {
            if let Some(source_id) = stroke.sampled_source_id() {
                visit(source_id);
            }
        }
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
