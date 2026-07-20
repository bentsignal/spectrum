use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ttf_parser::{Face, Permissions, name_id};

const MAX_EMBEDDED_FONT_BYTES: usize = 32 * 1024 * 1024;

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
    pub subset_allowed: bool,
    pub content_hash: String,
    pub path: PathBuf,
    #[serde(default)]
    pub original_path: Option<PathBuf>,
}

impl FontAsset {
    pub fn import(id: u64, path: &Path) -> Result<Self> {
        let bytes =
            fs::read(path).with_context(|| format!("could not read font {}", path.display()))?;
        if bytes.is_empty() {
            bail!("font data cannot be empty");
        }
        if bytes.len() > MAX_EMBEDDED_FONT_BYTES {
            bail!("font exceeds Prism's 32 MiB embedded-font limit");
        }
        let face = Face::parse(&bytes, 0)
            .with_context(|| format!("{} is not a supported OpenType font", path.display()))?;
        if !permissions_allow_editable_embedding(face.permissions()) {
            bail!(
                "font license does not permit portable editable embedding (preview/print-only fonts are unsupported)"
            );
        }
        if !face.is_outline_embedding_allowed() {
            bail!("font license does not permit editable outline embedding");
        }
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
        let canonical = fs::canonicalize(path)
            .with_context(|| format!("could not resolve font {}", path.display()))?;
        Ok(Self {
            id,
            family,
            style,
            weight: face.weight().to_number(),
            slant,
            source_name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("font")
                .to_owned(),
            subset_allowed: face.is_subsetting_allowed(),
            content_hash: sha256_hex(&bytes),
            path: canonical.clone(),
            original_path: Some(canonical),
        })
    }

    pub fn bytes(&self) -> Result<Vec<u8>> {
        fs::read(&self.path)
            .with_context(|| format!("could not read embedded font {}", self.path.display()))
    }
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

pub(crate) fn make_fonts_portable(
    fonts: &mut [FontAsset],
    project_directory: &Path,
    asset_directory: &Path,
) -> Result<()> {
    for font in fonts {
        let canonical = fs::canonicalize(&font.path)
            .with_context(|| format!("could not read font asset {}", font.path.display()))?;
        if font.original_path.is_none() {
            font.original_path = Some(canonical.clone());
        }
        let font_directory = asset_directory.join("fonts");
        fs::create_dir_all(&font_directory)?;
        let file_name = canonical
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("font.otf");
        let destination = font_directory.join(format!("font-{}-{file_name}", font.id));
        if canonical != destination {
            fs::copy(&canonical, &destination).with_context(|| {
                format!(
                    "could not copy {} into portable Prism fonts",
                    canonical.display()
                )
            })?;
        }
        font.path = destination.strip_prefix(project_directory)?.to_owned();
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

fn permissions_allow_editable_embedding(permissions: Option<Permissions>) -> bool {
    matches!(
        permissions,
        Some(Permissions::Installable | Permissions::Editable)
    )
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
