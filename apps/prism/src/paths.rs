use std::{fmt, sync::Arc};

use anyhow::{Result, bail};
use image::{Rgba, RgbaImage};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{SeqAccess, Visitor},
    ser::SerializeStruct,
};
use sha2::{Digest, Sha256};

use tiny_skia::{FillRule, LineCap, LineJoin, Mask, PathBuilder, Stroke, Transform};

use crate::{Layer, LayerKind, MAX_CANVAS_DIMENSION};

pub const PATH_GEOMETRY_VERSION: u32 = 1;
pub const MAX_PATH_ANCHORS: usize = 256;
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathAnchor {
    pub point: [f32; 2],
    /// Cubic control point offset entering this anchor.
    #[serde(default)]
    pub handle_in: [f32; 2],
    /// Cubic control point offset leaving this anchor.
    #[serde(default)]
    pub handle_out: [f32; 2],
}

impl PathAnchor {
    pub fn corner(x: f32, y: f32) -> Self {
        Self {
            point: [x, y],
            handle_in: [0.0; 2],
            handle_out: [0.0; 2],
        }
    }

    pub fn incoming(self) -> [f32; 2] {
        [
            self.point[0] + self.handle_in[0],
            self.point[1] + self.handle_in[1],
        ]
    }

    pub fn outgoing(self) -> [f32; 2] {
        [
            self.point[0] + self.handle_out[0],
            self.point[1] + self.handle_out[1],
        ]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathFillRule {
    #[default]
    EvenOdd,
}

/// Bounded, portable cubic path geometry shared by path layers and vector masks.
///
/// Coordinates are local to the explicit `width` by `height` viewport. Cubic
/// handles are relative to their anchor. Runtime identity is recomputed during
/// bounded deserialization, so serialized payloads cannot forge cache identity.
#[derive(Clone)]
pub struct PathGeometry {
    version: u32,
    width: u32,
    height: u32,
    closed: bool,
    fill_rule: PathFillRule,
    anchors: Arc<[PathAnchor]>,
    identity: [u8; 32],
}

impl PathGeometry {
    pub fn new(
        width: u32,
        height: u32,
        closed: bool,
        fill_rule: PathFillRule,
        anchors: impl Into<Arc<[PathAnchor]>>,
    ) -> Result<Self> {
        let anchors = anchors.into();
        validate_geometry(width, height, closed, &anchors)?;
        let identity = geometry_identity(width, height, closed, fill_rule, &anchors);
        Ok(Self {
            version: PATH_GEOMETRY_VERSION,
            width,
            height,
            closed,
            fill_rule,
            anchors,
            identity,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn closed(&self) -> bool {
        self.closed
    }

    pub fn fill_rule(&self) -> PathFillRule {
        self.fill_rule
    }

    pub fn anchors(&self) -> &[PathAnchor] {
        &self.anchors
    }

    pub fn identity(&self) -> [u8; 32] {
        self.identity
    }

    pub fn replacing_anchor(&self, index: usize, anchor: PathAnchor) -> Result<Self> {
        if index >= self.anchors.len() {
            bail!("path anchor index {index} is out of range");
        }
        let mut anchors = self.anchors.to_vec();
        anchors[index] = anchor;
        Self::new(
            self.width,
            self.height,
            self.closed,
            self.fill_rule,
            anchors,
        )
    }

    pub fn is_fill_degenerate(&self) -> bool {
        if !self.closed || self.anchors.len() < 3 {
            return true;
        }
        let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
        let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for anchor in self.anchors.iter() {
            for point in [anchor.point, anchor.incoming(), anchor.outgoing()] {
                min_x = min_x.min(point[0]);
                min_y = min_y.min(point[1]);
                max_x = max_x.max(point[0]);
                max_y = max_y.max(point[1]);
            }
        }
        max_x - min_x <= 0.001 || max_y - min_y <= 0.001
    }
}

impl fmt::Debug for PathGeometry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PathGeometry")
            .field("version", &self.version)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("closed", &self.closed)
            .field("fill_rule", &self.fill_rule)
            .field("anchors", &self.anchors)
            .field("identity", &self.identity)
            .finish()
    }
}

impl PartialEq for PathGeometry {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.version == other.version
            && self.width == other.width
            && self.height == other.height
            && self.closed == other.closed
            && self.fill_rule == other.fill_rule
            && (Arc::ptr_eq(&self.anchors, &other.anchors)
                || self.anchors.as_ref() == other.anchors.as_ref())
    }
}

impl Eq for PathGeometry {}

impl Serialize for PathGeometry {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("PathGeometry", 6)?;
        state.serialize_field("version", &self.version)?;
        state.serialize_field("width", &self.width)?;
        state.serialize_field("height", &self.height)?;
        state.serialize_field("closed", &self.closed)?;
        state.serialize_field("fill_rule", &self.fill_rule)?;
        state.serialize_field("anchors", &AnchorsRef(&self.anchors))?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for PathGeometry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            version: u32,
            width: u32,
            height: u32,
            closed: bool,
            #[serde(default)]
            fill_rule: PathFillRule,
            anchors: BoundedAnchors,
        }

        let wire = Wire::deserialize(deserializer)?;
        if wire.version != PATH_GEOMETRY_VERSION {
            return Err(serde::de::Error::custom(format!(
                "unsupported path geometry version {}",
                wire.version
            )));
        }
        Self::new(
            wire.width,
            wire.height,
            wire.closed,
            wire.fill_rule,
            wire.anchors.0,
        )
        .map_err(serde::de::Error::custom)
    }
}

