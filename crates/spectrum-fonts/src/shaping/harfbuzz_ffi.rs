//! RAII boundary around the pinned bundled HarfBuzz C API.

use std::{
    collections::BTreeSet,
    ffi::{CStr, c_char},
    ptr, slice,
    str::FromStr,
};

use hb_subset::{Blob, FontFace, Language, sys};
use sha2::{Digest, Sha256};
use ttf_parser::Face;

use super::{
    GlyphFlags, MAX_SHAPE_GLYPHS, OpenTypeFeature, Script, ShapeRequest, ShapedGlyph, ShapedRun,
    TextDirection, VariationCoordinate, validate_request,
};
use crate::ShapeError;

pub(super) struct NativeShaper<'font> {
    face: FontFace<'font>,
    face_index: u32,
    units_per_em: u16,
    ascender: i16,
    descender: i16,
    line_gap: i16,
    face_identity: [u8; 32],
    glyph_count: u32,
}

impl<'font> NativeShaper<'font> {
    pub(super) fn new(bytes: &'font [u8], face_index: u32) -> Result<Self, ShapeError> {
        if bytes.is_empty() {
            return Err(ShapeError::new("font data cannot be empty"));
        }
        crate::limits::validate_source_size(bytes)
            .map_err(|error| ShapeError::new(error.to_string()))?;
        let parsed = Face::parse(bytes, face_index)
            .map_err(|_| ShapeError::new("font face is not valid OpenType data"))?;
        let units_per_em = parsed.units_per_em();
        let ascender = parsed.ascender();
        let descender = parsed.descender();
        let line_gap = parsed.line_gap();
        let parsed_glyph_count = u32::from(parsed.number_of_glyphs());
        if parsed_glyph_count == 0 {
            return Err(ShapeError::new("font face contains no glyphs"));
        }

        let blob = Blob::from_bytes(bytes)
            .map_err(|_| ShapeError::new("could not allocate HarfBuzz font blob"))?;
        let face = FontFace::new_with_index(blob, face_index)
            .map_err(|_| ShapeError::new("could not create HarfBuzz font face"))?;
        let native_glyph_count = u32::try_from(face.glyph_count())
            .map_err(|_| ShapeError::new("font glyph count is out of range"))?;
        if native_glyph_count != parsed_glyph_count {
            return Err(ShapeError::new(
                "font parsers disagree about the face glyph count",
            ));
        }
        let native_units_per_em = unsafe { sys::hb_face_get_upem(face.as_raw()) };
        if native_units_per_em != u32::from(units_per_em) {
            return Err(ShapeError::new(
                "font parsers disagree about face units-per-em",
            ));
        }
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hasher.update(face_index.to_be_bytes());
        let face_identity = hasher.finalize().into();
        Ok(Self {
            face,
            face_index,
            units_per_em,
            ascender,
            descender,
            line_gap,
            face_identity,
            glyph_count: parsed_glyph_count,
        })
    }

    pub(super) fn face_index(&self) -> u32 {
        self.face_index
    }

