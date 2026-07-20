use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use prism_core::{Document, TextAlignment, TextTypography};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum CliTextAlignment {
    Left,
    Center,
    Right,
}

impl From<CliTextAlignment> for TextAlignment {
    fn from(value: CliTextAlignment) -> Self {
        match value {
            CliTextAlignment::Left => Self::Left,
            CliTextAlignment::Center => Self::Center,
            CliTextAlignment::Right => Self::Right,
        }
    }
}

#[derive(Args, Debug)]
pub(super) struct TypographyArgs {
    pub id: u64,
    /// Select an embedded face directly by asset id.
    #[arg(long, conflicts_with_all = ["family", "bundled"])]
    pub font_id: Option<u64>,
    /// Select the closest embedded face in this family.
    #[arg(long, conflicts_with_all = ["font_id", "bundled"])]
    pub family: Option<String>,
    /// Preferred OpenType weight when resolving --family.
    #[arg(long, requires = "family")]
    pub weight: Option<u16>,
    /// Preferred style/subfamily when resolving --family.
    #[arg(long, requires = "family")]
    pub style: Option<String>,
    /// Return to Prism's bundled portable font.
    #[arg(long, conflicts_with_all = ["font_id", "family"])]
    pub bundled: bool,
    #[arg(long)]
    pub align: Option<CliTextAlignment>,
    #[arg(long)]
    pub line_height: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    pub tracking: Option<f32>,
    /// Paragraph width; pass 0 to remove wrapping.
    #[arg(long)]
    pub box_width: Option<f32>,
    #[arg(long)]
    pub outline_width: Option<f32>,
    #[arg(long)]
    pub outline_color: Option<String>,
    #[arg(long, allow_negative_numbers = true)]
    pub shadow_x: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    pub shadow_y: Option<f32>,
    #[arg(long)]
    pub shadow_color: Option<String>,
}

pub(super) fn updated_typography(
    document: &Document,
    arguments: &TypographyArgs,
) -> Result<TextTypography> {
    let layer = document.layer(arguments.id)?;
    let prism_core::LayerKind::Text { typography, .. } = &layer.kind else {
        bail!("layer {} is not a text layer", arguments.id);
    };
    let mut updated = typography.clone();
    if arguments.bundled {
        updated.font_id = None;
    } else if let Some(id) = arguments.font_id {
        document.font_asset(id)?;
        updated.font_id = Some(id);
    } else if let Some(family) = &arguments.family {
        updated.font_id = Some(resolve_face(
            document,
            family,
            arguments.weight,
            arguments.style.as_deref(),
        )?);
    }
    if let Some(alignment) = arguments.align {
        updated.alignment = alignment.into();
    }
    if let Some(value) = arguments.line_height {
        updated.line_height = value;
    }
    if let Some(value) = arguments.tracking {
        updated.tracking = value;
    }
    if let Some(value) = arguments.box_width {
        updated.box_width = (value > 0.0).then_some(value);
    }
    if let Some(value) = arguments.outline_width {
        updated.effects.outline_width = value;
    }
    if let Some(value) = &arguments.outline_color {
        updated.effects.outline_color = parse_color(value)?;
    }
    if let Some(value) = arguments.shadow_x {
        updated.effects.shadow_offset_x = value;
    }
    if let Some(value) = arguments.shadow_y {
        updated.effects.shadow_offset_y = value;
    }
    if let Some(value) = &arguments.shadow_color {
        updated.effects.shadow_color = parse_color(value)?;
    }
    Ok(updated)
}

pub(super) fn font_list(document: &Document, query: Option<String>) -> Value {
    let query = query.unwrap_or_default().to_ascii_lowercase();
    let fonts: Vec<_> = document
        .font_assets
        .iter()
        .filter(|font| {
            query.is_empty()
                || font.family.to_ascii_lowercase().contains(&query)
                || font.style.to_ascii_lowercase().contains(&query)
        })
        .collect();
    json!({
        "ok": true,
        "action": "font_list",
        "bundled": {"id": null, "family": "Spectrum Sans", "style": "Regular", "weight": 300},
        "fonts": fonts,
    })
}

fn resolve_face(
    document: &Document,
    family: &str,
    weight: Option<u16>,
    style: Option<&str>,
) -> Result<u64> {
    let family = family.trim();
    let mut candidates: Vec<_> = document
        .font_assets
        .iter()
        .filter(|font| font.family.eq_ignore_ascii_case(family))
        .filter(|font| style.is_none_or(|style| font.style.eq_ignore_ascii_case(style)))
        .collect();
    let preferred_weight = weight.unwrap_or(400);
    candidates.sort_by_key(|font| font.weight.abs_diff(preferred_weight));
    candidates
        .first()
        .map(|font| font.id)
        .with_context(|| format!("no embedded font face matches family {family:?}"))
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
