use std::collections::{BTreeMap, BTreeSet};

use fontdue::{Font, FontSettings};
use ttf_parser::{Face, GlyphId, OutlineBuilder, Permissions};

use crate::{
    SubsetError, SubsetRequest, UnicodeVariationSequence,
    cmap::{UvsKind, UvsMapping, variation_mapping},
    sfnt::SfntFace,
};

// This allowlist is intentionally smaller than HarfBuzz's capability. Each richer
// table class is added only after its licensed corpus fixture passes on all targets.
const CURRENT_CANDIDATE_TABLES: [[u8; 4]; 13] = [
    *b"head", *b"hhea", *b"maxp", *b"OS/2", *b"hmtx", *b"cmap", *b"loca", *b"glyf", *b"name",
    *b"post", *b"GDEF", *b"GPOS", *b"GSUB",
];

const REQUIRED_TABLES: [[u8; 4]; 9] = [
    *b"head", *b"hhea", *b"maxp", *b"OS/2", *b"hmtx", *b"cmap", *b"loca", *b"glyf", *b"name",
];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RawNameRecord {
    platform_id: u16,
    encoding_id: u16,
    language_id: u16,
    name_id: u16,
    payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NameProfile {
    format: u16,
    records: Vec<RawNameRecord>,
    language_tags: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GlyphFingerprint {
    advance: Option<u16>,
    side_bearing: Option<i16>,
    bounds: Option<(i16, i16, i16, i16)>,
    outline: Vec<OutlineCommand>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum OutlineCommand {
    Move(u32, u32),
    Line(u32, u32),
    Quad(u32, u32, u32, u32),
    Curve(u32, u32, u32, u32, u32, u32),
    Close,
}

#[derive(Default)]
struct FingerprintBuilder {
    commands: Vec<OutlineCommand>,
}

impl OutlineBuilder for FingerprintBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.commands
            .push(OutlineCommand::Move(x.to_bits(), y.to_bits()));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.commands
            .push(OutlineCommand::Line(x.to_bits(), y.to_bits()));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.commands.push(OutlineCommand::Quad(
            x1.to_bits(),
            y1.to_bits(),
            x.to_bits(),
            y.to_bits(),
        ));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.commands.push(OutlineCommand::Curve(
            x1.to_bits(),
            y1.to_bits(),
            x2.to_bits(),
            y2.to_bits(),
            x.to_bits(),
            y.to_bits(),
        ));
    }

    fn close(&mut self) {
        self.commands.push(OutlineCommand::Close);
    }
}

pub(crate) struct SourceProfile {
    tags: BTreeSet<[u8; 4]>,
    fs_type: [u8; 2],
    names: NameProfile,
    name_ids: BTreeSet<u32>,
    name_languages: BTreeSet<u32>,
    variation_mappings: BTreeMap<UnicodeVariationSequence, UvsMapping>,
}

impl SourceProfile {
    pub(crate) fn name_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.name_ids.iter().copied()
    }

    pub(crate) fn name_languages(&self) -> impl Iterator<Item = u32> + '_ {
        self.name_languages.iter().copied()
    }
}

pub(crate) fn validate_source(
    bytes: &[u8],
    request: &SubsetRequest,
) -> Result<SourceProfile, SubsetError> {
    if bytes.is_empty() {
        return Err(SubsetError::new("font data cannot be empty"));
    }
    crate::limits::validate_source_size(bytes)?;
    crate::limits::validate_request(request)?;
    if request.subset_codepoints().is_empty() {
        return Err(SubsetError::new("font subset repertoire cannot be empty"));
    }

    let sfnt = SfntFace::parse(bytes, request.face_index())?;
    validate_table_class(&sfnt)?;
    if [*b"GDEF", *b"GPOS", *b"GSUB"]
        .iter()
        .any(|tag| sfnt.has(*tag))
        && request.shaping_samples().is_empty()
    {
        return Err(SubsetError::new(
            "fonts with OpenType layout tables require shaping samples for closure validation",
        ));
    }
    let face = Face::parse(bytes, request.face_index())
        .map_err(|_| SubsetError::new("font is not valid OpenType data"))?;
    if usize::from(face.number_of_glyphs()) > crate::limits::MAX_GLYPHS {
        return Err(SubsetError::new("font glyph count exceeds resource limit"));
    }
    crate::glyf::validate_unhinted(&sfnt, face.number_of_glyphs())?;
    validate_embedding_metadata(&face)?;
    validate_codepoints(&face, request)?;

    let names = parse_name_profile(&sfnt)?;
    let name_ids = names
        .records
        .iter()
        .map(|record| u32::from(record.name_id))
        .collect();
    let name_languages = names
        .records
        .iter()
        .map(|record| u32::from(record.language_id))
        .collect();
    let variation_mappings = request
        .variation_sequences()
        .iter()
        .copied()
        .map(|sequence| {
            variation_mapping(&sfnt, &face, sequence).map(|mapping| (sequence, mapping))
        })
        .collect::<Result<_, _>>()?;
    Ok(SourceProfile {
        tags: sfnt.tags().collect(),
        fs_type: fs_type(&sfnt)?,
        names,
        name_ids,
        name_languages,
        variation_mappings,
    })
}

