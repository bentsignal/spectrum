use std::collections::BTreeSet;

use anyhow::{Context, Result};
use serde::Serialize;
use ttf_parser::Face;

use crate::{Document, LayerKind};

/// The exact Unicode scalar values currently requested from one embedded font.
///
/// This is deliberately an analysis result, not a mutation contract. A portable
/// editable project still embeds the complete imported font so future edits can
/// introduce characters that are not present in this repertoire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FontUsage {
    pub font_id: u64,
    pub layer_ids: Vec<u64>,
    pub codepoints: Vec<u32>,
}

/// Usage and cmap coverage information needed to make a future font-subsetting
/// policy decision without changing the source or embedded font bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FontUsageAnalysis {
    pub usage: FontUsage,
    pub family: String,
    pub style: String,
    pub content_hash: String,
    pub subset_allowed: bool,
    pub source_bytes: u64,
    pub missing_codepoints: Vec<u32>,
}

pub fn font_usage(document: &Document, font_id: u64) -> Result<FontUsage> {
    document.font_asset(font_id)?;
    let mut layer_ids = Vec::new();
    let mut codepoints = BTreeSet::new();
    for layer in &document.layers {
        let LayerKind::Text {
            text, typography, ..
        } = &layer.kind
        else {
            continue;
        };
        if typography.font_id != Some(font_id) {
            continue;
        }
        layer_ids.push(layer.id);
        codepoints.extend(
            text.chars()
                .filter(|character| *character != '\n')
                .map(u32::from),
        );
    }
    layer_ids.sort_unstable();
    Ok(FontUsage {
        font_id,
        layer_ids,
        codepoints: codepoints.into_iter().collect(),
    })
}

pub fn analyze_font_usage(document: &Document, font_id: u64) -> Result<FontUsageAnalysis> {
    let font = document.font_asset(font_id)?;
    let usage = font_usage(document, font_id)?;
    let bytes = font.bytes()?;
    let face = Face::parse(&bytes, 0)
        .with_context(|| format!("embedded font {} is not valid OpenType data", font.id))?;
    let missing_codepoints = usage
        .codepoints
        .iter()
        .copied()
        .filter(|codepoint| {
            char::from_u32(*codepoint).is_none_or(|character| face.glyph_index(character).is_none())
        })
        .collect();
    Ok(FontUsageAnalysis {
        usage,
        family: font.family.clone(),
        style: font.style.clone(),
        content_hash: font.content_hash.clone(),
        subset_allowed: font.subset_allowed,
        source_bytes: bytes.len().try_into().unwrap_or(u64::MAX),
        missing_codepoints,
    })
}

pub fn analyze_all_font_usage(document: &Document) -> Result<Vec<FontUsageAnalysis>> {
    let mut font_ids = document
        .font_assets
        .iter()
        .map(|font| font.id)
        .collect::<Vec<_>>();
    font_ids.sort_unstable();
    font_ids
        .into_iter()
        .map(|font_id| analyze_font_usage(document, font_id))
        .collect()
}
