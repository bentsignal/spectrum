use std::collections::BTreeMap;

pub fn with_default_and_nondefault_uvs(source: &[u8], alternate_glyph: u16) -> Vec<u8> {
    with_uvs_encoding(source, alternate_glyph, (0, 5), true)
}

pub fn with_non_unicode_uvs(source: &[u8], alternate_glyph: u16) -> Vec<u8> {
    with_uvs_encoding(source, alternate_glyph, (3, 5), true)
}

pub fn with_unsorted_uvs_directory(source: &[u8], alternate_glyph: u16) -> Vec<u8> {
    with_uvs_encoding(source, alternate_glyph, (0, 5), false)
}

fn with_uvs_encoding(
    source: &[u8],
    alternate_glyph: u16,
    encoding: (u16, u16),
    sorted: bool,
) -> Vec<u8> {
    let mut tables = parse_tables(source);
    let cmap = tables.get_mut(b"cmap").expect("fixture has cmap");
    *cmap = append_format_14(cmap, alternate_glyph, encoding, sorted);
    rebuild_sfnt(source, tables)
}

pub fn without_tables(source: &[u8], dropped: &[[u8; 4]]) -> Vec<u8> {
    let mut tables = parse_tables(source);
    for tag in dropped {
        assert!(tables.remove(tag).is_some(), "fixture has no table {tag:?}");
    }
    rebuild_sfnt(source, tables)
}

pub fn glyph_contour_count(source: &[u8], glyph_id: u16) -> i16 {
    let glyph = glyph_data(source, glyph_id);
    assert!(!glyph.is_empty(), "test helper expects an outline glyph");
    i16::from_be_bytes(glyph[0..2].try_into().unwrap())
}

fn glyph_data(source: &[u8], glyph_id: u16) -> &[u8] {
    let tables = parse_table_ranges(source);
    let head = table(source, &tables, *b"head");
    let loca = table(source, &tables, *b"loca");
    let glyf = table(source, &tables, *b"glyf");
    let long_loca = i16::from_be_bytes(head[50..52].try_into().unwrap()) == 1;
    let loca_offset = |id: u16| {
        if long_loca {
            usize::try_from(read_u32(loca, usize::from(id) * 4)).unwrap()
        } else {
            usize::from(read_u16(loca, usize::from(id) * 2)) * 2
        }
    };
    let start = loca_offset(glyph_id);
    let end = loca_offset(glyph_id + 1);
    &glyf[start..end]
}

fn parse_tables(source: &[u8]) -> BTreeMap<[u8; 4], Vec<u8>> {
    parse_table_ranges(source)
        .into_iter()
        .map(|(tag, (offset, length))| (tag, source[offset..offset + length].to_vec()))
        .collect()
}

fn parse_table_ranges(source: &[u8]) -> BTreeMap<[u8; 4], (usize, usize)> {
    assert_eq!(&source[0..4], &[0, 1, 0, 0]);
    let count = usize::from(read_u16(source, 4));
    (0..count)
        .map(|index| {
            let record = 12 + index * 16;
            let tag = source[record..record + 4].try_into().unwrap();
            let offset = usize::try_from(read_u32(source, record + 8)).unwrap();
            let length = usize::try_from(read_u32(source, record + 12)).unwrap();
            (tag, (offset, length))
        })
        .collect()
}

fn table<'a>(
    source: &'a [u8],
    tables: &BTreeMap<[u8; 4], (usize, usize)>,
    tag: [u8; 4],
) -> &'a [u8] {
    let (offset, length) = tables[&tag];
    &source[offset..offset + length]
}

fn append_format_14(
    cmap: &[u8],
    alternate_glyph: u16,
    encoding: (u16, u16),
    sorted: bool,
) -> Vec<u8> {
    let count = usize::from(read_u16(cmap, 2));
    let old_header_length = 4 + count * 8;
    let mut records = (0..count)
        .map(|index| {
            let record = 4 + index * 8;
            (
                read_u16(cmap, record),
                read_u16(cmap, record + 2),
                read_u32(cmap, record + 4) + 8,
            )
        })
        .collect::<Vec<_>>();
    records.push((
        encoding.0,
        encoding.1,
        u32::try_from(cmap.len() + 8).unwrap(),
    ));
    records.sort_by_key(|record| (record.0, record.1));
    if !sorted {
        records.reverse();
    }

    let mut output = Vec::with_capacity(cmap.len() + 46);
    output.extend_from_slice(&cmap[0..2]);
    push_u16(&mut output, u16::try_from(count + 1).unwrap());
    for (platform_id, encoding_id, offset) in records {
        push_u16(&mut output, platform_id);
        push_u16(&mut output, encoding_id);
        push_u32(&mut output, offset);
    }
    output.extend_from_slice(&cmap[old_header_length..]);
    output.extend_from_slice(&format_14(alternate_glyph));
    output
}

fn format_14(alternate_glyph: u16) -> Vec<u8> {
    let mut table = Vec::with_capacity(38);
    push_u16(&mut table, 14);
    push_u32(&mut table, 38);
    push_u32(&mut table, 1);
    push_u24(&mut table, 0xfe0f);
    push_u32(&mut table, 21);
    push_u32(&mut table, 29);

    push_u32(&mut table, 1);
    push_u24(&mut table, u32::from('A'));
    table.push(0);

    push_u32(&mut table, 1);
    push_u24(&mut table, u32::from('B'));
    push_u16(&mut table, alternate_glyph);
    assert_eq!(table.len(), 38);
    table
}

fn rebuild_sfnt(source: &[u8], mut tables: BTreeMap<[u8; 4], Vec<u8>>) -> Vec<u8> {
    let head = tables.get_mut(b"head").expect("fixture has head");
    head[8..12].fill(0);
    let count = tables.len();
    let directory_length = 12 + count * 16;
    let mut offset = align4(directory_length);
    let records = tables
        .iter()
        .map(|(tag, data)| {
            let record = (*tag, checksum(data), offset, data.len());
            offset = align4(offset + data.len());
            record
        })
        .collect::<Vec<_>>();

    let mut output = vec![0; offset];
    output[0..12].copy_from_slice(&source[0..12]);
    let count = u16::try_from(count).unwrap();
    write_u16(&mut output, 4, count);
    let entry_selector = u16::try_from(count.ilog2()).unwrap();
    let search_range = (1_u16 << entry_selector) * 16;
    write_u16(&mut output, 6, search_range);
    write_u16(&mut output, 8, entry_selector);
    write_u16(&mut output, 10, count * 16 - search_range);
    for (index, (tag, checksum, table_offset, length)) in records.iter().enumerate() {
        let record = 12 + index * 16;
        output[record..record + 4].copy_from_slice(tag);
        write_u32(&mut output, record + 4, *checksum);
        write_u32(
            &mut output,
            record + 8,
            u32::try_from(*table_offset).unwrap(),
        );
        write_u32(&mut output, record + 12, u32::try_from(*length).unwrap());
        output[*table_offset..*table_offset + *length].copy_from_slice(&tables[tag]);
    }
    let head_offset = records
        .iter()
        .find_map(|(tag, _, offset, _)| (*tag == *b"head").then_some(*offset))
        .unwrap();
    let adjustment = 0xb1b0_afba_u32.wrapping_sub(checksum(&output));
    write_u32(&mut output, head_offset + 8, adjustment);
    output
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0_u32, |sum, chunk| {
        let mut word = [0; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn align4(value: usize) -> usize {
    (value + 3) & !3
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn push_u24(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes()[1..]);
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}
