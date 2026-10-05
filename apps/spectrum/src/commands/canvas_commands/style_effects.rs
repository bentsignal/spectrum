//! `spectrum canvas effect`: one layer style at a time. Unset flags keep the
//! layer's current values for that style, and the layer's other styles stay.
use anyhow::{Result, bail};
use clap::{Args, ValueEnum};
use spectrum_canvas::{
    BevelEmboss, BevelStyle, ColorOverlay, Command, DropShadow, Glow, GradientOverlay, LayerStroke,
    LayerStyle, Satin, ShapeGradient, StrokePosition,
};

use super::{
    blend::CliBlend,
    effects::{GradientKindArg, parse_color, parse_stop},
};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum EffectKind {
    Stroke,
    OuterGlow,
    InnerGlow,
    InnerShadow,
    ColorOverlay,
    GradientOverlay,
    Satin,
    Bevel,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum StrokePositionArg {
    Outside,
    Inside,
    Center,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum BevelStyleArg {
    Inner,
    Outer,
    Emboss,
    Pillow,
}

#[derive(Args, Debug)]
pub(super) struct EffectArgs {
    pub id: u64,
    pub kind: EffectKind,
    /// Remove this style from the layer.
    #[arg(long)]
    pub clear: bool,
    /// Color and strength as RRGGBBAA (stroke, glows, inner shadow, overlay, satin).
    #[arg(long)]
    pub color: Option<String>,
    /// Stroke width, glow size, blur, satin softness, or bevel size, in canvas pixels.
    #[arg(long)]
    pub size: Option<f32>,
    /// Glow spread from 0 (soft) to 1 (solid).
    #[arg(long)]
    pub spread: Option<f32>,
    /// Stroke position relative to the layer's edge.
    #[arg(long, value_enum)]
    position: Option<StrokePositionArg>,
    /// Inner shadow offsets in canvas pixels.
    #[arg(long, allow_negative_numbers = true)]
    pub x: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    pub y: Option<f32>,
    /// Light angle for bevels, offset angle for satin, or gradient angle, in degrees.
    #[arg(long, allow_negative_numbers = true)]
    pub angle: Option<f32>,
    /// Satin offset in canvas pixels.
    #[arg(long)]
    pub distance: Option<f32>,
    /// Bevel steepness; 1 is Photoshop's 100%.
    #[arg(long)]
    pub depth: Option<f32>,
    /// Bevel light height in degrees.
    #[arg(long)]
    pub altitude: Option<f32>,
    #[arg(long, value_enum)]
    bevel_style: Option<BevelStyleArg>,
    /// Press the bevel down instead of raising it (`--down true`).
    #[arg(long)]
    pub down: Option<bool>,
    /// Bevel highlight and shadow colors as RRGGBBAA.
    #[arg(long)]
    pub highlight: Option<String>,
    #[arg(long)]
    pub shadow_color: Option<String>,
    /// Satin: shade where the shape overlaps itself (`--invert false` for the opposite).
    #[arg(long)]
    pub invert: Option<bool>,
    /// Blend mode for satin and gradient overlays.
    #[arg(long, value_enum)]
    mode: Option<CliBlend>,
    /// Gradient overlay stops as POSITION:RRGGBBAA, repeated 2 to 32 times.
    #[arg(long = "stop")]
    pub stops: Vec<String>,
    #[arg(long, value_enum)]
    gradient_kind: Option<GradientKindArg>,
    /// Gradient overlay center as fractions of the layer (0 to 1).
    #[arg(long)]
    pub center_x: Option<f32>,
    #[arg(long)]
    pub center_y: Option<f32>,
    /// Radial gradient overlay size as a fraction of the layer's short side.
    #[arg(long)]
    pub radius: Option<f32>,
    /// Linear gradient overlay length as a fraction of the layer.
    #[arg(long)]
    pub scale: Option<f32>,
}

/// Sets one style, starting from the layer's current settings for it.
pub(super) fn effect_command(arguments: EffectArgs, mut style: LayerStyle) -> Result<Command> {
    let color = arguments.color.as_deref().map(parse_color).transpose()?;
    let glow = |current: Option<Glow>| {
        let current = current.unwrap_or_default();
        Glow {
            color: color.unwrap_or(current.color),
            size: arguments.size.unwrap_or(current.size),
            spread: arguments.spread.unwrap_or(current.spread),
        }
    };
    let clear = arguments.clear;
    match arguments.kind {
        EffectKind::Stroke => {
            let current = style.stroke.unwrap_or_default();
            style.stroke = (!clear).then(|| LayerStroke {
                size: arguments.size.unwrap_or(current.size),
                position: arguments.position.map_or(current.position, |p| match p {
                    StrokePositionArg::Outside => StrokePosition::Outside,
                    StrokePositionArg::Inside => StrokePosition::Inside,
                    StrokePositionArg::Center => StrokePosition::Center,
                }),
                color: color.unwrap_or(current.color),
            });
        }
        EffectKind::OuterGlow => style.outer_glow = (!clear).then(|| glow(style.outer_glow)),
        EffectKind::InnerGlow => style.inner_glow = (!clear).then(|| glow(style.inner_glow)),
        EffectKind::InnerShadow => {
            let current = style.inner_shadow.unwrap_or_default();
            style.inner_shadow = (!clear).then(|| DropShadow {
                color: color.unwrap_or(current.color),
                offset_x: arguments.x.unwrap_or(current.offset_x),
                offset_y: arguments.y.unwrap_or(current.offset_y),
                blur_radius: arguments.size.unwrap_or(current.blur_radius),
            });
        }
        EffectKind::ColorOverlay => {
            let current = style.color_overlay.unwrap_or_default();
            style.color_overlay = (!clear).then(|| ColorOverlay {
                color: color.unwrap_or(current.color),
            });
        }
        EffectKind::Satin => {
            let current = style.satin.unwrap_or_default();
            style.satin = (!clear).then(|| Satin {
                color: color.unwrap_or(current.color),
                angle: arguments.angle.unwrap_or(current.angle),
                distance: arguments.distance.unwrap_or(current.distance),
                size: arguments.size.unwrap_or(current.size),
                invert: arguments.invert.unwrap_or(current.invert),
                blend_mode: arguments.mode.map_or(current.blend_mode, Into::into),
            });
        }
        EffectKind::Bevel => {
            let current = style.bevel.unwrap_or_default();
            let highlight = arguments
                .highlight
                .as_deref()
                .map(parse_color)
                .transpose()?;
            let shadow = arguments
                .shadow_color
                .as_deref()
                .map(parse_color)
                .transpose()?;
            style.bevel = (!clear).then(|| BevelEmboss {
                style: arguments.bevel_style.map_or(current.style, |s| match s {
                    BevelStyleArg::Inner => BevelStyle::Inner,
                    BevelStyleArg::Outer => BevelStyle::Outer,
                    BevelStyleArg::Emboss => BevelStyle::Emboss,
                    BevelStyleArg::Pillow => BevelStyle::Pillow,
                }),
                size: arguments.size.unwrap_or(current.size),
                depth: arguments.depth.unwrap_or(current.depth),
                angle: arguments.angle.unwrap_or(current.angle),
                altitude: arguments.altitude.unwrap_or(current.altitude),
                up: arguments.down.map_or(current.up, |down| !down),
                highlight: highlight.unwrap_or(current.highlight),
                shadow: shadow.unwrap_or(current.shadow),
            });
        }
        EffectKind::GradientOverlay => {
            let current = style.gradient_overlay.clone().unwrap_or_default();
            if clear {
                style.gradient_overlay = None;
            } else {
                let stops = arguments
                    .stops
                    .iter()
                    .map(|stop| parse_stop(stop))
                    .collect::<Result<Vec<_>>>()?;
                if stops.len() == 1 {
                    bail!("a gradient needs at least two --stop values");
                }
                let gradient = ShapeGradient {
                    stops: if stops.is_empty() {
                        current.gradient.stops.clone()
                    } else {
                        stops
                    },
                    angle: arguments.angle.unwrap_or(current.gradient.angle),
                    kind: arguments
                        .gradient_kind
                        .map_or(current.gradient.kind, Into::into),
                    center: [
                        arguments.center_x.unwrap_or(current.gradient.center[0]),
                        arguments.center_y.unwrap_or(current.gradient.center[1]),
                    ],
                    radius: arguments.radius.unwrap_or(current.gradient.radius),
                    extent: arguments.scale.unwrap_or(current.gradient.extent),
                    ..current.gradient.clone()
                };
                style.gradient_overlay = Some(GradientOverlay {
                    gradient,
                    blend_mode: arguments.mode.map_or(current.blend_mode, Into::into),
                });
            }
        }
    }
    Ok(Command::SetLayerStyle {
        id: arguments.id,
        style,
    })
}
