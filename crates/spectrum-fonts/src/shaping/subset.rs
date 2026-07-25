use std::collections::{BTreeMap, BTreeSet};

use super::{HarfBuzzShaper, ShapeRequest, ShapedGlyph};
use crate::{ShapingSample, SubsetError, SubsetRequest};

pub(crate) fn validate_parity(
    source: &[u8],
    output: &[u8],
    request: &SubsetRequest,
    glyph_mapping: &BTreeMap<u16, u16>,
) -> Result<BTreeSet<(u16, u16)>, SubsetError> {
    let mut shaped_glyphs = BTreeSet::new();
    for sample in request.shaping_samples() {
        let source_shape = shape_subset_sample(source, sample)?;
        let output_shape = shape_subset_sample(output, sample)?;
        if source_shape.len() != output_shape.len() {
            return Err(SubsetError::new(
                "subset candidate changed shaped glyph count",
            ));
        }
        for (source_glyph, output_glyph) in source_shape.iter().zip(&output_shape) {
            let mapped_glyph = glyph_mapping.get(&source_glyph.glyph_id).ok_or_else(|| {
                SubsetError::new(format!(
                    "layout closure omitted shaped source glyph {}",
                    source_glyph.glyph_id
                ))
            })?;
            if *mapped_glyph != output_glyph.glyph_id
                || source_glyph.cluster != output_glyph.cluster
                || source_glyph.flags != output_glyph.flags
                || source_glyph.x_advance != output_glyph.x_advance
                || source_glyph.y_advance != output_glyph.y_advance
                || source_glyph.x_offset != output_glyph.x_offset
                || source_glyph.y_offset != output_glyph.y_offset
            {
                return Err(SubsetError::new(
                    "subset candidate changed default-feature HarfBuzz shaping",
                ));
            }
            shaped_glyphs.insert((source_glyph.glyph_id, output_glyph.glyph_id));
            if shaped_glyphs.len() > crate::limits::MAX_SHAPED_CLOSURE_GLYPHS {
                return Err(SubsetError::new(
                    "shaped closure glyph count exceeds resource limit",
                ));
            }
        }
    }
    Ok(shaped_glyphs)
}

fn shape_subset_sample(
    bytes: &[u8],
    sample: &ShapingSample,
) -> Result<Vec<ShapedGlyph>, SubsetError> {
    let codepoints = sample.codepoints();
    if codepoints.is_empty() {
        return Err(SubsetError::new("shaping sample cannot be empty"));
    }
    if codepoints.len() > crate::limits::MAX_SHAPING_SCALARS_PER_SAMPLE {
        return Err(SubsetError::new(
            "shaping sample length exceeds resource limit",
        ));
    }
    let text = codepoints
        .iter()
        .map(|codepoint| {
            char::from_u32(*codepoint).ok_or_else(|| {
                SubsetError::new(format!("U+{codepoint:04X} is not a Unicode scalar value"))
            })
        })
        .collect::<Result<String, _>>()?;
    let shaper = HarfBuzzShaper::new(bytes, 0).map_err(SubsetError::from_shape)?;
    let mut request = ShapeRequest::new(&text);
    if let Some(range) = sample.item_range_bytes() {
        request = request.item_range(range);
    }
    if let Some(direction) = sample.requested_direction() {
        request = request.direction(direction);
    }
    if let Some(script) = sample.requested_script() {
        request = request.script(script);
    }
    if let Some(language) = sample.requested_language() {
        request = request.language(language);
    }
    shaper
        .shape(&request)
        .map(|run| run.glyphs().to_vec())
        .map_err(SubsetError::from_shape)
}
