//! Bounded in-process OpenType shaping and candidate font subsetting.
//!
//! [`HarfBuzzShaper`] shapes one already-resolved font run and deliberately leaves
//! bidi paragraph resolution, fallback, line breaking, and editing behavior to
//! higher layers. The subset API remains an engine seam rather than a general
//! export command. Prism's optimized-copy transaction may use a passing artifact
//! only after its own history, asset, and exact-render checks. Passing these checks
//! is not proof of broad production subsetting conformance.
//! Callers must retain the immutable source snapshot, provenance, and font-license
//! decision; OS/2 embedding bits are technical metadata, not legal advice.

mod cmap;
mod conformance;
mod error;
mod glyf;
mod limits;
mod sfnt;
pub mod shaping;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use hb_subset::{Blob, Flags, FontFace, SubsetInput};

pub use error::{ShapeError, SubsetError};
pub use shaping::{
    GlyphFlags, HarfBuzzShaper, MAX_SHAPE_FEATURES, MAX_SHAPE_GLYPHS, MAX_SHAPE_SCALARS,
    MAX_SHAPE_TEXT_BYTES, MAX_VARIATION_COORDINATES, OpenTypeFeature, Script, ShapeRequest,
    ShapedGlyph, ShapedRun, TextDirection, TextShaper, VariationCoordinate,
};

/// One cmap format 14 base/selector mapping requested from the candidate engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnicodeVariationSequence {
    pub base_codepoint: u32,
    pub selector_codepoint: u32,
}

/// One exact source-context run whose shaping policy must survive subsetting.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ShapingSample {
    codepoints: Vec<u32>,
    item_range: Option<(usize, usize)>,
    direction: Option<TextDirection>,
    script: Option<Script>,
    language: Option<String>,
}

impl ShapingSample {
    /// Creates a default-policy sample over the complete source text.
    pub fn new(text: impl AsRef<str>) -> Self {
        Self::from_codepoints(text.as_ref().chars().map(u32::from))
    }

    fn from_codepoints(codepoints: impl IntoIterator<Item = u32>) -> Self {
        Self {
            codepoints: codepoints.into_iter().collect(),
            item_range: None,
            direction: None,
            script: None,
            language: None,
        }
    }

    /// Selects one UTF-8 item while preserving the sample's complete source context.
    pub fn item_range(mut self, range: Range<usize>) -> Self {
        self.item_range = Some((range.start, range.end));
        self
    }

    pub fn direction(mut self, direction: TextDirection) -> Self {
        self.direction = Some(direction);
        self
    }

    pub fn script(mut self, script: Script) -> Self {
        self.script = Some(script);
        self
    }

    pub fn language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    pub fn codepoints(&self) -> &[u32] {
        &self.codepoints
    }

    pub fn item_range_bytes(&self) -> Option<Range<usize>> {
        self.item_range.map(|(start, end)| start..end)
    }

    pub fn requested_direction(&self) -> Option<TextDirection> {
        self.direction
    }

    pub fn requested_script(&self) -> Option<Script> {
        self.script
    }

    pub fn requested_language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    fn selected_codepoints(&self) -> Vec<u32> {
        let Some((start, end)) = self.item_range else {
            return self.codepoints.clone();
        };
        let Ok(text) = self
            .codepoints
            .iter()
            .copied()
            .map(|codepoint| char::from_u32(codepoint).ok_or(()))
            .collect::<Result<String, _>>()
        else {
            return self.codepoints.clone();
        };
        text.get(start..end)
            .map(|item| item.chars().map(u32::from).collect())
            .unwrap_or_else(|| self.codepoints.clone())
    }
}

/// The exact nominal Unicode and default-feature shaping repertoire requested from a font.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubsetRequest {
    face_index: u32,
    nominal_codepoints: Vec<u32>,
    subset_codepoints: Vec<u32>,
    variation_sequences: Vec<UnicodeVariationSequence>,
    shaping_samples: Vec<ShapingSample>,
}

impl SubsetRequest {
    /// Canonicalizes a request and adds every UVS base to the nominal repertoire.
    ///
    /// Only face index zero is currently accepted because collection baselines have
    /// not passed the cross-platform corpus.
    pub fn new(
        face_index: u32,
        codepoints: impl IntoIterator<Item = u32>,
        variation_sequences: impl IntoIterator<Item = UnicodeVariationSequence>,
    ) -> Self {
        let variation_sequences = variation_sequences.into_iter().collect::<BTreeSet<_>>();
        let mut nominal_codepoints = codepoints.into_iter().collect::<BTreeSet<_>>();
        nominal_codepoints.extend(
            variation_sequences
                .iter()
                .map(|sequence| sequence.base_codepoint),
        );
        let nominal_codepoints = nominal_codepoints.into_iter().collect::<Vec<_>>();
        Self {
            face_index,
            subset_codepoints: nominal_codepoints.clone(),
            nominal_codepoints,
            variation_sequences: variation_sequences.into_iter().collect(),
            shaping_samples: Vec::new(),
        }
    }

