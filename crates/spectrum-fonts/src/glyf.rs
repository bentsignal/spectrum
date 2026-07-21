use crate::{SubsetError, sfnt::SfntFace};

pub(crate) fn validate_unhinted(sfnt: &SfntFace<'_>, glyph_count: u16) -> Result<(), SubsetError> {
    for glyph_id in 0..glyph_count {
        if !glyph_instructions(sfnt, glyph_id)?.is_empty() {
            return Err(SubsetError::new(format!(
                "TrueType hint instructions in glyph {glyph_id} are outside the candidate envelope"
            )));
        }
    }
    Ok(())
}

fn glyph_instructions<'a>(sfnt: &SfntFace<'a>, glyph_id: u16) -> Result<&'a [u8], SubsetError> {
    let head = sfnt
        .table(*b"head")
        .ok_or_else(|| SubsetError::new("font has no head table"))?;
    let maxp = sfnt
        .table(*b"maxp")
        .ok_or_else(|| SubsetError::new("font has no maxp table"))?;
    let loca = sfnt
        .table(*b"loca")
        .ok_or_else(|| SubsetError::new("font has no loca table"))?;
    let glyf = sfnt
        .table(*b"glyf")
        .ok_or_else(|| SubsetError::new("font has no glyf table"))?;
    let glyph_count = read_u16(maxp, 4)?;
    if glyph_id >= glyph_count {
        return Err(SubsetError::new(format!(
            "glyph {glyph_id} exceeds maxp glyph count {glyph_count}"
        )));
    }
    let long_loca = match read_i16(head, 50)? {
        0 => false,
        1 => true,
        value => {
            return Err(SubsetError::new(format!(
                "unsupported head indexToLocFormat {value}"
            )));
        }
    };
    let start = loca_offset(loca, glyph_id, long_loca)?;
    let end = loca_offset(loca, glyph_id + 1, long_loca)?;
    if end < start {
        return Err(SubsetError::new("decreasing loca offsets"));
    }
    let glyph = checked_slice(glyf, start, end - start)?;
    if glyph.is_empty() {
        return Ok(&[]);
    }
    let contour_count = read_i16(glyph, 0)?;
    checked_slice(glyph, 0, 10)?;
    if contour_count >= 0 {
        let contour_count = usize::try_from(contour_count)
            .map_err(|_| SubsetError::new("negative simple glyph contour count"))?;
        let contour_bytes = contour_count
            .checked_mul(2)
            .ok_or_else(|| SubsetError::new("simple glyph contour count overflow"))?;
        let instruction_length_offset = 10_usize
            .checked_add(contour_bytes)
            .ok_or_else(|| SubsetError::new("simple glyph contour count overflow"))?;
        let instruction_length = usize::from(read_u16(glyph, instruction_length_offset)?);
        return checked_slice(glyph, instruction_length_offset + 2, instruction_length);
    }

    compound_instructions(glyph)
}

fn loca_offset(loca: &[u8], glyph_id: u16, long: bool) -> Result<usize, SubsetError> {
    if long {
        usize::try_from(read_u32(loca, usize::from(glyph_id) * 4)?)
            .map_err(|_| SubsetError::new("long loca offset does not fit this platform"))
    } else {
        Ok(usize::from(read_u16(loca, usize::from(glyph_id) * 2)?) * 2)
    }
}

fn compound_instructions(glyph: &[u8]) -> Result<&[u8], SubsetError> {
    const ARG_WORDS: u16 = 0x0001;
    const HAVE_SCALE: u16 = 0x0008;
    const MORE_COMPONENTS: u16 = 0x0020;
    const HAVE_XY_SCALE: u16 = 0x0040;
    const HAVE_TWO_BY_TWO: u16 = 0x0080;
    const HAVE_INSTRUCTIONS: u16 = 0x0100;

    let mut offset = 10_usize;
    let mut has_instructions = false;
    loop {
        let flags = read_u16(glyph, offset)?;
        checked_slice(glyph, offset, 4)?;
        has_instructions |= flags & HAVE_INSTRUCTIONS != 0;
        offset += 4;
        offset += if flags & ARG_WORDS != 0 { 4 } else { 2 };
        offset += if flags & HAVE_TWO_BY_TWO != 0 {
            8
        } else if flags & HAVE_XY_SCALE != 0 {
            4
        } else if flags & HAVE_SCALE != 0 {
            2
        } else {
            0
        };
        checked_slice(glyph, 0, offset)?;
        if flags & MORE_COMPONENTS == 0 {
            break;
        }
    }
    if !has_instructions {
        return Ok(&[]);
    }
    let instruction_length = usize::from(read_u16(glyph, offset)?);
    checked_slice(glyph, offset + 2, instruction_length)
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

fn read_i16(bytes: &[u8], offset: usize) -> Result<i16, SubsetError> {
    let value: [u8; 2] = checked_slice(bytes, offset, 2)?
        .try_into()
        .map_err(|_| SubsetError::new("truncated font table data"))?;
    Ok(i16::from_be_bytes(value))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, SubsetError> {
    let value: [u8; 4] = checked_slice(bytes, offset, 4)?
        .try_into()
        .map_err(|_| SubsetError::new("truncated font table data"))?;
    Ok(u32::from_be_bytes(value))
}