    pub(super) fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    pub(super) fn shape(&self, request: &ShapeRequest<'_>) -> Result<ShapedRun, ShapeError> {
        validate_request(request)?;
        let text = request.text();
        let item = request.selected_item_range();
        let text_length =
            i32::try_from(text.len()).map_err(|_| ShapeError::new("shaping text is too long"))?;
        let item_start = u32::try_from(item.start)
            .map_err(|_| ShapeError::new("shaping item start is out of range"))?;
        let item_length = i32::try_from(item.end - item.start)
            .map_err(|_| ShapeError::new("shaping item length is out of range"))?;
        let scalar_count = u32::try_from(text[item.clone()].chars().count())
            .map_err(|_| ShapeError::new("shaping scalar count is out of range"))?;
        let buffer = HbBuffer::new()?;
        let font = HbFont::new(&self.face, self.units_per_em)?;
        font.set_variations(request.variation_coordinates())?;
        let language_text = request.requested_language().unwrap_or("und");
        let language = Language::from_str(language_text)
            .map_err(|_| ShapeError::new("could not intern shaping language"))?;
        let raw_features = request
            .feature_overrides()
            .iter()
            .copied()
            .map(raw_feature)
            .collect::<Vec<_>>();

        unsafe {
            if sys::hb_buffer_pre_allocate(buffer.0, scalar_count) == 0
                || sys::hb_buffer_allocation_successful(buffer.0) == 0
            {
                return Err(ShapeError::new(
                    "HarfBuzz could not preallocate shaping input",
                ));
            }
            sys::hb_buffer_set_cluster_level(
                buffer.0,
                sys::hb_buffer_cluster_level_t_HB_BUFFER_CLUSTER_LEVEL_CHARACTERS,
            );
            sys::hb_buffer_add_utf8(
                buffer.0,
                text.as_ptr().cast::<c_char>(),
                text_length,
                item_start,
                item_length,
            );
            if sys::hb_buffer_allocation_successful(buffer.0) == 0 {
                return Err(ShapeError::new("HarfBuzz could not allocate shaping input"));
            }
            if let Some(direction) = request.requested_direction() {
                sys::hb_buffer_set_direction(buffer.0, raw_direction(direction));
            }
            if let Some(script) = request.requested_script() {
                sys::hb_buffer_set_script(buffer.0, raw_script(script));
            }
            // Always set a deterministic language before guessing. HarfBuzz's
            // own omitted-language fallback reads the process locale.
            sys::hb_buffer_set_language(buffer.0, language.as_raw());
            sys::hb_buffer_guess_segment_properties(buffer.0);
            let features = if raw_features.is_empty() {
                ptr::null()
            } else {
                raw_features.as_ptr()
            };
            sys::hb_shape(
                font.0,
                buffer.0,
                features,
                u32::try_from(raw_features.len())
                    .map_err(|_| ShapeError::new("feature count is out of range"))?,
            );
            if sys::hb_buffer_allocation_successful(buffer.0) == 0 {
                return Err(ShapeError::new(
                    "HarfBuzz could not allocate shaping output",
                ));
            }
        }

        let direction = resolved_direction(unsafe { sys::hb_buffer_get_direction(buffer.0) })?;
        let script = resolved_script(unsafe { sys::hb_buffer_get_script(buffer.0) });
        let language = resolved_language(unsafe { sys::hb_buffer_get_language(buffer.0) })?;
        let glyphs = self.read_glyphs(&buffer, text, item.clone())?;
        Ok(ShapedRun {
            glyphs,
            direction,
            script,
            language,
            direction_was_guessed: request.requested_direction().is_none(),
            script_was_guessed: request.requested_script().is_none(),
            language_was_defaulted: request.requested_language().is_none(),
            units_per_em: self.units_per_em,
            ascender: self.ascender,
            descender: self.descender,
            line_gap: self.line_gap,
            face_index: self.face_index,
            face_identity: self.face_identity,
            source_text_bytes: u32::try_from(text.len())
                .map_err(|_| ShapeError::new("shaping text is too long"))?,
            item_start,
            item_end: u32::try_from(item.end)
                .map_err(|_| ShapeError::new("shaping item end is out of range"))?,
        })
    }

    fn read_glyphs(
        &self,
        buffer: &HbBuffer,
        text: &str,
        item: std::ops::Range<usize>,
    ) -> Result<Vec<ShapedGlyph>, ShapeError> {
        unsafe {
            let mut info_length = 0_u32;
            let infos = sys::hb_buffer_get_glyph_infos(buffer.0, &mut info_length);
            let mut position_length = 0_u32;
            let positions = sys::hb_buffer_get_glyph_positions(buffer.0, &mut position_length);
            let length = checked_output_length(info_length, position_length)?;
            if length != 0 && (infos.is_null() || positions.is_null()) {
                return Err(ShapeError::new(
                    "HarfBuzz returned inconsistent shaping arrays",
                ));
            }
            if length == 0 {
                return Ok(Vec::new());
            }
            let infos = slice::from_raw_parts(infos, length);
            let positions = slice::from_raw_parts(positions, length);
            let mut cluster_boundaries = infos
                .iter()
                .map(|info| info.cluster)
                .collect::<BTreeSet<_>>();
            cluster_boundaries.insert(
                u32::try_from(item.end)
                    .map_err(|_| ShapeError::new("shaping item end is out of range"))?,
            );
            let cluster_boundaries = cluster_boundaries.into_iter().collect::<Vec<_>>();
            infos
                .iter()
                .zip(positions)
                .map(|(info, position)| {
                    let glyph_id = u16::try_from(info.codepoint)
                        .map_err(|_| ShapeError::new("shaping produced invalid glyph ID"))?;
                    if u32::from(glyph_id) >= self.glyph_count {
                        return Err(ShapeError::new(
                            "shaping produced glyph ID outside the selected face",
                        ));
                    }
                    let cluster = usize::try_from(info.cluster)
                        .map_err(|_| ShapeError::new("shaping cluster is out of range"))?;
                    if cluster < item.start
                        || cluster >= item.end
                        || !text.is_char_boundary(cluster)
                    {
                        return Err(ShapeError::new(
                            "shaping produced an invalid UTF-8 byte cluster",
                        ));
                    }
                    let cluster_end = cluster_boundaries
                        .iter()
                        .copied()
                        .find(|boundary| *boundary > info.cluster)
                        .ok_or_else(|| {
                            ShapeError::new("shaping produced an unterminated source cluster")
                        })?;
                    Ok(ShapedGlyph {
                        glyph_id,
                        cluster: info.cluster,
                        cluster_end,
                        flags: GlyphFlags::from_bits(
                            sys::hb_glyph_info_get_glyph_flags(info) as u32
                        )?,
                        x_advance: position.x_advance,
                        y_advance: position.y_advance,
                        x_offset: position.x_offset,
                        y_offset: position.y_offset,
                    })
                })
                .collect()
        }
    }
}