    /// Adds exact default-feature text runs whose OpenType shaping must remain stable.
    ///
    /// The current candidate fixes language to `und`, guesses direction and script,
    /// and uses HarfBuzz's default feature set. Empty and duplicate runs are discarded.
    /// Every scalar in a retained run is added to HarfBuzz's subset input, but is not
    /// treated as an independently requested nominal cmap mapping. This distinction is
    /// required for variation selectors and other default-ignorable shaping inputs.
    pub fn with_shaping_samples<I, S>(mut self, samples: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: IntoIterator<Item = u32>,
    {
        let samples = samples
            .into_iter()
            .map(ShapingSample::from_codepoints)
            .filter(|sample| !sample.codepoints.is_empty());
        self = self.with_policy_shaping_samples(samples);
        self
    }

    /// Adds exact policy-bearing source-context runs for closure validation.
    pub fn with_policy_shaping_samples(
        mut self,
        samples: impl IntoIterator<Item = ShapingSample>,
    ) -> Self {
        let mut shaping_samples = self.shaping_samples.into_iter().collect::<BTreeSet<_>>();
        shaping_samples.extend(
            samples
                .into_iter()
                .filter(|sample| !sample.codepoints.is_empty()),
        );
        let mut subset_codepoints = self.subset_codepoints.into_iter().collect::<BTreeSet<_>>();
        subset_codepoints.extend(
            shaping_samples
                .iter()
                .flat_map(ShapingSample::selected_codepoints),
        );
        self.subset_codepoints = subset_codepoints.into_iter().collect();
        self.shaping_samples = shaping_samples.into_iter().collect();
        self
    }

    pub fn face_index(&self) -> u32 {
        self.face_index
    }

    /// Scalars that must retain direct nominal cmap mappings.
    pub fn codepoints(&self) -> &[u32] {
        &self.nominal_codepoints
    }

    pub(crate) fn subset_codepoints(&self) -> &[u32] {
        &self.subset_codepoints
    }

    pub fn variation_sequences(&self) -> &[UnicodeVariationSequence] {
        &self.variation_sequences
    }

    pub fn shaping_samples(&self) -> &[ShapingSample] {
        &self.shaping_samples
    }
}

/// Candidate output that passed the current runtime checks, not the conformance corpus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubsetArtifact {
    pub bytes: Vec<u8>,
    pub source_bytes: usize,
    pub subset_bytes: usize,
}

/// Pluggable candidate seam used only behind caller-owned fail-closed proof.
pub trait FontSubsetEngine {
    /// Produces a candidate or fails closed under the implementation's current gates.
    fn subset(&self, source: &[u8], request: &SubsetRequest)
    -> Result<SubsetArtifact, SubsetError>;
}

/// Bundled HarfBuzz 8.2.2 candidate with no system executable or runtime dylib.
///
/// It currently accepts only standalone, static, unhinted TrueType fonts with the
/// crate's strict table allowlist. Hint programs and local glyph instructions fail
/// closed because no portable interpreter proof exists. GSUB/GPOS/GDEF fonts require
/// explicit shaping samples. Passing does not authorize any broader export path.
#[derive(Clone, Copy, Debug, Default)]
pub struct HarfBuzzSubsetEngine;