struct AnchorsRef<'a>(&'a [PathAnchor]);

impl Serialize for AnchorsRef<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

struct BoundedAnchors(Vec<PathAnchor>);

impl<'de> Deserialize<'de> for BoundedAnchors {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct AnchorsVisitor;

        impl<'de> Visitor<'de> for AnchorsVisitor {
            type Value = BoundedAnchors;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "at most {MAX_PATH_ANCHORS} path anchors")
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                if sequence
                    .size_hint()
                    .is_some_and(|size| size > MAX_PATH_ANCHORS)
                {
                    return Err(serde::de::Error::custom("path exceeds the anchor limit"));
                }
                let mut anchors = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
                while let Some(anchor) = sequence.next_element()? {
                    if anchors.len() == MAX_PATH_ANCHORS {
                        return Err(serde::de::Error::custom("path exceeds the anchor limit"));
                    }
                    anchors.push(anchor);
                }
                Ok(BoundedAnchors(anchors))
            }
        }

        deserializer.deserialize_seq(AnchorsVisitor)
    }
}

fn validate_geometry(width: u32, height: u32, closed: bool, anchors: &[PathAnchor]) -> Result<()> {
    if width == 0 || height == 0 || width > MAX_CANVAS_DIMENSION || height > MAX_CANVAS_DIMENSION {
        bail!("path viewport must be within Prism's canvas dimension limit");
    }
    let minimum = if closed { 3 } else { 2 };
    if !(minimum..=MAX_PATH_ANCHORS).contains(&anchors.len()) {
        bail!(
            "{} path requires between {minimum} and {MAX_PATH_ANCHORS} anchors",
            if closed { "closed" } else { "open" }
        );
    }
    for (index, anchor) in anchors.iter().copied().enumerate() {
        for (label, point) in [
            ("anchor", anchor.point),
            ("incoming control", anchor.incoming()),
            ("outgoing control", anchor.outgoing()),
        ] {
            if point.iter().any(|value| !value.is_finite()) {
                bail!("path {label} {index} must contain finite coordinates");
            }
            if point[0] < 0.0
                || point[1] < 0.0
                || point[0] > width as f32
                || point[1] > height as f32
            {
                bail!("path {label} {index} must stay inside its explicit viewport");
            }
        }
    }
    Ok(())
}

fn geometry_identity(
    width: u32,
    height: u32,
    closed: bool,
    fill_rule: PathFillRule,
    anchors: &[PathAnchor],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(PATH_GEOMETRY_VERSION.to_le_bytes());
    digest.update(width.to_le_bytes());
    digest.update(height.to_le_bytes());
    digest.update([closed as u8, fill_rule as u8]);
    digest.update((anchors.len() as u32).to_le_bytes());
    for anchor in anchors {
        for point in [anchor.point, anchor.handle_in, anchor.handle_out] {
            digest.update(point[0].to_bits().to_le_bytes());
            digest.update(point[1].to_bits().to_le_bytes());
        }
    }
    digest.finalize().into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VectorMask {
    pub enabled: bool,
    pub invert: bool,
    pub path: PathGeometry,
}

impl VectorMask {
    pub fn new(path: PathGeometry, invert: bool) -> Result<Self> {
        if path.is_fill_degenerate() {
            bail!("vector masks require a nondegenerate closed path");
        }
        Ok(Self {
            enabled: true,
            invert,
            path,
        })
    }

    pub fn identity(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(self.path.identity());
        digest.update([self.enabled as u8, self.invert as u8]);
        digest.finalize().into()
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.path.is_fill_degenerate() {
            bail!("vector masks require a nondegenerate closed path");
        }
        Ok(())
    }
}

const PATH_AA_PADDING: f32 = 1.0;
const PATH_RASTER_TILE: u32 = 256;
pub(crate) const MAX_PATH_RASTER_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_VECTOR_MASK_TILE_PIXELS: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathSourceBounds {
    pub origin: [f32; 2],
    pub size: [f32; 2],
    pub viewport: [f32; 2],
}

impl PathSourceBounds {
    pub fn raster_dimensions(self, scale: [f32; 2]) -> Result<(u32, u32)> {
        if scale
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            bail!("path render scale must contain positive finite numbers");
        }
        let width = (self.size[0] * scale[0]).ceil().max(1.0) as u32;
        let height = (self.size[1] * scale[1]).ceil().max(1.0) as u32;
        if width > MAX_CANVAS_DIMENSION || height > MAX_CANVAS_DIMENSION {
            bail!("path render exceeds Prism's maximum raster dimension");
        }
        Ok((width, height))
    }
}