fn raw_feature(feature: OpenTypeFeature) -> sys::hb_feature_t {
    let (start, end) = feature.raw_range();
    sys::hb_feature_t {
        tag: u32::from_be_bytes(feature.tag()),
        value: feature.value(),
        start,
        end,
    }
}

fn raw_direction(direction: TextDirection) -> sys::hb_direction_t {
    match direction {
        TextDirection::LeftToRight => sys::hb_direction_t_HB_DIRECTION_LTR,
        TextDirection::RightToLeft => sys::hb_direction_t_HB_DIRECTION_RTL,
    }
}

fn resolved_direction(raw: sys::hb_direction_t) -> Result<TextDirection, ShapeError> {
    match raw {
        sys::hb_direction_t_HB_DIRECTION_LTR => Ok(TextDirection::LeftToRight),
        sys::hb_direction_t_HB_DIRECTION_RTL => Ok(TextDirection::RightToLeft),
        _ => Err(ShapeError::new(
            "HarfBuzz resolved an unsupported shaping direction",
        )),
    }
}

fn raw_script(script: Script) -> sys::hb_script_t {
    unsafe { sys::hb_script_from_iso15924_tag(u32::from_be_bytes(script.iso15924())) }
}

fn resolved_script(raw: sys::hb_script_t) -> Option<Script> {
    if raw == sys::hb_script_t_HB_SCRIPT_INVALID {
        return None;
    }
    let tag = unsafe { sys::hb_script_to_iso15924_tag(raw) };
    Some(Script::from_resolved_tag(tag.to_be_bytes()))
}

fn resolved_language(raw: sys::hb_language_t) -> Result<String, ShapeError> {
    if raw.is_null() {
        return Err(ShapeError::new(
            "HarfBuzz returned an invalid shaping language",
        ));
    }
    let language = unsafe { sys::hb_language_to_string(raw) };
    if language.is_null() {
        return Err(ShapeError::new(
            "HarfBuzz returned an invalid shaping language",
        ));
    }
    let language = unsafe { CStr::from_ptr(language) };
    let language = language
        .to_str()
        .map_err(|_| ShapeError::new("HarfBuzz returned non-UTF-8 shaping language"))?;
    Ok(language.to_owned())
}

fn checked_output_length(info_length: u32, position_length: u32) -> Result<usize, ShapeError> {
    if info_length != position_length {
        return Err(ShapeError::new(
            "HarfBuzz returned inconsistent shaping arrays",
        ));
    }
    let length = usize::try_from(info_length)
        .map_err(|_| ShapeError::new("shaping output length does not fit this platform"))?;
    if length > MAX_SHAPE_GLYPHS {
        return Err(ShapeError::new(
            "HarfBuzz shaping output exceeds resource limit",
        ));
    }
    Ok(length)
}

struct HbFont(*mut sys::hb_font_t);

impl HbFont {
    fn new(face: &FontFace<'_>, units_per_em: u16) -> Result<Self, ShapeError> {
        let font = unsafe { sys::hb_font_create(face.as_raw()) };
        if font.is_null() {
            return Err(ShapeError::new("could not allocate HarfBuzz font"));
        }
        let font = Self(font);
        unsafe {
            sys::hb_ot_font_set_funcs(font.0);
            let scale = i32::from(units_per_em);
            sys::hb_font_set_scale(font.0, scale, scale);
        }
        Ok(font)
    }

    fn set_variations(&self, coordinates: &[VariationCoordinate]) -> Result<(), ShapeError> {
        let raw = coordinates
            .iter()
            .copied()
            .map(|coordinate| sys::hb_variation_t {
                tag: u32::from_be_bytes(coordinate.tag()),
                value: coordinate.value(),
            })
            .collect::<Vec<_>>();
        let length = u32::try_from(raw.len())
            .map_err(|_| ShapeError::new("variable-font coordinate count is out of range"))?;
        unsafe {
            sys::hb_font_set_variations(
                self.0,
                if raw.is_empty() {
                    ptr::null()
                } else {
                    raw.as_ptr()
                },
                length,
            );
        }
        Ok(())
    }
}

impl Drop for HbFont {
    fn drop(&mut self) {
        unsafe { sys::hb_font_destroy(self.0) };
    }
}

struct HbBuffer(*mut sys::hb_buffer_t);

impl HbBuffer {
    fn new() -> Result<Self, ShapeError> {
        let buffer = unsafe { sys::hb_buffer_create() };
        if buffer.is_null() {
            return Err(ShapeError::new("could not allocate HarfBuzz buffer"));
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
    use super::*;

    #[test]
    fn output_lengths_fail_closed_before_native_slices() {
        let oversized = u32::try_from(MAX_SHAPE_GLYPHS + 1).unwrap();
        assert!(
            checked_output_length(oversized, oversized)
                .unwrap_err()
                .to_string()
                .contains("resource limit")
        );
        assert!(checked_output_length(4, 3).is_err());
    }
}
