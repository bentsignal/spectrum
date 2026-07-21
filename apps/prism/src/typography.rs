use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{FontSourceSnapshot, VerifiedFontSource};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontSlant {
    #[default]
    Normal,
    Italic,
    Oblique,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FontAsset {
    pub id: u64,
    pub family: String,
    pub style: String,
    pub weight: u16,
    pub slant: FontSlant,
    pub source_name: String,
    /// Whether OpenType OS/2 embedding metadata allows technical subsetting.
    /// This is not a legal license conclusion.
    pub subset_allowed: bool,
    pub content_hash: String,
    pub path: PathBuf,
    #[serde(default)]
    pub original_path: Option<PathBuf>,
}

impl FontAsset {
    pub fn import(id: u64, path: &Path) -> Result<Self> {
        let snapshot = FontSourceSnapshot::read(path)?;
        Ok(Self {
            id,
            family: snapshot.family.clone(),
            style: snapshot.style.clone(),
            weight: snapshot.weight,
            slant: snapshot.slant,
            source_name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("font")
                .to_owned(),
            subset_allowed: snapshot.subset_allowed(),
            content_hash: snapshot.content_hash().to_owned(),
            path: snapshot.canonical_path().to_owned(),
            original_path: Some(snapshot.canonical_path().to_owned()),
        })
    }

    pub fn source_snapshot(&self) -> Result<FontSourceSnapshot> {
        let snapshot = FontSourceSnapshot::read_verified(&self.path, &self.content_hash)?;
        if snapshot.family != self.family
            || snapshot.style != self.style
            || snapshot.weight != self.weight
            || snapshot.slant != self.slant
            || snapshot.subset_allowed != self.subset_allowed
        {
            bail!("embedded font metadata does not match its immutable source snapshot");
        }
        Ok(snapshot)
    }

    pub fn bytes(&self) -> Result<Vec<u8>> {
        Ok(self.source_snapshot()?.bytes().to_vec())
    }

    pub(crate) fn from_embedded_bytes(
        id: u64,
        source_name: String,
        path: PathBuf,
        bytes: Vec<u8>,
        expected_hash: &str,
    ) -> Result<Self> {
        let verified = VerifiedFontSource::from_embedded_bytes(bytes, expected_hash)?;
        Ok(Self {
            id,
            family: verified.family.clone(),
            style: verified.style.clone(),
            weight: verified.weight,
            slant: verified.slant,
            source_name,
            subset_allowed: verified.subset_allowed(),
            content_hash: verified.content_hash().to_owned(),
            path,
            original_path: None,
        })
    }

    pub(crate) fn verify_embedded_bytes(&self, bytes: Vec<u8>) -> Result<VerifiedFontSource> {
        let verified = VerifiedFontSource::from_embedded_bytes(bytes, &self.content_hash)?;
        if verified.family != self.family
            || verified.style != self.style
            || verified.weight != self.weight
            || verified.slant != self.slant
            || verified.subset_allowed != self.subset_allowed
        {
            bail!("embedded font metadata does not match its immutable source snapshot");
        }
        Ok(verified)
    }
}

pub(crate) fn make_fonts_portable(
    fonts: &mut [FontAsset],
    project_directory: &Path,
    asset_directory: &Path,
) -> Result<()> {
    for font in fonts {
        let snapshot = font.source_snapshot()?;
        let canonical = snapshot.canonical_path();
        if font.original_path.is_none() {
            font.original_path = Some(canonical.to_owned());
        }
        let font_directory = asset_directory.join("fonts");
        fs::create_dir_all(&font_directory)?;
        let extension = canonical
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| {
                value.len() <= 8 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
            .unwrap_or("otf");
        let destination = font_directory.join(format!(
            "font-{}-{}.{}",
            font.id,
            snapshot.content_hash(),
            extension
        ));
        if canonical != destination {
            persist_font_snapshot(&destination, &snapshot)?;
        }
        font.path = destination.strip_prefix(project_directory)?.to_owned();
    }
    Ok(())
}

fn persist_font_snapshot(destination: &Path, snapshot: &FontSourceSnapshot) -> Result<()> {
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
    {
        Ok(mut file) => {
            if let Err(error) = file
                .write_all(snapshot.bytes())
                .and_then(|()| file.sync_all())
            {
                drop(file);
                let _ = fs::remove_file(destination);
                return Err(error).with_context(|| {
                    format!("could not persist font snapshot {}", destination.display())
                });
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            FontSourceSnapshot::read_verified(destination, snapshot.content_hash())?;
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!("could not create font snapshot {}", destination.display())
            });
        }
    }
    Ok(())
}

pub(crate) fn resolve_portable_fonts(fonts: &mut [FontAsset], project_directory: &Path) {
    for font in fonts {
        if font.path.is_relative() {
            font.path = project_directory.join(&font.path);
            if let Ok(canonical) = fs::canonicalize(&font.path) {
                font.path = canonical;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlignment {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextEffects {
    pub outline_width: f32,
    pub outline_color: [u8; 4],
    pub shadow_offset_x: f32,
    pub shadow_offset_y: f32,
    pub shadow_color: [u8; 4],
}

impl Default for TextEffects {
    fn default() -> Self {
        Self {
            outline_width: 0.0,
            outline_color: [0, 0, 0, 255],
            shadow_offset_x: 0.0,
            shadow_offset_y: 0.0,
            shadow_color: [0, 0, 0, 0],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextTypography {
    pub font_id: Option<u64>,
    pub alignment: TextAlignment,
    /// Line-spacing multiplier. `1.25` exactly preserves Prism's legacy spacing.
    pub line_height: f32,
    /// Additional advance between glyphs in document pixels.
    pub tracking: f32,
    /// Optional paragraph width used for wrapping and alignment.
    pub box_width: Option<f32>,
    pub effects: TextEffects,
}

impl Default for TextTypography {
    fn default() -> Self {
        Self {
            font_id: None,
            alignment: TextAlignment::Left,
            line_height: 1.25,
            tracking: 0.0,
            box_width: None,
            effects: TextEffects::default(),
        }
    }
}

impl TextTypography {
    pub(crate) fn sanitized(mut self) -> Self {
        self.line_height = self.line_height.clamp(0.5, 4.0);
        self.tracking = self.tracking.clamp(-100.0, 500.0);
        self.box_width = self.box_width.map(|width| width.clamp(1.0, 100_000.0));
        self.effects.outline_width = self.effects.outline_width.clamp(0.0, 128.0);
        self.effects.shadow_offset_x = self.effects.shadow_offset_x.clamp(-2_048.0, 2_048.0);
        self.effects.shadow_offset_y = self.effects.shadow_offset_y.clamp(-2_048.0, 2_048.0);
        self
    }

    /// Scales document-pixel typography values for a higher-resolution raster.
    #[doc(hidden)]
    pub fn scale_for_raster(&mut self, scale: f32) {
        self.tracking *= scale;
        self.box_width = self.box_width.map(|width| width * scale);
        self.effects.outline_width *= scale;
        self.effects.shadow_offset_x *= scale;
        self.effects.shadow_offset_y *= scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_editable_embedding_requires_installable_or_editable_permission() {
        assert!(permissions_allow_editable_embedding(Some(
            Permissions::Installable
        )));
        assert!(permissions_allow_editable_embedding(Some(
            Permissions::Editable
        )));
        assert!(!permissions_allow_editable_embedding(Some(
            Permissions::PreviewAndPrint
        )));
        assert!(!permissions_allow_editable_embedding(Some(
            Permissions::Restricted
        )));
        assert!(!permissions_allow_editable_embedding(None));
    }

    #[test]
    fn content_hash_is_lowercase_sha256_hex() {
        let hash = sha256_hex(b"Spectrum typography");
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(hash, hash.to_ascii_lowercase());
        assert_eq!(
            hash,
            "08bf2a874548a4cad5f52023922a20f1b4b8724372b71a296673e9b6f1ce6696"
        );
    }
}
