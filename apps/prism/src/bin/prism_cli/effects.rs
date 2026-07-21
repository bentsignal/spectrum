use anyhow::{Context, Result, bail};
use clap::Args;
use prism_core::{Command, DropShadow, GradientStop, LayerStyle, ShapeFill, ShapeGradient};

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
    #[arg(long, default_value = "5dd8c7ff")]
    pub start: String,
    #[arg(long, default_value = "ae7bffff")]
    pub end: String,
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
        Some(ShapeFill::Gradient(ShapeGradient {
            angle: arguments.angle,
            stops: vec![
                GradientStop::new(0.0, parse_color(&arguments.start)?),
                GradientStop::new(1.0, parse_color(&arguments.end)?),
            ],
            ..ShapeGradient::default()
        }))
    };
    Ok(Command::SetShapeFill {
        id: arguments.id,
        fill,
    })
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
