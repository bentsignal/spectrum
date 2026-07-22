use std::{collections::BTreeSet, path::PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use ttf_parser::Face;

use crate::{Document, FontEmbeddingPermission, LayerKind};

/// The exact Unicode scalar values currently requested from one embedded font.
///
/// This is deliberately an analysis result, not a mutation contract. A portable
/// editable project still embeds the complete imported font so future edits can
/// introduce characters that are not present in this repertoire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FontUsage {
    pub font_id: u64,
    pub layer_ids: Vec<u64>,
    /// Non-selector Unicode scalars whose cmap mappings a subset must retain.
    pub codepoints: Vec<u32>,
    /// Adjacent base/selector pairs whose cmap format 14 mappings must be retained.
    pub variation_sequences: Vec<UnicodeVariationSequence>,
    /// Selectors without an adjacent base scalar cannot form a valid cmap query.
    pub unpaired_variation_selectors: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct UnicodeVariationSequence {
    pub base_codepoint: u32,
    pub selector_codepoint: u32,
}

/// Unicode cmap retention information for a future font subset.
///
/// This does not model symbol or other non-Unicode cmaps, shaping, font fallback,
/// or Prism renderer behavior. It must not be described as legal license advice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FontUsageAnalysis {
    pub usage: FontUsage,
    pub family: String,
    pub style: String,
    pub content_hash: String,
    pub source_name: String,
    pub original_path: Option<PathBuf>,
    pub embedding_permission: FontEmbeddingPermission,
    pub embedding_advisory: Option<String>,
    /// The OpenType OS/2 embedding metadata's no-subsetting bit, not legal advice.
    pub embedding_metadata_allows_subsetting: bool,
    pub source_bytes: u64,
    pub missing_codepoints: Vec<u32>,
    pub missing_variation_sequences: Vec<UnicodeVariationSequence>,
}

pub fn font_usage(document: &Document, font_id: u64) -> Result<FontUsage> {
    document.font_asset(font_id)?;
    let mut layer_ids = Vec::new();
    let mut codepoints = BTreeSet::new();
    let mut variation_sequences = BTreeSet::new();
    let mut unpaired_variation_selectors = BTreeSet::new();
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
        collect_unicode_repertoire(
            text,
            &mut codepoints,
            &mut variation_sequences,
            &mut unpaired_variation_selectors,
        );
    }
    layer_ids.sort_unstable();
    Ok(FontUsage {
        font_id,
        layer_ids,
        codepoints: codepoints.into_iter().collect(),
        variation_sequences: variation_sequences.into_iter().collect(),
        unpaired_variation_selectors: unpaired_variation_selectors.into_iter().collect(),
    })
}

fn collect_unicode_repertoire(
    text: &str,
    codepoints: &mut BTreeSet<u32>,
    variation_sequences: &mut BTreeSet<UnicodeVariationSequence>,
    unpaired_variation_selectors: &mut BTreeSet<u32>,
) {
    let mut adjacent_base = None;
    for character in text.chars() {
        if character == '\n' {
            adjacent_base = None;
            continue;
        }
        if is_variation_selector(character) {
            if let Some(base) = adjacent_base.take() {
                variation_sequences.insert(UnicodeVariationSequence {
                    base_codepoint: u32::from(base),
                    selector_codepoint: u32::from(character),
                });
            } else {
                unpaired_variation_selectors.insert(u32::from(character));
            }
            continue;
        }
        codepoints.insert(u32::from(character));
        adjacent_base = Some(character);
    }
}

fn is_variation_selector(character: char) -> bool {
    matches!(u32::from(character), 0xfe00..=0xfe0f | 0xe0100..=0xe01ef)
}

pub fn analyze_font_usage(document: &Document, font_id: u64) -> Result<FontUsageAnalysis> {
    let font = document.font_asset(font_id)?;
    let bytes = font.bytes()?;
    analyze_font_usage_with_source(document, font_id, &bytes)
}

pub(crate) fn analyze_font_usage_with_source(
    document: &Document,
    font_id: u64,
    bytes: &[u8],
) -> Result<FontUsageAnalysis> {
    let font = document.font_asset(font_id)?;
    let usage = font_usage(document, font_id)?;
    let face = Face::parse(bytes, 0)
        .with_context(|| format!("embedded font {} is not valid OpenType data", font.id))?;
    let missing_codepoints = usage
        .codepoints
        .iter()
        .copied()
        .filter(|codepoint| {
            char::from_u32(*codepoint).is_none_or(|character| face.glyph_index(character).is_none())
        })
        .collect();
    let missing_variation_sequences = usage
        .variation_sequences
        .iter()
        .copied()
        .filter(|sequence| {
            let Some(base) = char::from_u32(sequence.base_codepoint) else {
                return true;
            };
            let Some(selector) = char::from_u32(sequence.selector_codepoint) else {
                return true;
            };
            face.glyph_variation_index(base, selector).is_none()
        })
        .collect();
    Ok(FontUsageAnalysis {
        usage,
        family: font.family.clone(),
        style: font.style.clone(),
        content_hash: font.content_hash.clone(),
        source_name: font.source_name.clone(),
        original_path: font.original_path.clone(),
        embedding_permission: font.embedding_permission,
        embedding_advisory: font.embedding_permission.advisory().map(str::to_owned),
        embedding_metadata_allows_subsetting: font.subset_allowed,
        source_bytes: bytes.len().try_into().unwrap_or(u64::MAX),
        missing_codepoints,
        missing_variation_sequences,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repertoire_preserves_adjacent_variation_sequences_and_flags_orphans() {
        let mut codepoints = BTreeSet::new();
        let mut sequences = BTreeSet::new();
        let mut unpaired = BTreeSet::new();
        collect_unicode_repertoire(
            "A\u{fe0f}B\u{e0100}\u{fe0e}\n\u{fe0f}C",
            &mut codepoints,
            &mut sequences,
            &mut unpaired,
        );

        assert_eq!(codepoints.into_iter().collect::<Vec<_>>(), vec![65, 66, 67]);
        assert_eq!(
            sequences.into_iter().collect::<Vec<_>>(),
            vec![
                UnicodeVariationSequence {
                    base_codepoint: 65,
                    selector_codepoint: 0xfe0f,
                },
                UnicodeVariationSequence {
                    base_codepoint: 66,
                    selector_codepoint: 0xe0100,
                },
            ]
        );
        assert_eq!(
            unpaired.into_iter().collect::<Vec<_>>(),
            vec![0xfe0e, 0xfe0f]
        );
    }
}