/// Logical source bounds for a path layer. `Transform.x/y` always names the
/// path viewport origin; centered stroke padding is represented by a negative
/// source origin so changing stroke width cannot move anchors or the pivot.
pub fn path_source_bounds(layer: &Layer) -> Option<PathSourceBounds> {
    let LayerKind::Path { geometry, .. } = &layer.kind else {
        return None;
    };
    let padding = if layer.stroke.enabled && layer.stroke.width > 0.0 {
        (layer.stroke.width * 0.5 + PATH_AA_PADDING).ceil()
    } else {
        0.0
    };
    Some(PathSourceBounds {
        origin: [-padding; 2],
        size: [
            geometry.width() as f32 + padding * 2.0,
            geometry.height() as f32 + padding * 2.0,
        ],
        viewport: [geometry.width() as f32, geometry.height() as f32],
    })
}

pub(crate) fn render_path(layer: &Layer, scale: [f32; 2]) -> Result<RgbaImage> {
    let bounds = path_source_bounds(layer).ok_or_else(|| anyhow::anyhow!("layer is not a path"))?;
    let (width, height) = bounds.raster_dimensions(scale)?;
    render_path_region(layer, scale, 0, 0, width, height)
}

/// Renders a global source-raster tile. The path transform includes the tile
/// origin rather than translating a pre-rasterized surface, keeping AA seam
/// samples identical between full export and bounded region rendering.
pub(crate) fn render_path_region(
    layer: &Layer,
    scale: [f32; 2],
    region_x: u32,
    region_y: u32,
    region_width: u32,
    region_height: u32,
) -> Result<RgbaImage> {
    let LayerKind::Path { .. } = &layer.kind else {
        bail!("layer is not a path");
    };
    let bounds = path_source_bounds(layer).expect("path kind has path bounds");
    let (full_width, full_height) = bounds.raster_dimensions(scale)?;
    let right = region_x
        .checked_add(region_width)
        .ok_or_else(|| anyhow::anyhow!("path render region overflows"))?;
    let bottom = region_y
        .checked_add(region_height)
        .ok_or_else(|| anyhow::anyhow!("path render region overflows"))?;
    if region_width == 0 || region_height == 0 || right > full_width || bottom > full_height {
        bail!("path render region must be nonempty and inside its source raster");
    }
    let pixels = u64::from(region_width) * u64::from(region_height);
    if pixels > MAX_PATH_RASTER_PIXELS {
        bail!("path render exceeds the bounded raster area");
    }

    let mut output = RgbaImage::new(region_width, region_height);
    let tile_left = region_x / PATH_RASTER_TILE * PATH_RASTER_TILE;
    let tile_top = region_y / PATH_RASTER_TILE * PATH_RASTER_TILE;
    let tile_right = right.div_ceil(PATH_RASTER_TILE) * PATH_RASTER_TILE;
    let tile_bottom = bottom.div_ceil(PATH_RASTER_TILE) * PATH_RASTER_TILE;
    for tile_y in (tile_top..tile_bottom).step_by(PATH_RASTER_TILE as usize) {
        for tile_x in (tile_left..tile_right).step_by(PATH_RASTER_TILE as usize) {
            let tile_width = PATH_RASTER_TILE.min(full_width - tile_x);
            let tile_height = PATH_RASTER_TILE.min(full_height - tile_y);
            let tile = render_path_tile(layer, scale, tile_x, tile_y, tile_width, tile_height)?;
            let copy_left = region_x.max(tile_x);
            let copy_top = region_y.max(tile_y);
            let copy_right = right.min(tile_x + tile_width);
            let copy_bottom = bottom.min(tile_y + tile_height);
            for global_y in copy_top..copy_bottom {
                for global_x in copy_left..copy_right {
                    output.put_pixel(
                        global_x - region_x,
                        global_y - region_y,
                        *tile.get_pixel(global_x - tile_x, global_y - tile_y),
                    );
                }
            }
        }
    }
    Ok(output)
}

