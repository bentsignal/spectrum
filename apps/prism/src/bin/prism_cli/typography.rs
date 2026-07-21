use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use prism_core::{
    Document, DurableProject, FontAsset, TextAlignment, TextTypography, VerifiedFontSource,
    Workspace, inspect_font_source_read_only, inspect_font_subset_read_only,
};
use serde_json::{Value, json};
use spectrum_revisions::SessionId;

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

pub(super) fn font_usage(document: &Document, font_id: Option<u64>) -> Result<Value> {
    let fonts = match font_id {
        Some(font_id) => vec![prism_core::analyze_font_usage(document, font_id)?],
        None => prism_core::analyze_all_font_usage(document)?,
    };
    Ok(json!({
        "ok": true,
        "action": "font_usage",
        "analysis_scope": "unicode_cmap_subset_retention",
        "font_bytes_modified": false,
        "editable_font_bytes_preserved": true,
        "limitations": [
            "does not inspect symbol or other non-Unicode cmaps",
            "does not model shaping, renderer fallback, or legal license terms",
            "opening --session retains Prism's standard session-resume behavior"
        ],
        "fonts": fonts,
    }))
}

pub(super) fn font_source(document: &Document, font_id: u64) -> Result<Value> {
    let font = document.font_asset(font_id)?;
    let snapshot = font.source_snapshot()?;
    Ok(font_source_value(
        font,
        snapshot.content_hash(),
        snapshot.len(),
        snapshot.subset_allowed(),
    ))
}

pub(super) fn font_source_command(
    project: &Path,
    session: Option<SessionId>,
    font_id: u64,
) -> Result<Value> {
    if session.is_some() {
        bail!("font-source is read-only and does not accept --session");
    }
    if DurableProject::looks_durable(project)? {
        let inspected = inspect_font_source_read_only(project, font_id)?;
        Ok(verified_font_source(&inspected.font, &inspected.source))
    } else {
        font_source(&Workspace::load_read_only(project)?, font_id)
    }
}

pub(super) fn verified_font_source(font: &FontAsset, source: &VerifiedFontSource) -> Value {
    font_source_value(
        font,
        source.content_hash(),
        source.len(),
        source.subset_allowed(),
    )
}

pub(super) fn font_subset_plan(document: &Document, font_id: u64) -> Result<Value> {
    subset_plan_value(prism_core::plan_font_subset(document, font_id)?)
}

pub(super) fn font_subset_plan_command(
    project: &Path,
    session: Option<SessionId>,
    font_id: u64,
) -> Result<Value> {
    if session.is_some() {
        bail!("font-subset-plan is read-only and does not accept --session");
    }
    if DurableProject::looks_durable(project)? {
        let inspected = inspect_font_subset_read_only(project, font_id)?;
        verified_font_subset_plan(&inspected.document, font_id, &inspected.source)
    } else {
        font_subset_plan(&Workspace::load_read_only(project)?, font_id)
    }
}

pub(super) fn verified_font_subset_plan(
    document: &Document,
    font_id: u64,
    source: &VerifiedFontSource,
) -> Result<Value> {
    subset_plan_value(prism_core::plan_font_subset_with_verified_source(
        document, font_id, source,
    )?)
}

fn subset_plan_value(plan: prism_core::FontSubsetPlan) -> Result<Value> {
    Ok(json!({
        "ok": true,
        "action": "font_subset_plan",
        "mutates_project": false,
        "font_bytes_modified": false,
        "candidate_bytes_emitted": false,
        "storage_decision": "a history-preserving reduction requires a separate fresh-database compact-copy transaction; appending a subset cannot remove retained full-font assets",
        "license_notice": "OpenType embedding metadata is a technical check, not a legal license conclusion",
        "plan": plan,
    }))
}

fn font_source_value(
    font: &FontAsset,
    content_hash: &str,
    source_bytes: usize,
    subset_allowed: bool,
) -> Value {
    json!({
        "ok": true,
        "action": "font_source",
        "font_id": font.id,
        "family": &font.family,
        "style": &font.style,
        "source_name": &font.source_name,
        "content_hash": content_hash,
        "source_bytes": source_bytes,
        "embedding_metadata_allows_subsetting": subset_allowed,
        "editable_embedding_verified": true,
        "immutable_identity_verified": true,
        "font_bytes_modified": false,
        "mutates_project": false,
        "license_notice": "OpenType embedding metadata is a technical check, not a legal license conclusion"
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
