use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{Document, Layer, LayerKind, Transform, require_finite};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuideOrientation {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub id: u64,
    pub orientation: GuideOrientation,
    pub position: f32,
}

impl Guide {
    pub(crate) fn sanitize(&mut self, width: u32, height: u32) -> Result<()> {
        require_finite("guide position", self.position)?;
        self.position = self
            .position
            .clamp(0.0, self.orientation.extent(width, height));
        Ok(())
    }
}

impl GuideOrientation {
    pub(crate) fn extent(self, width: u32, height: u32) -> f32 {
        match self {
            Self::Horizontal => height as f32,
            Self::Vertical => width as f32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    Left,
    HorizontalCenter,
    Right,
    Top,
    VerticalCenter,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AlignmentReference {
    Canvas,
    Layer { id: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerGeometry {
    pub corners: [[f32; 2]; 4],
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub center: [f32; 2],
}

impl LayerGeometry {
    pub fn width(self) -> f32 {
        self.max[0] - self.min[0]
    }

    pub fn height(self) -> f32 {
        self.max[1] - self.min[1]
    }
}

pub fn layer_geometry(layer: &Layer) -> Result<LayerGeometry> {
    let (source_origin, source_size) = layer_source_bounds(layer)?;
    Ok(layer_geometry_with_bounds(
        layer,
        source_origin,
        source_size,
    ))
}

pub fn layer_geometry_with_size(layer: &Layer, source_size: [f32; 2]) -> LayerGeometry {
    layer_geometry_with_bounds(layer, [0.0, 0.0], source_size)
}

pub fn layer_geometry_with_bounds(
    layer: &Layer,
    source_origin: [f32; 2],
    source_size: [f32; 2],
) -> LayerGeometry {
    let width = source_size[0] * layer.transform.scale_x;
    let height = source_size[1] * layer.transform.scale_y;
    let center = [
        layer.transform.x + (source_origin[0] + source_size[0] * 0.5) * layer.transform.scale_x,
        layer.transform.y + (source_origin[1] + source_size[1] * 0.5) * layer.transform.scale_y,
    ];
    let radians = layer.transform.rotation.to_radians();
    let (sin, cos) = radians.sin_cos();
    let mut corners = [[0.0; 2]; 4];
    for (corner, [x, y]) in corners.iter_mut().zip([
        [-width * 0.5, -height * 0.5],
        [width * 0.5, -height * 0.5],
        [width * 0.5, height * 0.5],
        [-width * 0.5, height * 0.5],
    ]) {
        *corner = [center[0] + x * cos - y * sin, center[1] + x * sin + y * cos];
    }
    let min = [
        corners
            .iter()
            .map(|corner| corner[0])
            .fold(f32::INFINITY, f32::min),
        corners
            .iter()
            .map(|corner| corner[1])
            .fold(f32::INFINITY, f32::min),
    ];
    let max = [
        corners
            .iter()
            .map(|corner| corner[0])
            .fold(f32::NEG_INFINITY, f32::max),
        corners
            .iter()
            .map(|corner| corner[1])
            .fold(f32::NEG_INFINITY, f32::max),
    ];
    LayerGeometry {
        corners,
        min,
        max,
        center,
    }
}

pub fn align_layer_transform(
    document: &Document,
    id: u64,
    alignment: Alignment,
    reference: AlignmentReference,
) -> Result<Transform> {
    let layer = document.layer(id)?;
    if layer.locked {
        bail!("layer {id} is locked");
    }
    let moving = layer_geometry(layer)?;
    let reference = match reference {
        AlignmentReference::Canvas => LayerGeometry {
            corners: [
                [0.0, 0.0],
                [document.width as f32, 0.0],
                [document.width as f32, document.height as f32],
                [0.0, document.height as f32],
            ],
            min: [0.0, 0.0],
            max: [document.width as f32, document.height as f32],
            center: [document.width as f32 * 0.5, document.height as f32 * 0.5],
        },
        AlignmentReference::Layer { id: reference_id } => {
            if reference_id == id {
                bail!("a layer cannot be aligned to itself");
            }
            layer_geometry(document.layer(reference_id)?)?
        }
    };
    let delta = match alignment {
        Alignment::Left => [reference.min[0] - moving.min[0], 0.0],
        Alignment::HorizontalCenter => [reference.center[0] - moving.center[0], 0.0],
        Alignment::Right => [reference.max[0] - moving.max[0], 0.0],
        Alignment::Top => [0.0, reference.min[1] - moving.min[1]],
        Alignment::VerticalCenter => [0.0, reference.center[1] - moving.center[1]],
        Alignment::Bottom => [0.0, reference.max[1] - moving.max[1]],
    };
    let mut transform = layer.transform;
    transform.x += delta[0];
    transform.y += delta[1];
    Ok(transform)
}

pub(crate) fn add_guide(
    document: &mut Document,
    orientation: GuideOrientation,
    position: f32,
) -> Result<u64> {
    require_finite("guide position", position)?;
    let id = document.next_guide_id;
    document.next_guide_id += 1;
    document.guides.push(Guide {
        id,
        orientation,
        position: position.clamp(0.0, orientation.extent(document.width, document.height)),
    });
    Ok(id)
}

pub(crate) fn move_guide(document: &mut Document, id: u64, position: f32) -> Result<()> {
    require_finite("guide position", position)?;
    let (width, height) = (document.width, document.height);
    let guide = document
        .guides
        .iter_mut()
        .find(|guide| guide.id == id)
        .with_context(|| format!("guide {id} is not in this document"))?;
    guide.position = position.clamp(0.0, guide.orientation.extent(width, height));
    Ok(())
}

pub(crate) fn remove_guide(document: &mut Document, id: u64) -> Result<()> {
    let index = document
        .guides
        .iter()
        .position(|guide| guide.id == id)
        .with_context(|| format!("guide {id} is not in this document"))?;
    document.guides.remove(index);
    Ok(())
}

pub(crate) fn clamp_guides(document: &mut Document) {
    for guide in &mut document.guides {
        guide.position = guide.position.clamp(
            0.0,
            guide.orientation.extent(document.width, document.height),
        );
    }
}

pub(crate) fn crop_guides(document: &mut Document, x: u32, y: u32) {
    for guide in &mut document.guides {
        guide.position -= match guide.orientation {
            GuideOrientation::Horizontal => y as f32,
            GuideOrientation::Vertical => x as f32,
        };
    }
    document.guides.retain(|guide| {
        guide.position >= 0.0
            && guide.position <= guide.orientation.extent(document.width, document.height)
    });
}

fn layer_source_bounds(layer: &Layer) -> Result<([f32; 2], [f32; 2])> {
    match &layer.kind {
        LayerKind::Raster { path, .. } => {
            let (width, height) = image::image_dimensions(path)
                .with_context(|| format!("could not inspect layer source {}", path.display()))?;
            Ok(([0.0, 0.0], [width as f32, height as f32]))
        }
        LayerKind::Text {
            text, font_size, ..
        } => {
            let geometry = crate::measure_text_geometry(text, *font_size)?;
            Ok((
                [geometry.visual_left, geometry.visual_top],
                [geometry.visual_width, geometry.visual_height],
            ))
        }
        LayerKind::Rectangle { width, height, .. } | LayerKind::Ellipse { width, height, .. } => {
            Ok(([0.0, 0.0], [*width as f32, *height as f32]))
        }
    }
}
