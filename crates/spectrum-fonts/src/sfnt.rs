use std::collections::BTreeSet;

use crate::{SubsetError, limits::MAX_SFNT_TABLES};

const TTC_TAG: [u8; 4] = *b"ttcf";
const TRUE_TYPE: [u8; 4] = [0, 1, 0, 0];

#[derive(Clone, Debug)]
pub(crate) struct SfntFace<'a> {
    bytes: &'a [u8],
    records: Vec<TableRecord>,
}

#[derive(Clone, Copy, Debug)]
struct TableRecord {
    tag: [u8; 4],
    offset: usize,
    length: usize,
}

impl<'a> SfntFace<'a> {
    pub(crate) fn parse(bytes: &'a [u8], face_index: u32) -> Result<Self, SubsetError> {
        if read_array::<4>(bytes, 0)? == TTC_TAG {
            return Err(SubsetError::new(
                "font collections are not accepted until standalone-face baselines are validated",
            ));
        }
        if face_index != 0 {
            return Err(SubsetError::new(format!(
                "face index {face_index} is invalid for a standalone font"
            )));
        }
        let sfnt_offset = 0;
        let signature = read_array::<4>(bytes, sfnt_offset)?;
        if signature != TRUE_TYPE {
            return Err(SubsetError::new(format!(
                "{} outlines are not enabled before cross-platform corpus validation",
                display_tag(signature)
            )));
        }
        let table_count = usize::from(read_u16(bytes, sfnt_offset + 4)?);
        if table_count > MAX_SFNT_TABLES {
            return Err(SubsetError::new("SFNT table count exceeds resource limit"));
        }
        let records_offset = sfnt_offset
            .checked_add(12)
            .ok_or_else(|| SubsetError::new("SFNT directory offset overflow"))?;
        let records_bytes = table_count
            .checked_mul(16)
            .ok_or_else(|| SubsetError::new("SFNT table count overflow"))?;
        checked_slice(bytes, records_offset, records_bytes)?;

        let mut records = Vec::with_capacity(table_count);
        let mut tags = BTreeSet::new();
        for index in 0..table_count {
            let record_offset = records_offset + index * 16;
            let tag = read_array::<4>(bytes, record_offset)?;
            if !tags.insert(tag) {
                return Err(SubsetError::new(format!(
                    "duplicate SFNT table {}",
                    display_tag(tag)
                )));
            }
            let offset = usize::try_from(read_u32(bytes, record_offset + 8)?)
                .map_err(|_| SubsetError::new("SFNT table offset does not fit this platform"))?;
            let length = usize::try_from(read_u32(bytes, record_offset + 12)?)
                .map_err(|_| SubsetError::new("SFNT table length does not fit this platform"))?;
            checked_slice(bytes, offset, length)?;
            records.push(TableRecord {
                tag,
                offset,
                length,
            });
        }
        Ok(Self { bytes, records })
    }

    pub(crate) fn has(&self, tag: [u8; 4]) -> bool {
        self.records.iter().any(|record| record.tag == tag)
    }

    pub(crate) fn table(&self, tag: [u8; 4]) -> Option<&'a [u8]> {
        let record = self.records.iter().find(|record| record.tag == tag)?;
        self.bytes.get(record.offset..record.offset + record.length)
    }

    pub(crate) fn tags(&self) -> impl Iterator<Item = [u8; 4]> + '_ {
        self.records.iter().map(|record| record.tag)
    }
}

fn checked_slice(bytes: &[u8], offset: usize, length: usize) -> Result<&[u8], SubsetError> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| SubsetError::new("SFNT range overflow"))?;
    bytes
        .get(offset..end)
        .ok_or_else(|| SubsetError::new("truncated SFNT data"))
}

fn read_array<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], SubsetError> {
    checked_slice(bytes, offset, N)?
        .try_into()
        .map_err(|_| SubsetError::new("truncated SFNT data"))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, SubsetError> {
    Ok(u16::from_be_bytes(read_array(bytes, offset)?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, SubsetError> {
    Ok(u32::from_be_bytes(read_array(bytes, offset)?))
}

pub(crate) fn display_tag(tag: [u8; 4]) -> String {
    tag.into_iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || byte == b' ' {
                char::from(byte)
            } else {
                '\u{fffd}'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_sfnt_is_rejected_without_panicking() {
        assert!(SfntFace::parse(&[], 0).is_err());
        assert!(SfntFace::parse(&TRUE_TYPE, 0).is_err());
        assert!(SfntFace::parse(b"wOFF malformed", 0).is_err());
    }

    #[test]
    fn nonzero_face_is_rejected_for_single_face_data() {
        let mut header = vec![0; 12];
        header[0..4].copy_from_slice(&TRUE_TYPE);
        assert!(SfntFace::parse(&header, 1).is_err());
    }

    #[test]
    fn collections_are_rejected_before_face_selection() {
        let mut header = vec![0; 16];
        header[0..4].copy_from_slice(&TTC_TAG);
        header[8..12].copy_from_slice(&1_u32.to_be_bytes());
        header[12..16].copy_from_slice(&16_u32.to_be_bytes());
        assert!(SfntFace::parse(&header, 0).is_err());
    }

    #[test]
    fn oversized_table_directories_are_rejected_before_allocation() {
        let mut header = vec![0; 12];
        header[0..4].copy_from_slice(&TRUE_TYPE);
        header[4..6].copy_from_slice(
            &u16::try_from(crate::limits::MAX_SFNT_TABLES + 1)
                .unwrap()
                .to_be_bytes(),
        );
        let error = SfntFace::parse(&header, 0).unwrap_err();
        assert!(error.to_string().contains("resource limit"));
    }
}