impl FontSubsetEngine for HarfBuzzSubsetEngine {
    fn subset(
        &self,
        source: &[u8],
        request: &SubsetRequest,
    ) -> Result<SubsetArtifact, SubsetError> {
        let source_profile = conformance::validate_source(source, request)?;
        let blob = Blob::from_bytes(source)
            .map_err(|error| SubsetError::new(format!("HarfBuzz rejected font bytes: {error}")))?;
        let face = FontFace::new_with_index(blob, request.face_index())
            .map_err(|error| SubsetError::new(format!("HarfBuzz rejected font face: {error}")))?;
        let mut input = SubsetInput::new().map_err(|error| {
            SubsetError::new(format!("could not allocate subset input: {error}"))
        })?;

        // Do not call hb_subset_input_keep_everything: bundled HarfBuzz 8.2.2 clears
        // its default drop set. The strict table allowlist makes the untouched default
        // drop set sufficient, while these sets preserve every source name identity.
        input.glyph_set().clear();
        input.unicode_set().clear();
        input.name_id_set().clear();
        for name_id in source_profile.name_ids() {
            input.name_id_set().insert(name_id);
        }
        input.name_lang_id_set().clear();
        for language_id in source_profile.name_languages() {
            input.name_lang_id_set().insert(language_id);
        }
        *input.flags() = *Flags::default()
            .remove_hinting()
            .remap_glyph_indices()
            .retain_subroutines()
            .retain_legacy_names()
            .retain_notdef_outline()
            .retain_glyph_names()
            .retain_unicode_ranges()
            .retain_layout_closure();
        {
            let mut unicode_set = input.unicode_set();
            for codepoint in request.subset_codepoints() {
                let character = char::from_u32(*codepoint).ok_or_else(|| {
                    SubsetError::new(format!("U+{codepoint:04X} is not a Unicode scalar value"))
                })?;
                unicode_set.insert(character);
            }
            for sequence in request.variation_sequences() {
                if let Some(selector) = char::from_u32(sequence.selector_codepoint) {
                    unicode_set.insert(selector);
                }
            }
        }

        let plan = input
            .plan(&face)
            .map_err(|error| SubsetError::new(format!("HarfBuzz planning failed: {error}")))?;
        let glyph_mapping = plan.old_to_new_glyph_mapping();
        if glyph_mapping.len() > limits::MAX_GLYPHS {
            return Err(SubsetError::new(
                "HarfBuzz glyph mapping exceeds resource limit",
            ));
        }
        let glyph_mapping = glyph_mapping
            .iter()
            .map(|(source, output)| {
                let source = u16::try_from(source).map_err(|_| {
                    SubsetError::new("HarfBuzz returned an out-of-range source glyph ID")
                })?;
                let output = u16::try_from(output).map_err(|_| {
                    SubsetError::new("HarfBuzz returned an out-of-range output glyph ID")
                })?;
                Ok((source, output))
            })
            .collect::<Result<BTreeMap<_, _>, SubsetError>>()?;
        let subset_face = plan
            .subset()
            .map_err(|error| SubsetError::new(format!("HarfBuzz subsetting failed: {error}")))?;
        let bytes = subset_face.underlying_blob().to_vec();
        conformance::validate_output(source, &bytes, request, &source_profile, &glyph_mapping)?;
        Ok(SubsetArtifact {
            source_bytes: source.len(),
            subset_bytes: bytes.len(),
            bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_deterministically_sorted_and_deduplicated() {
        let request = SubsetRequest::new(
            2,
            [90, 65, 90],
            [
                UnicodeVariationSequence {
                    base_codepoint: 66,
                    selector_codepoint: 0xfe0f,
                },
                UnicodeVariationSequence {
                    base_codepoint: 65,
                    selector_codepoint: 0xfe0f,
                },
            ],
        );
        assert_eq!(request.face_index(), 2);
        assert_eq!(request.codepoints(), &[65, 66, 90]);
        assert_eq!(request.variation_sequences()[0].base_codepoint, 65);
    }

    #[test]
    fn every_variation_base_is_canonicalized_into_the_repertoire() {
        let request = SubsetRequest::new(
            0,
            [],
            [UnicodeVariationSequence {
                base_codepoint: u32::from('Z'),
                selector_codepoint: 0xfe0f,
            }],
        );
        assert_eq!(request.codepoints(), &[u32::from('Z')]);
    }

    #[test]
    fn shaping_samples_extend_subset_input_without_creating_nominal_requirements() {
        let request = SubsetRequest::new(0, [u32::from('Z')], [])
            .with_shaping_samples([vec![u32::from('f'), u32::from('i')], vec![65, 0xfe0f, 86]])
            .with_shaping_samples([Vec::new(), vec![65, 86]]);
        assert_eq!(request.codepoints(), &[90]);
        assert_eq!(request.subset_codepoints(), &[65, 86, 90, 102, 105, 0xfe0f]);
        assert_eq!(
            request
                .shaping_samples()
                .iter()
                .map(ShapingSample::codepoints)
                .collect::<Vec<_>>(),
            &[&[65, 86][..], &[65, 0xfe0f, 86][..], &[102, 105][..]]
        );
    }

    #[test]
    fn malformed_fonts_fail_closed() {
        let request = SubsetRequest::new(0, [u32::from('A')], []);
        assert!(
            HarfBuzzSubsetEngine
                .subset(b"not a font", &request)
                .is_err()
        );
    }
}
