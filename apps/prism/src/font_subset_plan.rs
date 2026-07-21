use anyhow::{Result, bail};
use serde::Serialize;
use spectrum_fonts::{
    FontSubsetEngine, HarfBuzzSubsetEngine, SubsetRequest,
    UnicodeVariationSequence as SubsetVariationSequence,
};
use spectrum_revisions::AssetId;

use crate::{Document, LayerKind, VerifiedFontSource, font_usage::analyze_font_usage_with_source};

const ENGINE_NAME: &str = "spectrum_fonts_harfbuzz_candidate";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FontShapingSample {
    pub layer_id: u64,
    pub line_index: usize,
    pub codepoints: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FontSubsetCandidate {
    pub content_hash: String,
    pub bytes: u64,
    pub reduction_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FontSubsetPlan {
    pub analysis: crate::FontUsageAnalysis,
    pub shaping_mode: String,
    pub shaping_samples: Vec<FontShapingSample>,
    pub engine: String,
    pub candidate: Option<FontSubsetCandidate>,
    pub candidate_blockers: Vec<String>,
    pub physical_replacement_supported: bool,
    pub physical_replacement_blockers: Vec<String>,
}

pub fn plan_font_subset(document: &Document, font_id: u64) -> Result<FontSubsetPlan> {
    let font = document.font_asset(font_id)?;
    let source = font.source_snapshot()?;
    plan_font_subset_bytes(document, font_id, source.bytes())
}

pub fn plan_font_subset_with_verified_source(
    document: &Document,
    font_id: u64,
    source: &VerifiedFontSource,
) -> Result<FontSubsetPlan> {
    let font = document.font_asset(font_id)?;
    if font.content_hash != source.content_hash() {
        bail!("verified font source does not match the requested font asset");
    }
    plan_font_subset_bytes(document, font_id, source.bytes())
}

fn plan_font_subset_bytes(
    document: &Document,
    font_id: u64,
    source: &[u8],
) -> Result<FontSubsetPlan> {
    let analysis = analyze_font_usage_with_source(document, font_id, source)?;
    let shaping_samples = shaping_samples(document, font_id);
    let mut candidate_blockers = preflight_blockers(&analysis);
    let mut candidate = None;
    if candidate_blockers.is_empty() {
        let request = SubsetRequest::new(
            0,
            analysis.usage.codepoints.iter().copied(),
            analysis
                .usage
                .variation_sequences
                .iter()
                .map(|sequence| SubsetVariationSequence {
                    base_codepoint: sequence.base_codepoint,
                    selector_codepoint: sequence.selector_codepoint,
                }),
        )
        .with_shaping_samples(
            shaping_samples
                .iter()
                .map(|sample| sample.codepoints.iter().copied()),
        );
        match HarfBuzzSubsetEngine.subset(source, &request) {
            Ok(artifact) => {
                candidate = Some(FontSubsetCandidate {
                    content_hash: AssetId::for_bytes(&artifact.bytes).to_string(),
                    bytes: artifact.subset_bytes.try_into().unwrap_or(u64::MAX),
                    reduction_bytes: artifact
                        .source_bytes
                        .saturating_sub(artifact.subset_bytes)
                        .try_into()
                        .unwrap_or(u64::MAX),
                });
            }
            Err(error) => candidate_blockers.push(error.to_string()),
        }
    }
    candidate_blockers.sort();
    candidate_blockers.dedup();
    Ok(FontSubsetPlan {
        analysis,
        shaping_mode: "harfbuzz_default_features_language_und_guessed_script_direction".into(),
        shaping_samples,
        engine: ENGINE_NAME.into(),
        candidate,
        candidate_blockers,
        physical_replacement_supported: false,
        physical_replacement_blockers: vec![
            "editable projects must retain the complete source font for future text edits".into(),
            "durable revision history retains every referenced content-addressed full-font blob"
                .into(),
            "history-preserving compact-copy rewriting and whole-document render parity are not implemented"
                .into(),
        ],
    })
}

fn shaping_samples(document: &Document, font_id: u64) -> Vec<FontShapingSample> {
    let mut samples = document
        .layers
        .iter()
        .filter_map(|layer| {
            let LayerKind::Text {
                text, typography, ..
            } = &layer.kind
            else {
                return None;
            };
            (typography.font_id == Some(font_id)).then_some((layer.id, text))
        })
        .flat_map(|(layer_id, text)| {
            text.split('\n')
                .enumerate()
                .filter_map(move |(line_index, line)| {
                    let codepoints = line.chars().map(u32::from).collect::<Vec<_>>();
                    (!codepoints.is_empty()).then_some(FontShapingSample {
                        layer_id,
                        line_index,
                        codepoints,
                    })
                })
        })
        .collect::<Vec<_>>();
    samples.sort_by(|left, right| {
        (left.layer_id, left.line_index, &left.codepoints).cmp(&(
            right.layer_id,
            right.line_index,
            &right.codepoints,
        ))
    });
    samples
}

fn preflight_blockers(analysis: &crate::FontUsageAnalysis) -> Vec<String> {
    let mut blockers = Vec::new();
    if analysis.usage.layer_ids.is_empty() {
        blockers.push("font is not used by any current text layer".into());
    }
    if analysis.usage.codepoints.is_empty() && analysis.usage.variation_sequences.is_empty() {
        blockers.push("font subset repertoire is empty".into());
    }
    if !analysis.usage.unpaired_variation_selectors.is_empty() {
        blockers.push("text contains unpaired Unicode variation selectors".into());
    }
    if !analysis.missing_codepoints.is_empty() {
        blockers.push("font does not map every requested Unicode codepoint".into());
    }
    if !analysis.missing_variation_sequences.is_empty() {
        blockers.push("font does not map every requested Unicode variation sequence".into());
    }
    if !analysis.embedding_metadata_allows_subsetting {
        blockers.push("OpenType embedding metadata forbids technical subsetting".into());
    }
    blockers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, FontAsset, TextTypography, apply_command};
    use std::path::PathBuf;

    const STATIC_FONT: &[u8] =
        include_bytes!("../../../crates/spectrum-fonts/tests/fonts/noto-sans-static-source.ttf");

    fn font_document(text: &str, bytes: &[u8]) -> (Document, VerifiedFontSource) {
        let hash = AssetId::for_bytes(bytes).to_string();
        let path = PathBuf::from(format!("spectrum-asset:{hash}.ttf"));
        let source = VerifiedFontSource::from_embedded_bytes(bytes.to_vec(), &hash).unwrap();
        let font =
            FontAsset::from_embedded_bytes(1, "fixture.ttf".into(), path, bytes.to_vec(), &hash)
                .unwrap();
        let mut document = Document::new("Subset plan", 320, 200);
        document.font_assets.push(font);
        document.next_font_id = 2;
        apply_command(
            &mut document,
            Command::AddText {
                text: text.into(),
                name: None,
                font_size: 32.0,
                color: [255; 4],
                x: 0.0,
                y: 0.0,
            },
        )
        .unwrap();
        let id = document.selected.unwrap();
        apply_command(
            &mut document,
            Command::SetTextTypography {
                id,
                typography: TextTypography {
                    font_id: Some(1),
                    ..Default::default()
                },
            },
        )
        .unwrap();
        (document, source)
    }

    #[test]
    fn deterministic_plan_proves_reduction_without_mutating_the_document() {
        let (document, source) = font_document("BA\nAV", STATIC_FONT);
        let before = document.clone();

        let first = plan_font_subset_with_verified_source(&document, 1, &source).unwrap();
        let second = plan_font_subset_with_verified_source(&document, 1, &source).unwrap();

        assert_eq!(first, second);
        let candidate = first.candidate.as_ref().unwrap();
        assert!(candidate.bytes < first.analysis.source_bytes);
        assert_eq!(
            candidate.reduction_bytes,
            first.analysis.source_bytes - candidate.bytes
        );
        assert!(first.candidate_blockers.is_empty());
        assert!(!first.physical_replacement_supported);
        assert_eq!(
            first
                .shaping_samples
                .iter()
                .map(|sample| sample.codepoints.clone())
                .collect::<Vec<_>>(),
            vec![vec![66, 65], vec![65, 86]]
        );
        assert_eq!(document, before);
    }

    #[test]
    fn unsupported_candidate_is_a_read_only_plan_blocker() {
        let (document, source) = font_document("AB", epaint_default_fonts::HACK_REGULAR);

        let plan = plan_font_subset_with_verified_source(&document, 1, &source).unwrap();

        assert!(plan.candidate.is_none());
        assert!(!plan.candidate_blockers.is_empty());
        assert!(!plan.physical_replacement_supported);
    }
}
