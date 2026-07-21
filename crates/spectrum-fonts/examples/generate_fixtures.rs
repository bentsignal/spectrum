use std::{env, error::Error, fs, path::Path};

use hb_subset::{Blob, Flags, FontFace, SubsetInput, Tag};
use sha2::{Digest, Sha256};

const INPUT_SHA256: &str = "b85c38ecea8a7cfb39c24e395a4007474fa5a4fc864f6ee33309eb4948d232d5";
const STATIC_SHA256: &str = "1de794fb16bb4fc99afef5d597b909791230be4ea216c5762797f09d9d56a04c";
const LAYOUT_SHA256: &str = "05549e889a11eb65b542a071e97d71f4333caf5a88408ab10a1ac3de25d4be3a";
const STATIC_TABLES: [[u8; 4]; 10] = [
    *b"head", *b"hhea", *b"maxp", *b"OS/2", *b"hmtx", *b"cmap", *b"loca", *b"glyf", *b"name",
    *b"post",
];
const LAYOUT_TABLES: [[u8; 4]; 13] = [
    *b"head", *b"hhea", *b"maxp", *b"OS/2", *b"hmtx", *b"cmap", *b"loca", *b"glyf", *b"name",
    *b"post", *b"GDEF", *b"GPOS", *b"GSUB",
];

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os().skip(1);
    let source_path = arguments.next().ok_or("missing source-font path")?;
    let output_directory = arguments.next().ok_or("missing output-directory path")?;
    if arguments.next().is_some() {
        return Err("expected exactly two arguments".into());
    }

    let source = fs::read(&source_path)?;
    require_hash("upstream Noto Sans source", &source, INPUT_SHA256)?;
    fs::create_dir_all(&output_directory)?;
    let output_directory = Path::new(&output_directory);
    fs::write(
        output_directory.join("noto-sans-rich-rejected.ttf"),
        &source,
    )?;

    let static_subset = subset(&source, 'A'..='Z', &STATIC_TABLES)?;
    let layout_characters = ('A'..='Z')
        .chain('a'..='z')
        .chain(['\u{c5}'])
        .collect::<Vec<_>>();
    let layout_subset = subset(&source, layout_characters, &LAYOUT_TABLES)?;
    let mut pending = Vec::new();
    if let Err(error) = require_hash(
        "generated unhinted static fixture",
        &static_subset,
        STATIC_SHA256,
    ) {
        pending.push(error.to_string());
    }
    if let Err(error) = require_hash(
        "generated unhinted layout fixture",
        &layout_subset,
        LAYOUT_SHA256,
    ) {
        pending.push(error.to_string());
    }
    if !pending.is_empty() {
        return Err(pending.join("\n").into());
    }
    fs::write(
        output_directory.join("noto-sans-static-source.ttf"),
        static_subset,
    )?;
    fs::write(
        output_directory.join("noto-sans-layout-source.ttf"),
        layout_subset,
    )?;
    Ok(())
}

fn subset(
    source: &[u8],
    characters: impl IntoIterator<Item = char>,
    allowed_tables: &[[u8; 4]],
) -> Result<Vec<u8>, Box<dyn Error>> {
    let blob = Blob::from_bytes(source)?;
    let face = FontFace::new(blob)?;
    let mut input = SubsetInput::new()?;
    input.glyph_set().clear();
    input.unicode_set().clear();
    input.name_id_set().clear();
    input.name_id_set().insert_range(0..=u32::from(u16::MAX));
    input.name_lang_id_set().clear();
    input
        .name_lang_id_set()
        .insert_range(0..=u32::from(u16::MAX));
    *input.flags() = *Flags::default()
        .remove_hinting()
        .remap_glyph_indices()
        .retain_subroutines()
        .retain_legacy_names()
        .retain_notdef_outline()
        .retain_glyph_names()
        .retain_unicode_ranges()
        .retain_layout_closure();
    for character in characters {
        input.unicode_set().insert(character);
    }
    for tag in table_tags(source)? {
        if !allowed_tables.contains(&tag) {
            input.drop_table_tag_set().insert(Tag::new(tag));
        }
    }

    Ok(input.subset_font(&face)?.underlying_blob().to_vec())
}

fn require_hash(label: &str, bytes: &[u8], expected: &str) -> Result<(), Box<dyn Error>> {
    let actual = sha256_hex(bytes);
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{label} SHA-256 is pending; approved build produced {actual}").into());
    }
    if actual != expected {
        return Err(format!("{label} SHA-256 mismatch: expected {expected}, got {actual}").into());
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn table_tags(bytes: &[u8]) -> Result<Vec<[u8; 4]>, Box<dyn Error>> {
    if bytes.get(0..4) != Some(&[0, 1, 0, 0]) {
        return Err("fixture generator accepts one standalone TrueType face".into());
    }
    let count = usize::from(read_u16(bytes, 4)?);
    let mut tags = Vec::with_capacity(count);
    for index in 0..count {
        let offset = 12_usize
            .checked_add(index.checked_mul(16).ok_or("table index overflow")?)
            .ok_or("table-directory offset overflow")?;
        tags.push(
            bytes
                .get(offset..offset + 4)
                .ok_or("truncated table directory")?
                .try_into()?,
        );
    }
    Ok(tags)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, Box<dyn Error>> {
    Ok(u16::from_be_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or("truncated font data")?
            .try_into()?,
    ))
}