fn render_path_tile(
    layer: &Layer,
    scale: [f32; 2],
    tile_x: u32,
    tile_y: u32,
    tile_width: u32,
    tile_height: u32,
) -> Result<RgbaImage> {
    let LayerKind::Path { geometry, color } = &layer.kind else {
        bail!("layer is not a path");
    };
    let bounds = path_source_bounds(layer).expect("path kind has path bounds");
    let path = tiny_path(geometry)?;
    let transform = Transform::from_row(
        scale[0],
        0.0,
        0.0,
        scale[1],
        -bounds.origin[0] * scale[0] - tile_x as f32,
        -bounds.origin[1] * scale[1] - tile_y as f32,
    );
    let mut fill_mask = Mask::new(tile_width, tile_height)
        .ok_or_else(|| anyhow::anyhow!("could not allocate path fill mask"))?;
    if geometry.closed() {
        fill_mask.fill_path(&path, FillRule::EvenOdd, true, transform);
    }
    let stroke_mask = if layer.stroke.enabled && layer.stroke.width > 0.0 {
        let stroke = Stroke {
            width: layer.stroke.width,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        let stroked = path
            .stroke(&stroke, scale[0].max(scale[1]))
            .ok_or_else(|| anyhow::anyhow!("could not expand path stroke"))?;
        let mut mask = Mask::new(tile_width, tile_height)
            .ok_or_else(|| anyhow::anyhow!("could not allocate path stroke mask"))?;
        mask.fill_path(&stroked, FillRule::Winding, true, transform);
        Some(mask)
    } else {
        None
    };
    let direction = layer.shape_fill.as_ref().map(|fill| fill.direction());
    Ok(RgbaImage::from_fn(tile_width, tile_height, |x, y| {
        let global_x = tile_x + x;
        let global_y = tile_y + y;
        let local_x = (global_x as f32 + 0.5) / scale[0] + bounds.origin[0];
        let local_y = (global_y as f32 + 0.5) / scale[1] + bounds.origin[1];
        let fill_color = layer
            .shape_fill
            .as_ref()
            .map(|fill| {
                fill.sample(
                    local_x,
                    local_y,
                    geometry.width(),
                    geometry.height(),
                    direction.unwrap_or((1.0, 0.0)),
                )
            })
            .unwrap_or(*color);
        let index = (u64::from(y) * u64::from(tile_width) + u64::from(x)) as usize;
        let fill = covered_color(fill_color, fill_mask.data()[index]);
        let stroke = stroke_mask
            .as_ref()
            .map(|mask| covered_color(layer.stroke.color, mask.data()[index]))
            .unwrap_or([0; 4]);
        Rgba(source_over(stroke, fill))
    }))
}

pub fn apply_vector_mask_to_image(
    image: &mut RgbaImage,
    vector_mask: Option<&VectorMask>,
    full_width: u32,
    full_height: u32,
    region_x: u32,
    region_y: u32,
) -> Result<()> {
    let Some(vector_mask) = vector_mask.filter(|mask| mask.enabled) else {
        return Ok(());
    };
    if image.width() == 0 || image.height() == 0 || full_width == 0 || full_height == 0 {
        bail!("vector mask target must have positive dimensions");
    }
    let right = region_x
        .checked_add(image.width())
        .ok_or_else(|| anyhow::anyhow!("vector mask region overflows"))?;
    let bottom = region_y
        .checked_add(image.height())
        .ok_or_else(|| anyhow::anyhow!("vector mask region overflows"))?;
    if right > full_width || bottom > full_height {
        bail!("vector mask region exceeds the target source");
    }
    let pixels = u64::from(image.width()) * u64::from(image.height());
    if pixels > MAX_VECTOR_MASK_TILE_PIXELS {
        bail!("vector mask alpha tile exceeds the 64 MiB limit");
    }
    let path = tiny_path(&vector_mask.path)?;
    let transform = Transform::from_row(
        full_width as f32 / vector_mask.path.width() as f32,
        0.0,
        0.0,
        full_height as f32 / vector_mask.path.height() as f32,
        -(region_x as f32),
        -(region_y as f32),
    );
    let mut alpha = Mask::new(image.width(), image.height())
        .ok_or_else(|| anyhow::anyhow!("could not allocate vector mask alpha tile"))?;
    alpha.fill_path(&path, FillRule::EvenOdd, true, transform);
    for (pixel, coverage) in image.pixels_mut().zip(alpha.data().iter().copied()) {
        let coverage = if vector_mask.invert {
            255 - coverage
        } else {
            coverage
        };
        pixel[3] = multiply_alpha(pixel[3], coverage);
    }
    Ok(())
}

fn tiny_path(geometry: &PathGeometry) -> Result<tiny_skia::Path> {
    let anchors = geometry.anchors();
    let first = anchors[0];
    let mut builder = PathBuilder::new();
    builder.move_to(first.point[0], first.point[1]);
    let segment_count = if geometry.closed() {
        anchors.len()
    } else {
        anchors.len() - 1
    };
    for index in 0..segment_count {
        let start = anchors[index];
        let end = anchors[(index + 1) % anchors.len()];
        let control_1 = start.outgoing();
        let control_2 = end.incoming();
        builder.cubic_to(
            control_1[0],
            control_1[1],
            control_2[0],
            control_2[1],
            end.point[0],
            end.point[1],
        );
    }
    if geometry.closed() {
        builder.close();
    }
    builder
        .finish()
        .ok_or_else(|| anyhow::anyhow!("path geometry produced no drawable contour"))
}

fn covered_color(mut color: [u8; 4], coverage: u8) -> [u8; 4] {
    color[3] = multiply_alpha(color[3], coverage);
    color
}

fn multiply_alpha(left: u8, right: u8) -> u8 {
    ((u16::from(left) * u16::from(right) + 127) / 255) as u8
}

fn source_over(source: [u8; 4], destination: [u8; 4]) -> [u8; 4] {
    let source_alpha = source[3] as f32 / 255.0;
    let destination_alpha = destination[3] as f32 / 255.0;
    let output_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    if output_alpha <= f32::EPSILON {
        return [0; 4];
    }
    let mut output = [0; 4];
    for channel in 0..3 {
        output[channel] = ((source[channel] as f32 * source_alpha
            + destination[channel] as f32 * destination_alpha * (1.0 - source_alpha))
            / output_alpha)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    output[3] = (output_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> PathGeometry {
        PathGeometry::new(
            100,
            80,
            true,
            PathFillRule::EvenOdd,
            vec![
                PathAnchor::corner(10.0, 70.0),
                PathAnchor::corner(50.0, 10.0),
                PathAnchor::corner(90.0, 70.0),
            ],
        )
        .unwrap()
    }

    #[test]
    fn identity_covers_viewport_flags_and_handles() {
        let base = triangle();
        let moved = base
            .replacing_anchor(
                1,
                PathAnchor {
                    handle_out: [4.0, 2.0],
                    ..base.anchors()[1]
                },
            )
            .unwrap();
        assert_ne!(base.identity(), moved.identity());
        let open = PathGeometry::new(
            100,
            80,
            false,
            PathFillRule::EvenOdd,
            base.anchors().to_vec(),
        )
        .unwrap();
        assert_ne!(base.identity(), open.identity());
    }

    #[test]
    fn serde_recomputes_identity_and_rejects_oversized_sequences() {
        let geometry = triangle();
        let reopened: PathGeometry =
            serde_json::from_str(&serde_json::to_string(&geometry).unwrap()).unwrap();
        assert_eq!(reopened, geometry);
        let anchors = vec![PathAnchor::corner(1.0, 1.0); MAX_PATH_ANCHORS + 1];
        let value = serde_json::json!({
            "version": 1,
            "width": 10,
            "height": 10,
            "closed": true,
            "fill_rule": "even_odd",
            "anchors": anchors,
        });
        assert!(serde_json::from_value::<PathGeometry>(value).is_err());
    }

    #[test]
    fn malformed_and_open_masks_fail_closed() {
        let open = PathGeometry::new(
            10,
            10,
            false,
            PathFillRule::EvenOdd,
            vec![PathAnchor::corner(0.0, 0.0), PathAnchor::corner(10.0, 10.0)],
        )
        .unwrap();
        assert!(VectorMask::new(open, false).is_err());
        assert!(
            PathGeometry::new(
                10,
                10,
                false,
                PathFillRule::EvenOdd,
                vec![
                    PathAnchor::corner(0.0, 0.0),
                    PathAnchor::corner(f32::NAN, 4.0)
                ],
            )
            .is_err()
        );
    }
}
