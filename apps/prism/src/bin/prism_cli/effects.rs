use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use prism_core::{
    Command, DropShadow, GradientKind, GradientSpread, GradientStop, LayerStyle, ShapeFill,
    ShapeGradient,
};

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
    #[arg(long)]
    pub clear: bool,
    #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
    pub angle: f32,
    /// Gradient geometry.
    #[arg(long, value_enum, default_value_t = GradientKindArg::Linear)]
    kind: GradientKindArg,
    /// Behavior beyond the first and last stop.
    #[arg(long, value_enum, default_value_t = GradientSpreadArg::Pad)]
    spread: GradientSpreadArg,
    #[arg(long, default_value_t = 0.5)]
    center_x: f32,
    #[arg(long, default_value_t = 0.5)]
    center_y: f32,
    #[arg(long, default_value_t = 0.5)]
    radius: f32,
    /// Ordered POSITION:RRGGBBAA stop. Repeat for 2 through 32 stops.
    #[arg(long = "stop")]
    stops: Vec<String>,
    #[arg(long, default_value = "5dd8c7ff")]
    pub start: String,
    #[arg(long, default_value = "ae7bffff")]
    pub end: String,
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
    let fill = if arguments.clear {
        None
    } else {
        let stops = if arguments.stops.is_empty() {
            vec![
                GradientStop::new(0.0, parse_color(&arguments.start)?),
                GradientStop::new(1.0, parse_color(&arguments.end)?),
            ]
        } else {
            arguments
                .stops
                .iter()
                .map(|value| parse_stop(value))
                .collect::<Result<Vec<_>>>()?
        };
        Some(ShapeFill::Gradient(ShapeGradient {
            kind: arguments.kind.into(),
            angle: arguments.angle,
            stops,
            center: [arguments.center_x, arguments.center_y],
            radius: arguments.radius,
            spread: arguments.spread.into(),
        }))
    };
    Ok(Command::SetShapeFill {
        id: arguments.id,
        fill,
    })
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
