use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use prism_core::{
    Command, DropShadow, GradientInterpolation, GradientKind, GradientSpread, GradientStop,
    LayerStyle, ShapeFill, ShapeGradient,
};
use serde::Deserialize;

const MAX_STRUCTURED_GRADIENT_BYTES: usize = 16 * 1024;

#[derive(Args, Debug)]
pub(super) struct ShadowArgs {
    pub id: u64,
    /// Remove the drop shadow from this layer.
    #[arg(long)]
    pub clear: bool,
    #[arg(long, default_value_t = 12.0, allow_negative_numbers = true)]
    pub x: f32,
    #[arg(long, default_value_t = 12.0, allow_negative_numbers = true)]
    pub y: f32,
    #[arg(long, default_value_t = 10.0)]
    pub blur: f32,
    #[arg(long, default_value = "000000a0")]
    pub color: String,
}

#[derive(Args, Debug)]
pub(super) struct GradientArgs {
    pub id: u64,
    /// Return this shape to its solid legacy color.
    #[arg(
        long,
        conflicts_with_all = [
            "gradient_json",
            "angle",
            "kind",
            "spread",
            "center_x",
            "center_y",
            "radius",
            "offset",
            "extent",
            "stops",
            "start",
            "end"
        ]
    )]
    pub clear: bool,
    /// Bounded strict JSON gradient object; mutually exclusive with all flags.
    #[arg(long, conflicts_with_all = ["clear", "angle", "kind", "spread", "center_x", "center_y", "radius", "offset", "extent", "stops", "start", "end"])]
    gradient_json: Option<String>,
    #[arg(long, allow_negative_numbers = true)]
    pub angle: Option<f32>,
    /// Gradient geometry.
    #[arg(long, value_enum)]
    kind: Option<GradientKindArg>,
    /// Behavior beyond the first and last stop.
    #[arg(long, value_enum)]
    spread: Option<GradientSpreadArg>,
    #[arg(long)]
    center_x: Option<f32>,
    #[arg(long)]
    center_y: Option<f32>,
    #[arg(long)]
    radius: Option<f32>,
    /// Scalar origin for Linear and Angle geometry.
    #[arg(long, allow_negative_numbers = true)]
    offset: Option<f32>,
    /// Positive scalar span for Linear and Angle geometry.
    #[arg(long)]
    extent: Option<f32>,
    /// Ordered POSITION:RRGGBBAA stop. Repeat for 2 through 32 stops.
    #[arg(long = "stop", conflicts_with_all = ["start", "end"])]
    stops: Vec<String>,
    /// Legacy first color. Mutually exclusive with --stop.
    #[arg(long, conflicts_with = "stops")]
    pub start: Option<String>,
    /// Legacy last color. Mutually exclusive with --stop.
    #[arg(long, conflicts_with = "stops")]
    pub end: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuredGradient {
    kind: GradientKind,
    #[serde(default)]
    angle: f32,
    stops: Vec<StructuredGradientStop>,
    #[serde(default)]
    center: Option<[f32; 2]>,
    #[serde(default)]
    radius: Option<f32>,
    #[serde(default)]
    spread: GradientSpread,
    interpolation: GradientInterpolation,
    #[serde(default)]
    offset: f32,
    #[serde(default)]
    extent: Option<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuredGradientStop {
    position: f32,
    color: [u8; 4],
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum GradientKindArg {
    #[default]
    Linear,
    Radial,
    Angle,
}

impl From<GradientKindArg> for GradientKind {
    fn from(value: GradientKindArg) -> Self {
        match value {
            GradientKindArg::Linear => Self::Linear,
            GradientKindArg::Radial => Self::Radial,
            GradientKindArg::Angle => Self::Angle,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum GradientSpreadArg {
    #[default]
    Pad,
    Repeat,
    Reflect,
}

impl From<GradientSpreadArg> for GradientSpread {
    fn from(value: GradientSpreadArg) -> Self {
        match value {
            GradientSpreadArg::Pad => Self::Pad,
            GradientSpreadArg::Repeat => Self::Repeat,
            GradientSpreadArg::Reflect => Self::Reflect,
        }
    }
}

pub(super) fn shadow_command(arguments: ShadowArgs) -> Result<Command> {
    let drop_shadow = if arguments.clear {
        None
    } else {
        Some(DropShadow {
            color: parse_color(&arguments.color)?,
            offset_x: arguments.x,
            offset_y: arguments.y,
            blur_radius: arguments.blur,
        })
    };
    Ok(Command::SetLayerStyle {
        id: arguments.id,
        style: LayerStyle { drop_shadow },
    })
}

pub(super) fn gradient_command(arguments: GradientArgs) -> Result<Command> {
    let flag_surface = arguments.angle.is_some()
        || arguments.kind.is_some()
        || arguments.spread.is_some()
        || arguments.center_x.is_some()
        || arguments.center_y.is_some()
        || arguments.radius.is_some()
        || arguments.offset.is_some()
        || arguments.extent.is_some()
        || !arguments.stops.is_empty()
        || arguments.start.is_some()
        || arguments.end.is_some();
    if arguments.clear && (arguments.gradient_json.is_some() || flag_surface) {
        bail!("--clear cannot be combined with gradient values");
    }
    if arguments.gradient_json.is_some() && flag_surface {
        bail!("--gradient-json cannot be combined with gradient flags");
    }
    let fill = if arguments.clear {
        None
    } else if let Some(json) = arguments.gradient_json.as_deref() {
        Some(ShapeFill::Gradient(parse_structured_gradient(json)?))
    } else {
        if !arguments.stops.is_empty() && (arguments.start.is_some() || arguments.end.is_some()) {
            bail!("--stop cannot be combined with legacy --start or --end");
        }
        let stops = if arguments.stops.is_empty() {
            vec![
                GradientStop::new(
                    0.0,
                    parse_color(arguments.start.as_deref().unwrap_or("5dd8c7ff"))?,
                ),
                GradientStop::new(
                    1.0,
                    parse_color(arguments.end.as_deref().unwrap_or("ae7bffff"))?,
                ),
            ]
        } else {
            arguments
                .stops
                .iter()
                .map(|value| parse_stop(value))
                .collect::<Result<Vec<_>>>()?
        };
        Some(ShapeFill::Gradient(ShapeGradient {
            kind: arguments.kind.unwrap_or_default().into(),
            angle: arguments.angle.unwrap_or(0.0),
            stops,
            center: [
                arguments.center_x.unwrap_or(0.5),
                arguments.center_y.unwrap_or(0.5),
            ],
            radius: arguments.radius.unwrap_or(0.5),
            spread: arguments.spread.unwrap_or_default().into(),
            interpolation: GradientInterpolation::PremultipliedSrgbV1,
            offset: arguments.offset.unwrap_or(0.0),
            extent: arguments.extent.unwrap_or(1.0),
        }))
    };
    Ok(Command::SetShapeFill {
        id: arguments.id,
        fill,
    })
}

fn parse_structured_gradient(json: &str) -> Result<ShapeGradient> {
    if json.len() > MAX_STRUCTURED_GRADIENT_BYTES {
        bail!("structured gradient exceeds the 16 KiB input limit");
    }
    let structured: StructuredGradient =
        serde_json::from_str(json).context("invalid structured gradient JSON")?;
    let gradient = ShapeGradient {
        kind: structured.kind,
        angle: structured.angle,
        stops: structured
            .stops
            .into_iter()
            .map(|stop| GradientStop::new(stop.position, stop.color))
            .collect(),
        center: structured.center.unwrap_or([0.5, 0.5]),
        radius: structured.radius.unwrap_or(0.5),
        spread: structured.spread,
        interpolation: structured.interpolation,
        offset: structured.offset,
        extent: structured.extent.unwrap_or(1.0),
    };
    gradient.validate().map_err(anyhow::Error::new)?;
    Ok(gradient)
}

fn parse_stop(value: &str) -> Result<GradientStop> {
    let (position, color) = value
        .split_once(':')
        .context("gradient stop must use POSITION:RRGGBBAA")?;
    let position = position
        .parse::<f32>()
        .context("gradient stop position is not a number")?;
    Ok(GradientStop::new(position, parse_color(color)?))
}

fn parse_color(value: &str) -> Result<[u8; 4]> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 8 {
        bail!("color must use 8 hexadecimal RGBA digits");
    }
    let mut output = [0; 4];
    for (index, channel) in output.iter_mut().enumerate() {
        *channel = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .context("color contains invalid hexadecimal digits")?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments() -> GradientArgs {
        GradientArgs {
            id: 1,
            clear: false,
            gradient_json: None,
            angle: None,
            kind: None,
            spread: None,
            center_x: None,
            center_y: None,
            radius: None,
            offset: None,
            extent: None,
            stops: Vec::new(),
            start: None,
            end: None,
        }
    }

    #[test]
    fn command_construction_rejects_mixed_modern_and_legacy_stop_surfaces() {
        let mut arguments = arguments();
        arguments.stops = vec!["0:ff0000ff".into(), "1:0000ffff".into()];
        arguments.start = Some("00ff00ff".into());
        assert!(gradient_command(arguments).is_err());
    }

    #[test]
    fn command_construction_rejects_clear_and_structured_surface_mixes() {
        let mut clear = arguments();
        clear.clear = true;
        clear.kind = Some(GradientKindArg::Radial);
        assert!(gradient_command(clear).is_err());

        let mut structured = arguments();
        structured.gradient_json =
            Some(r#"{"kind":"linear","stops":[],"interpolation":"premultiplied_srgb_v1"}"#.into());
        structured.radius = Some(0.75);
        assert!(gradient_command(structured).is_err());
    }
}
