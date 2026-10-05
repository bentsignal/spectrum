use anyhow::{Result, bail};
use spectrum_imaging::Adjustments;

use crate::{LayerMask, ShapeStroke, Transform};

pub(super) fn require_finite(label: &str, value: f32) -> Result<()> {
    if !value.is_finite() {
        bail!("{label} must be a finite number");
    }
    Ok(())
}

pub(super) fn validate_transform(transform: Transform) -> Result<()> {
    for (label, value) in [
        ("x", transform.x),
        ("y", transform.y),
        ("horizontal scale", transform.scale_x),
        ("vertical scale", transform.scale_y),
        ("rotation", transform.rotation),
    ] {
        require_finite(label, value)?;
    }
    Ok(())
}

pub(super) fn validate_mask(mask: LayerMask) -> Result<()> {
    for (label, value) in [
        ("mask x", mask.x),
        ("mask y", mask.y),
        ("mask width", mask.width),
        ("mask height", mask.height),
    ] {
        require_finite(label, value)?;
    }
    Ok(())
}

pub(super) fn validate_shape_stroke(stroke: ShapeStroke) -> Result<()> {
    require_finite("shape stroke width", stroke.width)
}

pub(super) fn validate_adjustments(value: &Adjustments) -> Result<()> {
    for (label, number) in [
        ("exposure", value.exposure),
        ("temperature", value.temperature),
        ("tint", value.tint),
        ("contrast", value.contrast),
        ("highlights", value.highlights),
        ("shadows", value.shadows),
        ("whites", value.whites),
        ("blacks", value.blacks),
        ("texture", value.texture),
        ("clarity", value.clarity),
        ("dehaze", value.dehaze),
        ("vibrance", value.vibrance),
        ("saturation", value.saturation),
        ("vignette", value.vignette),
        ("sharpening", value.sharpening),
        ("noise reduction", value.noise_reduction),
        ("straighten", value.straighten),
        ("color balance", value.color_grading.balance),
    ] {
        require_finite(label, number)?;
    }
    if let Some(crop) = value.crop {
        for (label, number) in [
            ("crop x", crop.x),
            ("crop y", crop.y),
            ("crop width", crop.width),
            ("crop height", crop.height),
        ] {
            require_finite(label, number)?;
        }
    }
    for band in value.hsl.bands() {
        require_finite("HSL hue", band.hue)?;
        require_finite("HSL saturation", band.saturation)?;
        require_finite("HSL luminance", band.luminance)?;
    }
    for curve in [
        &value.curves.master,
        &value.curves.red,
        &value.curves.green,
        &value.curves.blue,
    ] {
        for point in &curve.points {
            require_finite("curve x", point.x)?;
            require_finite("curve y", point.y)?;
        }
    }
    for grade in [
        value.color_grading.shadows,
        value.color_grading.midtones,
        value.color_grading.highlights,
    ] {
        require_finite("grade hue", grade.hue)?;
        require_finite("grade saturation", grade.saturation)?;
        require_finite("grade luminance", grade.luminance)?;
    }
    for spot in &value.spots {
        require_finite("spot x", spot.x)?;
        require_finite("spot y", spot.y)?;
        require_finite("spot radius", spot.radius)?;
        require_finite("spot opacity", spot.opacity)?;
    }
    Ok(())
}
