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
            content_hash: Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            path: fs::canonicalize(path)
                .with_context(|| format!("could not resolve font {}", path.display()))?,
            original_path: Some(fs::canonicalize(path)?),
        })
    }

    pub fn bytes(&self) -> Result<Vec<u8>> {
        fs::read(&self.path)
            .with_context(|| format!("could not read embedded font {}", self.path.display()))
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
    /// Line-spacing multiplier. `1.25` exactly preserves Prism's legacy spacing;
    /// other values scale that baseline proportionally.
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
}