pub(crate) fn validate_output(
    source: &[u8],
    output: &[u8],
    request: &SubsetRequest,
    source_profile: &SourceProfile,
    glyph_mapping: &BTreeMap<u16, u16>,
) -> Result<(), SubsetError> {
    if output.len() >= source.len() {
        return Err(SubsetError::new(format!(
            "subset candidate did not reduce bytes (source {}, output {})",
            source.len(),
            output.len()
        )));
    }

    let sfnt = SfntFace::parse(output, 0)?;
    validate_table_class(&sfnt)?;
    let output_tags = sfnt.tags().collect::<BTreeSet<_>>();
    if output_tags != source_profile.tags {
        return Err(SubsetError::new(
            "subset candidate changed the current fail-closed SFNT table inventory",
        ));
    }
    let face = Face::parse(output, 0)
        .map_err(|_| SubsetError::new("subset candidate cannot be parsed as OpenType data"))?;
    crate::glyf::validate_unhinted(&sfnt, face.number_of_glyphs())?;
    validate_codepoints(&face, request)?;
    if fs_type(&sfnt)? != source_profile.fs_type {
        return Err(SubsetError::new(
            "subset candidate changed the OpenType OS/2 embedding metadata",
        ));
    }
    if parse_name_profile(&sfnt)? != source_profile.names {
        return Err(SubsetError::new(
            "subset candidate changed protected raw name records",
        ));
    }
    let output_variations = request
        .variation_sequences()
        .iter()
        .copied()
        .map(|sequence| {
            variation_mapping(&sfnt, &face, sequence).map(|mapping| (sequence, mapping))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    for (sequence, source_mapping) in &source_profile.variation_mappings {
        let output_mapping = output_variations
            .get(sequence)
            .ok_or_else(|| SubsetError::new("subset candidate dropped a requested UVS mapping"))?;
        if output_mapping.kind != source_mapping.kind {
            return Err(SubsetError::new(format!(
                "subset candidate changed U+{:04X} U+{:04X} from {:?} to {:?} mapping",
                sequence.base_codepoint,
                sequence.selector_codepoint,
                source_mapping.kind,
                output_mapping.kind
            )));
        }
    }
    validate_resolved_glyph_parity(
        source,
        output,
        &source_profile.variation_mappings,
        &output_variations,
        glyph_mapping,
    )?;
    validate_nominal_glyph_parity(source, output, request, glyph_mapping)?;
    let shaped_glyphs = crate::shaping::validate_parity(source, output, request, glyph_mapping)?;
    let nondefault_uvs_pairs = source_profile
        .variation_mappings
        .iter()
        .filter_map(|(sequence, source_mapping)| {
            if source_mapping.kind != UvsKind::NonDefault {
                return None;
            }
            output_variations
                .get(sequence)
                .map(|output_mapping| (source_mapping.glyph_id, output_mapping.glyph_id))
        })
        .collect::<BTreeSet<_>>();
    validate_shaped_glyph_parity(source, output, &shaped_glyphs, &nondefault_uvs_pairs)?;
    validate_raster_parity(source, output, request)?;
    Ok(())
}

fn validate_table_class(sfnt: &SfntFace<'_>) -> Result<(), SubsetError> {
    for tag in sfnt.tags() {
        if !CURRENT_CANDIDATE_TABLES.contains(&tag) {
            return Err(SubsetError::new(format!(
                "{} tables are not enabled before cross-platform corpus validation",
                crate::sfnt::display_tag(tag)
            )));
        }
    }
    for tag in REQUIRED_TABLES {
        if !sfnt.has(tag) {
            return Err(SubsetError::new(format!(
                "standalone TrueType candidate is missing required {} table",
                crate::sfnt::display_tag(tag)
            )));
        }
    }
    Ok(())
}

fn validate_embedding_metadata(face: &Face<'_>) -> Result<(), SubsetError> {
    if !matches!(
        face.permissions(),
        Some(Permissions::Installable | Permissions::Editable)
    ) {
        return Err(SubsetError::new(
            "OpenType embedding metadata does not allow portable editable embedding",
        ));
    }
    if !face.is_outline_embedding_allowed() {
        return Err(SubsetError::new(
            "OpenType embedding metadata does not allow outline embedding",
        ));
    }
    if !face.is_subsetting_allowed() {
        return Err(SubsetError::new(
            "OpenType embedding metadata forbids technical subsetting",
        ));
    }
    Ok(())
}

fn validate_codepoints(face: &Face<'_>, request: &SubsetRequest) -> Result<(), SubsetError> {
    for codepoint in request.codepoints() {
        let character = char::from_u32(*codepoint).ok_or_else(|| {
            SubsetError::new(format!("U+{codepoint:04X} is not a Unicode scalar value"))
        })?;
        if face.glyph_index(character).is_none() {
            return Err(SubsetError::new(format!(
                "font does not map requested U+{codepoint:04X}"
            )));
        }
    }
    Ok(())
}

fn parse_name_profile(sfnt: &SfntFace<'_>) -> Result<NameProfile, SubsetError> {
    let table = sfnt
        .table(*b"name")
        .ok_or_else(|| SubsetError::new("font has no name table"))?;
    let format = read_u16(table, 0)?;
    if table.len() > crate::limits::MAX_NAME_COPY_BYTES {
        return Err(SubsetError::new("name table exceeds resource limit"));
    }
    if format > 1 {
        return Err(SubsetError::new(format!(
            "name table format {format} is not validated"
        )));
    }
    let count = usize::from(read_u16(table, 2)?);
    if count > crate::limits::MAX_NAME_RECORDS {
        return Err(SubsetError::new("name-record count exceeds resource limit"));
    }
    let storage_offset = usize::from(read_u16(table, 4)?);
    checked_slice(
        table,
        6,
        count
            .checked_mul(12)
            .ok_or_else(|| SubsetError::new("name-record count overflow"))?,
    )?;
    let mut records = Vec::with_capacity(count);
    let mut copied_bytes = 0_usize;
    for index in 0..count {
        let record = 6 + index * 12;
        let length = usize::from(read_u16(table, record + 8)?);
        let string_offset = usize::from(read_u16(table, record + 10)?);
        let payload_offset = storage_offset
            .checked_add(string_offset)
            .ok_or_else(|| SubsetError::new("name string offset overflow"))?;
        copied_bytes = copied_bytes
            .checked_add(length)
            .ok_or_else(|| SubsetError::new("name payload byte count overflow"))?;
        if copied_bytes > crate::limits::MAX_NAME_COPY_BYTES {
            return Err(SubsetError::new(
                "name payload copies exceed resource limit",
            ));
        }
        records.push(RawNameRecord {
            platform_id: read_u16(table, record)?,
            encoding_id: read_u16(table, record + 2)?,
            language_id: read_u16(table, record + 4)?,
            name_id: read_u16(table, record + 6)?,
            payload: checked_slice(table, payload_offset, length)?.to_vec(),
        });
    }
    records.sort();

    let mut language_tags = Vec::new();
    if format == 1 {
        let language_count_offset = 6 + count * 12;
        let language_count = usize::from(read_u16(table, language_count_offset)?);
        if language_count > crate::limits::MAX_NAME_RECORDS {
            return Err(SubsetError::new(
                "name language-tag count exceeds resource limit",
            ));
        }
        checked_slice(
            table,
            language_count_offset + 2,
            language_count
                .checked_mul(4)
                .ok_or_else(|| SubsetError::new("name language-tag count overflow"))?,
        )?;
        for index in 0..language_count {
            let record = language_count_offset + 2 + index * 4;
            let length = usize::from(read_u16(table, record)?);
            let string_offset = usize::from(read_u16(table, record + 2)?);
            let payload_offset = storage_offset
                .checked_add(string_offset)
                .ok_or_else(|| SubsetError::new("name language-tag offset overflow"))?;
            copied_bytes = copied_bytes
                .checked_add(length)
                .ok_or_else(|| SubsetError::new("name payload byte count overflow"))?;
            if copied_bytes > crate::limits::MAX_NAME_COPY_BYTES {
                return Err(SubsetError::new(
                    "name payload copies exceed resource limit",
                ));
            }
            language_tags.push(checked_slice(table, payload_offset, length)?.to_vec());
        }
    }
    Ok(NameProfile {
        format,
        records,
        language_tags,
    })
}

fn validate_resolved_glyph_parity(
    source: &[u8],
    output: &[u8],
    source_variations: &BTreeMap<UnicodeVariationSequence, UvsMapping>,
    output_variations: &BTreeMap<UnicodeVariationSequence, UvsMapping>,
    glyph_mapping: &BTreeMap<u16, u16>,
) -> Result<(), SubsetError> {
    let source_face = Face::parse(source, 0)
        .map_err(|_| SubsetError::new("source cannot be parsed for glyph parity"))?;
    let output_face = Face::parse(output, 0)
        .map_err(|_| SubsetError::new("subset candidate cannot be parsed for glyph parity"))?;
    for (sequence, source_mapping) in source_variations {
        let output_mapping = output_variations
            .get(sequence)
            .ok_or_else(|| SubsetError::new("subset candidate dropped a UVS glyph mapping"))?;
        let mapped_glyph = glyph_mapping
            .get(&source_mapping.glyph_id)
            .ok_or_else(|| SubsetError::new("subset plan omitted a requested UVS source glyph"))?;
        if *mapped_glyph != output_mapping.glyph_id {
            return Err(SubsetError::new(format!(
                "subset cmap maps U+{:04X} U+{:04X} to glyph {}, but the subset plan maps source glyph {} to {}",
                sequence.base_codepoint,
                sequence.selector_codepoint,
                output_mapping.glyph_id,
                source_mapping.glyph_id,
                mapped_glyph
            )));
        }
        let source_fingerprint = glyph_fingerprint(&source_face, source_mapping.glyph_id)?;
        let output_fingerprint = glyph_fingerprint(&output_face, output_mapping.glyph_id)?;
        if source_fingerprint != output_fingerprint {
            return Err(SubsetError::new(format!(
                "subset candidate changed the resolved outline or horizontal metrics for U+{:04X} U+{:04X}",
                sequence.base_codepoint, sequence.selector_codepoint
            )));
        }
    }
    Ok(())
}

fn validate_nominal_glyph_parity(
    source: &[u8],
    output: &[u8],
    request: &SubsetRequest,
    glyph_mapping: &BTreeMap<u16, u16>,
) -> Result<(), SubsetError> {
    let source_face = Face::parse(source, 0)
        .map_err(|_| SubsetError::new("source cannot be parsed for nominal glyph parity"))?;
    let output_face = Face::parse(output, 0)
        .map_err(|_| SubsetError::new("subset cannot be parsed for nominal glyph parity"))?;
    for codepoint in request.codepoints() {
        let character = char::from_u32(*codepoint)
            .ok_or_else(|| SubsetError::new("invalid scalar reached nominal glyph parity"))?;
        let source_glyph = source_face
            .glyph_index(character)
            .ok_or_else(|| SubsetError::new("source lost a validated nominal glyph"))?;
        let output_glyph = output_face
            .glyph_index(character)
            .ok_or_else(|| SubsetError::new("subset lost a validated nominal glyph"))?;
        let mapped_glyph = glyph_mapping.get(&source_glyph.0).ok_or_else(|| {
            SubsetError::new(format!(
                "subset plan omitted nominal source glyph {} for U+{codepoint:04X}",
                source_glyph.0
            ))
        })?;
        if *mapped_glyph != output_glyph.0 {
            return Err(SubsetError::new(format!(
                "subset cmap maps U+{codepoint:04X} to glyph {}, but the subset plan maps source glyph {} to {}",
                output_glyph.0, source_glyph.0, mapped_glyph
            )));
        }
        if glyph_fingerprint(&source_face, source_glyph.0)?
            != glyph_fingerprint(&output_face, output_glyph.0)?
        {
            return Err(SubsetError::new(format!(
                "subset candidate changed nominal outline or horizontal metrics for U+{codepoint:04X}"
            )));
        }
    }
    Ok(())
}

fn validate_shaped_glyph_parity(
    source: &[u8],
    output: &[u8],
    shaped_glyphs: &BTreeSet<(u16, u16)>,
    nondefault_uvs_pairs: &BTreeSet<(u16, u16)>,
) -> Result<(), SubsetError> {
    let source_face = Face::parse(source, 0)
        .map_err(|_| SubsetError::new("source cannot be parsed for shaped glyph parity"))?;
    let output_face = Face::parse(output, 0)
        .map_err(|_| SubsetError::new("subset cannot be parsed for shaped glyph parity"))?;
    let source_font = Font::from_bytes(source, FontSettings::default()).map_err(|error| {
        SubsetError::new(format!(
            "source cannot be loaded for shaped glyph raster parity: {error}"
        ))
    })?;
    let output_font = Font::from_bytes(output, FontSettings::default()).map_err(|error| {
        SubsetError::new(format!(
            "subset cannot be loaded for shaped glyph raster parity: {error}"
        ))
    })?;

    for (source_glyph, output_glyph) in shaped_glyphs {
        if glyph_fingerprint(&source_face, *source_glyph)?
            != glyph_fingerprint(&output_face, *output_glyph)?
        {
            return Err(SubsetError::new(format!(
                "subset candidate changed shaped closure glyph {source_glyph} outline or horizontal metrics"
            )));
        }
        let source_raster = source_font.rasterize_indexed(*source_glyph, 64.0);
        let output_raster = output_font.rasterize_indexed(*output_glyph, 64.0);
        // fontdue 0.9.3 materializes indexed glyphs found through nominal cmap
        // records and GSUB, but not alternates reachable solely through cmap format 14.
        // HarfBuzz mapping plus exact ttf-parser outline/metric parity above remains
        // mandatory for those non-default UVS pairs. All other shaped glyphs retain
        // indexed fontdue raster parity.
        if nondefault_uvs_pairs.contains(&(*source_glyph, *output_glyph)) {
            continue;
        }
        if source_raster != output_raster {
            let differing_pixels = source_raster
                .1
                .iter()
                .zip(&output_raster.1)
                .filter(|(source, output)| source != output)
                .count()
                + source_raster.1.len().abs_diff(output_raster.1.len());
            return Err(SubsetError::new(format!(
                "subset candidate changed shaped closure glyph {source_glyph} in the app's unhinted renderer: source metrics {:?}, output metrics {:?}, {} differing pixels",
                source_raster.0, output_raster.0, differing_pixels
            )));
        }
    }
    Ok(())
}

fn glyph_fingerprint(face: &Face<'_>, glyph_id: u16) -> Result<GlyphFingerprint, SubsetError> {
    if glyph_id >= face.number_of_glyphs() {
        return Err(SubsetError::new(format!(
            "glyph {glyph_id} is outside the font glyph range"
        )));
    }
    let glyph_id = GlyphId(glyph_id);
    let mut builder = FingerprintBuilder::default();
    let bounds = face
        .outline_glyph(glyph_id, &mut builder)
        .map(|bounds| (bounds.x_min, bounds.y_min, bounds.x_max, bounds.y_max));
    Ok(GlyphFingerprint {
        advance: face.glyph_hor_advance(glyph_id),
        side_bearing: face.glyph_hor_side_bearing(glyph_id),
        bounds,
        outline: builder.commands,
    })
}

fn fs_type(sfnt: &SfntFace<'_>) -> Result<[u8; 2], SubsetError> {
    let os2 = sfnt
        .table(*b"OS/2")
        .ok_or_else(|| SubsetError::new("font has no OS/2 embedding metadata"))?;
    os2.get(8..10)
        .ok_or_else(|| SubsetError::new("font has truncated OS/2 embedding metadata"))?
        .try_into()
        .map_err(|_| SubsetError::new("font has truncated OS/2 embedding metadata"))
}

fn validate_raster_parity(
    source: &[u8],
    output: &[u8],
    request: &SubsetRequest,
) -> Result<(), SubsetError> {
    let source_font = Font::from_bytes(source, FontSettings::default()).map_err(|error| {
        SubsetError::new(format!("source cannot be loaded by renderer: {error}"))
    })?;
    let output_font = Font::from_bytes(output, FontSettings::default()).map_err(|error| {
        SubsetError::new(format!(
            "subset candidate cannot be loaded by renderer: {error}"
        ))
    })?;
    for codepoint in request.codepoints() {
        let character = char::from_u32(*codepoint)
            .ok_or_else(|| SubsetError::new("invalid Unicode scalar reached raster validation"))?;
        if source_font.rasterize(character, 64.0) != output_font.rasterize(character, 64.0) {
            return Err(SubsetError::new(format!(
                "subset candidate renderer parity failed for U+{codepoint:04X}"
            )));
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_name_records_preserve_platform_encoding_and_language() {
        let left = RawNameRecord {
            platform_id: 3,
            encoding_id: 1,
            language_id: 0x0409,
            name_id: 1,
            payload: vec![0, b'A'],
        };
        let mut right = left.clone();
        right.language_id = 0x0411;
        assert_ne!(left, right);
    }
}
