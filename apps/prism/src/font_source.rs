use std::{
    fs::{self, File, Metadata},
    io::{Read, Take},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use ttf_parser::{Face, Permissions, name_id};

use crate::FontSlant;

pub(crate) const MAX_EMBEDDED_FONT_BYTES: usize = 32 * 1024 * 1024;

/// One bounded, authorized, immutable read of a source OpenType font.
///
/// The bytes are private so callers cannot change the identity that was
/// validated at construction time. Durable Prism projects store these exact
/// bytes through the existing content-addressed asset transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontSourceSnapshot {
    canonical_path: PathBuf,
    content_hash: String,
    bytes: Vec<u8>,
    pub(crate) family: String,
    pub(crate) style: String,
    pub(crate) weight: u16,
    pub(crate) slant: FontSlant,
    pub(crate) subset_allowed: bool,
}

impl FontSourceSnapshot {
    pub fn read(path: &Path) -> Result<Self> {
        reject_symlink(path)?;
        let canonical_path = fs::canonicalize(path)
            .with_context(|| format!("could not resolve font {}", path.display()))?;
        let path_metadata = require_regular_file(&canonical_path)?;
        let file = File::open(&canonical_path)
            .with_context(|| format!("could not read font {}", path.display()))?;
        let opened_metadata = file.metadata()?;
        if !same_file(&path_metadata, &opened_metadata) {
            bail!("font source changed while opening {}", path.display());
        }
        let bytes = read_bounded(file, &opened_metadata, path)?;
        verify_unchanged_path(path, &canonical_path, &opened_metadata)?;

        let face = Face::parse(&bytes, 0)
            .with_context(|| format!("{} is not a supported OpenType font", path.display()))?;
        require_editable_embedding(&face)?;
        let family = font_name(&face, &[name_id::TYPOGRAPHIC_FAMILY, name_id::FAMILY])
            .unwrap_or_else(|| "Imported Font".into());
        let style = font_name(&face, &[name_id::TYPOGRAPHIC_SUBFAMILY, name_id::SUBFAMILY])
            .unwrap_or_else(|| "Regular".into());
        let slant = if face.is_italic() {
            FontSlant::Italic
        } else if face.is_oblique() {
            FontSlant::Oblique
        } else {
            FontSlant::Normal
        };
        let weight = face.weight().to_number();
        let subset_allowed = face.is_subsetting_allowed();
        let content_hash = sha256_hex(&bytes);
        Ok(Self {
            canonical_path,
            content_hash,
            bytes,
            family,
            style,
            weight,
            slant,
            subset_allowed,
        })
    }

    pub(crate) fn read_verified(path: &Path, expected_hash: &str) -> Result<Self> {
        if !path.is_absolute() {
            bail!("embedded font source path is not materialized safely");
        }
        if expected_hash.len() != 64
            || !expected_hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            bail!("embedded font source has an invalid content identity");
        }
        let snapshot = Self::read(path)?;
        if snapshot.content_hash != expected_hash {
            bail!("embedded font source bytes do not match their content identity");
        }
        Ok(snapshot)
    }

    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn subset_allowed(&self) -> bool {
        self.subset_allowed
    }
}

fn read_bounded(file: File, before: &Metadata, path: &Path) -> Result<Vec<u8>> {
    if before.len() == 0 {
        bail!("font data cannot be empty");
    }
    if before.len() > MAX_EMBEDDED_FONT_BYTES as u64 {
        bail!("font exceeds Prism's 32 MiB embedded-font limit");
    }
    let expected_len = usize::try_from(before.len()).context("font length does not fit memory")?;
    let mut bytes = Vec::with_capacity(expected_len);
    let mut reader: Take<File> = file.take(MAX_EMBEDDED_FONT_BYTES as u64 + 1);
    reader.read_to_end(&mut bytes)?;
    let file = reader.into_inner();
    let after = file.metadata()?;
    if !stable_file(before, &after) || bytes.len() != expected_len {
        bail!("font source changed while reading {}", path.display());
    }
    if bytes.len() > MAX_EMBEDDED_FONT_BYTES {
        bail!("font exceeds Prism's 32 MiB embedded-font limit");
    }
    Ok(bytes)
}

fn verify_unchanged_path(path: &Path, canonical: &Path, opened: &Metadata) -> Result<()> {
    reject_symlink(path)?;
    if fs::canonicalize(path)? != canonical {
        bail!("font source path changed while reading {}", path.display());
    }
    let current = require_regular_file(canonical)?;
    if !stable_file(opened, &current) {
        bail!("font source changed while reading {}", path.display());
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("could not inspect font {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("refusing symlink font source {}", path.display());
    }
    Ok(())
}

fn require_regular_file(path: &Path) -> Result<Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("font source {} is not a regular file", path.display());
    }
    Ok(metadata)
}

fn stable_file(before: &Metadata, after: &Metadata) -> bool {
    same_file(before, after)
        && before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
}

#[cfg(unix)]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(windows)]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    left.volume_serial_number().is_some()
        && left.volume_serial_number() == right.volume_serial_number()
        && left.file_index().is_some()
        && left.file_index() == right.file_index()
}

#[cfg(not(any(unix, windows)))]
fn same_file(_left: &Metadata, _right: &Metadata) -> bool {
    false
}

fn require_editable_embedding(face: &Face<'_>) -> Result<()> {
    if !matches!(
        face.permissions(),
        Some(Permissions::Installable | Permissions::Editable)
    ) {
        bail!(
            "OpenType embedding metadata does not allow portable editable embedding (preview/print-only fonts are unsupported); verify the font license separately"
        );
    }
    if !face.is_outline_embedding_allowed() {
        bail!(
            "OpenType embedding metadata does not allow editable outline embedding; verify the font license separately"
        );
    }
    Ok(())
}

fn font_name(face: &Face<'_>, ids: &[u16]) -> Option<String> {
    ids.iter().find_map(|wanted| {
        face.names()
            .into_iter()
            .filter(|name| name.name_id == *wanted)
            .find_map(|name| name.to_string())
            .filter(|name| !name.trim().is_empty())
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn file_identity_rejects_same_length_path_replacement() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("prism-font-identity-{stamp}"));
        fs::create_dir_all(&directory).unwrap();
        let first = directory.join("first.ttf");
        let replacement = directory.join("replacement.ttf");
        fs::write(&first, b"same length").unwrap();
        fs::write(&replacement, b"same length").unwrap();
        let opened = File::open(&first).unwrap().metadata().unwrap();
        fs::remove_file(&first).unwrap();
        fs::rename(&replacement, &first).unwrap();
        let current = fs::metadata(&first).unwrap();
        assert!(!same_file(&opened, &current));
        fs::remove_dir_all(directory).unwrap();
    }
}
