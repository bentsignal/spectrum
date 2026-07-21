use std::{
    collections::{BTreeMap, BTreeSet},
    ptr::null,
    slice,
    str::FromStr,
};

use hb_subset::{Blob, FontFace, Language, sys};

use crate::{SubsetError, SubsetRequest};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ShapedGlyph {
    pub(crate) glyph_id: u16,
    pub(crate) cluster: u32,
    pub(crate) flags: u32,
    pub(crate) x_advance: i32,
    pub(crate) y_advance: i32,
    pub(crate) x_offset: i32,
    pub(crate) y_offset: i32,
}

pub(crate) fn validate_parity(
    source: &[u8],
    output: &[u8],
    request: &SubsetRequest,
    glyph_mapping: &BTreeMap<u16, u16>,
) -> Result<BTreeSet<(u16, u16)>, SubsetError> {
    let mut shaped_glyphs = BTreeSet::new();
    for sample in request.shaping_samples() {
        let source_shape = shape(source, sample)?;
        let output_shape = shape(output, sample)?;
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

pub(crate) fn shape(bytes: &[u8], codepoints: &[u32]) -> Result<Vec<ShapedGlyph>, SubsetError> {
    if codepoints.is_empty() {
        return Err(SubsetError::new("shaping sample cannot be empty"));
    }
    if codepoints.len() > crate::limits::MAX_SHAPING_SCALARS_PER_SAMPLE {
        return Err(SubsetError::new(
            "shaping sample length exceeds resource limit",
        ));
    }
    let text_length = i32::try_from(codepoints.len())
        .map_err(|_| SubsetError::new("shaping sample is too long"))?;
    let blob = Blob::from_bytes(bytes)
        .map_err(|_| SubsetError::new("could not allocate HarfBuzz shaping blob"))?;
    let face = FontFace::new(blob)
        .map_err(|_| SubsetError::new("could not create HarfBuzz shaping face"))?;
    let font = HbFont::new(&face)?;
    let buffer = HbBuffer::new()?;
    let language = Language::from_str("und")
        .map_err(|_| SubsetError::new("could not intern deterministic shaping language"))?;

    unsafe {
        sys::hb_buffer_set_cluster_level(
            buffer.0,
            sys::hb_buffer_cluster_level_t_HB_BUFFER_CLUSTER_LEVEL_CHARACTERS,
        );
        sys::hb_buffer_add_codepoints(buffer.0, codepoints.as_ptr(), text_length, 0, text_length);
        if sys::hb_buffer_allocation_successful(buffer.0) == 0 {
            return Err(SubsetError::new(
                "HarfBuzz could not allocate shaping input",
            ));
        }
        sys::hb_buffer_set_language(buffer.0, language.as_raw());
        sys::hb_buffer_guess_segment_properties(buffer.0);
        sys::hb_shape(font.0, buffer.0, null(), 0);
        if sys::hb_buffer_allocation_successful(buffer.0) == 0 {
            return Err(SubsetError::new(
                "HarfBuzz could not allocate shaping output",
            ));
        }

        let mut info_length = 0_u32;
        let infos = sys::hb_buffer_get_glyph_infos(buffer.0, &mut info_length);
        let mut position_length = 0_u32;
        let positions = sys::hb_buffer_get_glyph_positions(buffer.0, &mut position_length);
        let length = checked_output_length(info_length, position_length)?;
        if length != 0 && (infos.is_null() || positions.is_null()) {
            return Err(SubsetError::new(
                "HarfBuzz returned inconsistent shaping arrays",
            ));
        }
        if length == 0 {
            return Ok(Vec::new());
        }
        let infos = slice::from_raw_parts(infos, length);
        let positions = slice::from_raw_parts(positions, length);
        infos
            .iter()
            .zip(positions)
            .map(|(info, position)| {
                Ok(ShapedGlyph {
                    glyph_id: u16::try_from(info.codepoint).map_err(|_| {
                        SubsetError::new("shaping produced an out-of-range glyph ID")
                    })?,
                    cluster: info.cluster,
                    flags: sys::hb_glyph_info_get_glyph_flags(info) as u32,
                    x_advance: position.x_advance,
                    y_advance: position.y_advance,
                    x_offset: position.x_offset,
                    y_offset: position.y_offset,
                })
            })
            .collect()
    }
}

fn checked_output_length(info_length: u32, position_length: u32) -> Result<usize, SubsetError> {
    if info_length != position_length {
        return Err(SubsetError::new(
            "HarfBuzz returned inconsistent shaping arrays",
        ));
    }
    let length = usize::try_from(info_length)
        .map_err(|_| SubsetError::new("shaping output length does not fit this platform"))?;
    if length > crate::limits::MAX_SHAPED_OUTPUT_GLYPHS {
        return Err(SubsetError::new(
            "HarfBuzz shaping output exceeds resource limit",
        ));
    }
    Ok(length)
}

struct HbFont(*mut sys::hb_font_t);

impl HbFont {
    fn new(face: &FontFace<'_>) -> Result<Self, SubsetError> {
        let font = unsafe { sys::hb_font_create(face.as_raw()) };
        if font.is_null() {
            return Err(SubsetError::new("could not allocate HarfBuzz font"));
        }
        let font = Self(font);
        unsafe {
            sys::hb_ot_font_set_funcs(font.0);
            let units_per_em = i32::try_from(sys::hb_face_get_upem(face.as_raw()))
                .map_err(|_| SubsetError::new("font units-per-em is out of range"))?;
            sys::hb_font_set_scale(font.0, units_per_em, units_per_em);
        }
        Ok(font)
    }
}

impl Drop for HbFont {
    fn drop(&mut self) {
        unsafe { sys::hb_font_destroy(self.0) };
    }
}

struct HbBuffer(*mut sys::hb_buffer_t);

impl HbBuffer {
    fn new() -> Result<Self, SubsetError> {
        let buffer = unsafe { sys::hb_buffer_create() };
        if buffer.is_null() {
            return Err(SubsetError::new("could not allocate HarfBuzz buffer"));
        }
        Ok(Self(buffer))
    }
}

impl Drop for HbBuffer {
    fn drop(&mut self) {
        unsafe { sys::hb_buffer_destroy(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use ttf_parser::Face;

    use super::*;

    const NOTO_SANS: &[u8] = include_bytes!("../tests/fonts/noto-sans-rich-rejected.ttf");

    #[test]
    fn real_layout_fixture_exercises_gsub_ligature_and_gpos_kerning() {
        let ligature = shape(NOTO_SANS, &[u32::from('f'), u32::from('f'), u32::from('i')])
            .expect("Noto ffi shapes");
        assert!(
            ligature.len() < 3,
            "fixture must substitute at least one ligature"
        );

        let positioned =
            shape(NOTO_SANS, &[u32::from('A'), u32::from('V')]).expect("Noto AV shapes");
        let face = Face::parse(NOTO_SANS, 0).expect("Noto fixture parses");
        let nominal_a = face
            .glyph_hor_advance(face.glyph_index('A').expect("Noto maps A"))
            .expect("Noto has horizontal metrics");
        assert_eq!(positioned.len(), 2);
        assert_ne!(
            positioned[0].x_advance,
            i32::from(nominal_a),
            "fixture must apply GPOS kerning to AV"
        );
    }

    #[test]
    fn oversized_input_and_output_lengths_fail_before_native_slices() {
        let oversized_input =
            vec![u32::from('A'); crate::limits::MAX_SHAPING_SCALARS_PER_SAMPLE + 1];
        let error = shape(b"not a font", &oversized_input).unwrap_err();
        assert!(error.to_string().contains("resource limit"));

        let oversized_output = u32::try_from(crate::limits::MAX_SHAPED_OUTPUT_GLYPHS + 1).unwrap();
        let error = checked_output_length(oversized_output, oversized_output).unwrap_err();
        assert!(error.to_string().contains("resource limit"));
        assert!(checked_output_length(4, 3).is_err());
    }
}
