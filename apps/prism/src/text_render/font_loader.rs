use anyhow::Result;
use fontdue::Font;

use crate::FontAsset;

const MIN_FONT_OUTLINE_SCALE: u32 = 64;
const MAX_FONT_OUTLINE_SCALE: u32 = 4_096;
const RETAINED_FONTDUE_CACHE_BYTES: u64 = 0;

pub(crate) fn font_outline_scale(font_size: f32) -> u32 {
    if !font_size.is_finite() {
        return MIN_FONT_OUTLINE_SCALE;
    }
    (font_size
        .ceil()
        .clamp(MIN_FONT_OUTLINE_SCALE as f32, MAX_FONT_OUTLINE_SCALE as f32) as u32)
        .next_power_of_two()
}

/// Constructs one caller-owned fontdue object and retains no parsed font state.
///
/// fontdue keeps scale-dependent outline vectors private, so their owned
/// capacities cannot be measured or safely admitted to a process-wide cache.
pub(super) fn load_font(font_asset: Option<&FontAsset>, font_size: f32) -> Result<Font> {
    debug_assert_eq!(RETAINED_FONTDUE_CACHE_BYTES, 0);
    let settings = fontdue::FontSettings {
        scale: font_outline_scale(font_size) as f32,
        ..fontdue::FontSettings::default()
    };
    if let Some(asset) = font_asset {
        Font::from_bytes(asset.bytes()?, settings)
            .map_err(|error| anyhow::anyhow!("could not load imported font: {error}"))
    } else {
        Font::from_bytes(epaint_default_fonts::UBUNTU_LIGHT, settings)
            .map_err(|error| anyhow::anyhow!("could not load bundled font: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_outline_tier_uses_ephemeral_caller_owned_fonts() {
        for tier in [64, 128, 256, 512, 1_024, 2_048, 4_096] {
            let font = load_font(None, tier as f32).unwrap();
            assert_eq!(font.glyph_count(), 1_261);
            drop(font);
        }
    }

    #[test]
    fn eighteen_tier_512_loads_leave_no_cache_to_undercharge() {
        for _ in 0..18 {
            let font = load_font(None, 512.0).unwrap();
            assert_eq!(font.glyph_count(), 1_261);
            drop(font);
        }
        // The rejected estimator admitted these tier-512 objects at roughly
        // half their measured retained size. The replacement cannot
        // undercharge them because its retained budget is zero by construction.
        assert_eq!(RETAINED_FONTDUE_CACHE_BYTES, 0);
    }
}
