use ttf_parser::Face;

use crate::{SubsetError, UnicodeVariationSequence, sfnt::SfntFace};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UvsKind {
    Default,
    NonDefault,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UvsMapping {
    pub(crate) kind: UvsKind,
    pub(crate) glyph_id: u16,
}

pub(crate) fn variation_mapping(
    sfnt: &SfntFace<'_>,
    face: &Face<'_>,
    sequence: UnicodeVariationSequence,
) -> Result<UvsMapping, SubsetError> {
    let base = char::from_u32(sequence.base_codepoint).ok_or_else(|| {
        SubsetError::new(format!(
            "U+{:04X} is not a Unicode scalar value",
            sequence.base_codepoint
        ))
    })?;
    let selector = char::from_u32(sequence.selector_codepoint).ok_or_else(|| {
        SubsetError::new(format!(
            "U+{:04X} is not a Unicode scalar value",
            sequence.selector_codepoint
        ))
    })?;
    if !is_variation_selector(selector) {
        return Err(SubsetError::new(format!(
            "U+{:04X} is not a Unicode variation selector",
            sequence.selector_codepoint
        )));
    }

    let cmap = sfnt
        .table(*b"cmap")
        .ok_or_else(|| SubsetError::new("font has no cmap table"))?;
    let table_count = usize::from(read_u16(cmap, 2)?);
    if table_count > crate::limits::MAX_CMAP_ENCODING_RECORDS {
        return Err(SubsetError::new(
            "cmap encoding-record count exceeds resource limit",
        ));
    }
    checked_slice(
        cmap,
        4,
        table_count
            .checked_mul(8)
            .ok_or_else(|| SubsetError::new("cmap encoding-record count overflow"))?,
    )?;
    let mut found = None;
    let mut previous_encoding = None;
    let mut saw_ineffective_format_14 = false;
    for index in 0..table_count {
        let record = 4 + index * 8;
        let encoding = (read_u16(cmap, record)?, read_u16(cmap, record + 2)?);
        if previous_encoding.is_some_and(|previous| previous >= encoding) {
            return Err(SubsetError::new(
                "cmap encoding records are not strictly sorted by platform and encoding",
            ));
        }
        previous_encoding = Some(encoding);
        let offset = usize::try_from(read_u32(cmap, record + 4)?)
            .map_err(|_| SubsetError::new("cmap subtable offset does not fit this platform"))?;
        if read_u16(cmap, offset)? != 14 {
            continue;
        }
        if encoding != (0, 5) {
            saw_ineffective_format_14 = true;
            continue;
        }
        let mapping = lookup_format_14(cmap, offset, sequence, face, base)?;
        if let Some(mapping) = mapping
            && found.replace(mapping).is_some()
        {
            return Err(SubsetError::new(
                "font contains duplicate requested cmap format 14 mappings",
            ));
        }
    }
    found.ok_or_else(|| {
        if saw_ineffective_format_14 {
            SubsetError::new(
                "requested cmap format 14 mapping is not in Unicode platform 0 encoding 5",
            )
        } else {
            SubsetError::new(format!(
                "font does not map requested U+{:04X} U+{:04X} variation sequence",
                sequence.base_codepoint, sequence.selector_codepoint
            ))
        }
    })
}

fn lookup_format_14(
    cmap: &[u8],
    offset: usize,
    sequence: UnicodeVariationSequence,
    face: &Face<'_>,
    base: char,
) -> Result<Option<UvsMapping>, SubsetError> {
    let length = usize::try_from(read_u32(cmap, offset + 2)?)
        .map_err(|_| SubsetError::new("cmap format 14 length does not fit this platform"))?;
    let subtable = checked_slice(cmap, offset, length)?;
    let selector_count = usize::try_from(read_u32(subtable, 6)?).map_err(|_| {
        SubsetError::new("cmap variation selector count does not fit this platform")
    })?;
    if selector_count > crate::limits::MAX_UVS_SELECTOR_RECORDS {
        return Err(SubsetError::new(
            "cmap variation selector count exceeds resource limit",
        ));
    }
    checked_slice(
        subtable,
        10,
        selector_count
            .checked_mul(11)
            .ok_or_else(|| SubsetError::new("cmap variation-selector count overflow"))?,
    )?;
    for index in 0..selector_count {
        let record = 10 + index * 11;
        if read_u24(subtable, record)? != sequence.selector_codepoint {
            continue;
        }
        let default_offset = read_u32(subtable, record + 3)?;
        let nondefault_offset = read_u32(subtable, record + 7)?;
        let is_default = default_offset != 0
            && default_uvs_contains(
                subtable,
                usize::try_from(default_offset).map_err(|_| {
                    SubsetError::new("default UVS offset does not fit this platform")
                })?,
                sequence.base_codepoint,
            )?;
        let nondefault = if nondefault_offset == 0 {
            None
        } else {
            nondefault_uvs_glyph(
                subtable,
                usize::try_from(nondefault_offset).map_err(|_| {
                    SubsetError::new("non-default UVS offset does not fit this platform")
                })?,
                sequence.base_codepoint,
            )?
        };
        return match (is_default, nondefault) {
            (true, Some(_)) => Err(SubsetError::new(
                "cmap format 14 maps one sequence as both default and non-default",
            )),
            (true, None) => face
                .glyph_index(base)
                .map(|glyph| {
                    Some(UvsMapping {
                        kind: UvsKind::Default,
                        glyph_id: glyph.0,
                    })
                })
                .ok_or_else(|| SubsetError::new("default UVS has no base cmap glyph")),
            (false, Some(glyph_id)) => Ok(Some(UvsMapping {
                kind: UvsKind::NonDefault,
                glyph_id: validate_glyph_id(face, glyph_id)?,
            })),
            (false, None) => Ok(None),
        };
    }
    Ok(None)
}

fn default_uvs_contains(
    subtable: &[u8],
    offset: usize,
    base_codepoint: u32,
) -> Result<bool, SubsetError> {
    let count = usize::try_from(read_u32(subtable, offset)?)
        .map_err(|_| SubsetError::new("default UVS range count does not fit this platform"))?;
    if count > crate::limits::MAX_DEFAULT_UVS_RANGES {
        return Err(SubsetError::new(
            "default UVS range count exceeds resource limit",
        ));
    }
    checked_slice(
        subtable,
        offset + 4,
        count
            .checked_mul(4)
            .ok_or_else(|| SubsetError::new("default UVS range count overflow"))?,
    )?;
    for index in 0..count {
        let record = offset + 4 + index * 4;
        let start = read_u24(subtable, record)?;
        let end = start
            .checked_add(u32::from(subtable[record + 3]))
            .ok_or_else(|| SubsetError::new("default UVS range overflow"))?;
        if (start..=end).contains(&base_codepoint) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn nondefault_uvs_glyph(
    subtable: &[u8],
    offset: usize,
    base_codepoint: u32,
) -> Result<Option<u16>, SubsetError> {
    let count = usize::try_from(read_u32(subtable, offset)?)
        .map_err(|_| SubsetError::new("non-default UVS count does not fit this platform"))?;
    if count > crate::limits::MAX_NONDEFAULT_UVS_MAPPINGS {
        return Err(SubsetError::new(
            "non-default UVS count exceeds resource limit",
        ));
    }
    checked_slice(
        subtable,
        offset + 4,
        count
            .checked_mul(5)
            .ok_or_else(|| SubsetError::new("non-default UVS count overflow"))?,
    )?;
    for index in 0..count {
        let record = offset + 4 + index * 5;
        if read_u24(subtable, record)? == base_codepoint {
            return Ok(Some(read_u16(subtable, record + 3)?));
        }
    }
    Ok(None)
}

fn is_variation_selector(character: char) -> bool {
    matches!(u32::from(character), 0xfe00..=0xfe0f | 0xe0100..=0xe01ef)
}

fn validate_glyph_id(face: &Face<'_>, glyph_id: u16) -> Result<u16, SubsetError> {
    if glyph_id >= face.number_of_glyphs() {
        return Err(SubsetError::new(format!(
            "cmap format 14 references out-of-range glyph {glyph_id}"
        )));
    }
    Ok(glyph_id)
}

fn checked_slice(bytes: &[u8], offset: usize, length: usize) -> Result<&[u8], SubsetError> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| SubsetError::new("font table range overflow"))?;
    bytes
        .get(offset..end)
        .ok_or_else(|| SubsetError::new("truncated font table data"))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, SubsetError> {
    let value: [u8; 2] = checked_slice(bytes, offset, 2)?
        .try_into()
        .map_err(|_| SubsetError::new("truncated font table data"))?;
    Ok(u16::from_be_bytes(value))
}

fn read_u24(bytes: &[u8], offset: usize) -> Result<u32, SubsetError> {
    let value = checked_slice(bytes, offset, 3)?;
    Ok((u32::from(value[0]) << 16) | (u32::from(value[1]) << 8) | u32::from(value[2]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, SubsetError> {
    let value: [u8; 4] = checked_slice(bytes, offset, 4)?
        .try_into()
        .map_err(|_| SubsetError::new("truncated font table data"))?;
    Ok(u32::from_be_bytes(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_14_default_and_nondefault_tables_are_distinct() {
        let mut default = vec![0; 8];
        default[0..4].copy_from_slice(&1_u32.to_be_bytes());
        default[4..7].copy_from_slice(&[0, 0, 65]);
        assert!(default_uvs_contains(&default, 0, 65).unwrap());
        assert!(!default_uvs_contains(&default, 0, 66).unwrap());

        let mut nondefault = vec![0; 9];
        nondefault[0..4].copy_from_slice(&1_u32.to_be_bytes());
        nondefault[4..7].copy_from_slice(&[0, 0, 65]);
        nondefault[7..9].copy_from_slice(&42_u16.to_be_bytes());
        assert_eq!(nondefault_uvs_glyph(&nondefault, 0, 65).unwrap(), Some(42));
        assert_eq!(nondefault_uvs_glyph(&nondefault, 0, 66).unwrap(), None);
    }
}
