use anyhow::{Context, Result, bail};
use ttf_parser::Face;
use unicode_segmentation::UnicodeSegmentation;

use crate::FontAsset;

pub(super) const BUNDLED_UBUNTU: &[u8] = epaint_default_fonts::UBUNTU_LIGHT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FaceChoice {
    Primary,
    Bundled,
}

pub(super) struct ResolvedFonts {
    primary: Vec<u8>,
    primary_is_bundled: bool,
}

impl ResolvedFonts {
    pub(super) fn new(font_asset: Option<&FontAsset>) -> Result<Self> {
        let (primary, primary_is_bundled) = match font_asset {
            Some(asset) => (
                asset
                    .bytes()
                    .context("could not load the exact imported font snapshot")?,
                false,
            ),
            None => (BUNDLED_UBUNTU.to_vec(), true),
        };
        Face::parse(&primary, 0).context("primary text font is malformed")?;
        Face::parse(BUNDLED_UBUNTU, 0).context("bundled Ubuntu fallback is malformed")?;
        Ok(Self {
            primary,
            primary_is_bundled,
        })
    }

    pub(super) fn from_primary_bytes(primary: &[u8]) -> Result<Self> {
        Face::parse(primary, 0).context("primary text font is malformed")?;
        Face::parse(BUNDLED_UBUNTU, 0).context("bundled Ubuntu fallback is malformed")?;
        Ok(Self {
            primary: primary.to_vec(),
            primary_is_bundled: false,
        })
    }

    pub(super) fn bytes(&self, choice: FaceChoice) -> &[u8] {
        match choice {
            FaceChoice::Primary => &self.primary,
            FaceChoice::Bundled => BUNDLED_UBUNTU,
        }
    }

    pub(super) fn faces(&self) -> Result<(Face<'_>, Face<'static>)> {
        Ok((
            Face::parse(&self.primary, 0).context("primary text font is malformed")?,
            Face::parse(BUNDLED_UBUNTU, 0).context("bundled Ubuntu fallback is malformed")?,
        ))
    }

    pub(super) fn choose_grapheme(
        &self,
        primary: &Face<'_>,
        bundled: &Face<'_>,
        grapheme: &str,
    ) -> FaceChoice {
        debug_assert_eq!(grapheme.graphemes(true).count(), 1);
        if grapheme_covered(primary, grapheme) {
            return FaceChoice::Primary;
        }
        if !self.primary_is_bundled && grapheme_covered(bundled, grapheme) {
            return FaceChoice::Bundled;
        }
        // A visible missing cluster deliberately resolves to primary glyph zero.
        FaceChoice::Primary
    }
}

fn grapheme_covered(face: &Face<'_>, grapheme: &str) -> bool {
    let mut characters = grapheme.chars().peekable();
    while let Some(character) = characters.next() {
        if is_default_ignorable(character) {
            continue;
        }
        if characters
            .peek()
            .is_some_and(|selector| is_variation_selector(*selector))
        {
            let selector = characters.next().expect("peeked selector");
            if face.glyph_variation_index(character, selector).is_none() {
                return false;
            }
        } else if face.glyph_index(character).is_none() {
            return false;
        }
    }
    true
}

fn is_variation_selector(character: char) -> bool {
    matches!(character as u32, 0xfe00..=0xfe0f | 0xe0100..=0xe01ef)
}

fn is_default_ignorable(character: char) -> bool {
    matches!(
        character as u32,
        0x00ad
            | 0x034f
            | 0x061c
            | 0x115f..=0x1160
            | 0x17b4..=0x17b5
            | 0x180b..=0x180f
            | 0x200b..=0x200f
            | 0x202a..=0x202e
            | 0x2060..=0x206f
            | 0x3164
            | 0xfe00..=0xfe0f
            | 0xfeff
            | 0xffa0
            | 0x1bca0..=0x1bca3
            | 0x1d173..=0x1d17a
            | 0xe0000..=0xe0fff
    )
}

pub(super) fn validate_grapheme_boundaries(text: &str) -> Result<()> {
    if text.graphemes(true).count() > 16_384 {
        bail!("shaped text exceeds the grapheme resource limit");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_is_whole_grapheme_and_never_uses_os_fonts() {
        let fonts = ResolvedFonts {
            primary: epaint_default_fonts::HACK_REGULAR.to_vec(),
            primary_is_bundled: false,
        };
        let (primary, bundled) = fonts.faces().unwrap();
        assert_eq!(
            fonts.choose_grapheme(&primary, &bundled, "A"),
            FaceChoice::Primary
        );
        let fallback_character = (0..=0x10ffff)
            .filter_map(char::from_u32)
            .find(|character| {
                primary.glyph_index(*character).is_none()
                    && bundled.glyph_index(*character).is_some()
            })
            .expect("fixtures must exercise deterministic Ubuntu fallback");
        let mut encoded = [0; 4];
        assert_eq!(
            fonts.choose_grapheme(
                &primary,
                &bundled,
                fallback_character.encode_utf8(&mut encoded)
            ),
            FaceChoice::Bundled
        );
        assert_eq!(
            fonts.choose_grapheme(&primary, &bundled, "\u{10ffff}"),
            FaceChoice::Primary,
            "a cluster missing from both faces resolves to primary glyph zero"
        );
    }
}
