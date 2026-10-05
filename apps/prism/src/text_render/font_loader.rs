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

/// Parsed fonts kept for interactive clients: the last few font and outline
/// scale pairs, so dragging a text slider does not re-parse every glyph.
/// Off by default (exports and tests retain nothing), and dropped when the
/// interactive caches are turned off.
type ParsedFont = (String, u32, std::sync::Arc<Font>);
static PARSED: std::sync::Mutex<Vec<ParsedFont>> = std::sync::Mutex::new(Vec::new());
const KEPT_PARSED_FONTS: usize = 3;

pub(crate) fn clear_parsed_fonts() {
    PARSED.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// The font to lay out and draw text with. Outside interactive clients
/// every call constructs a caller-owned fontdue object and keeps nothing.
pub(super) fn load_font(
    font_asset: Option<&FontAsset>,
    font_size: f32,
) -> Result<std::sync::Arc<Font>> {
    debug_assert_eq!(RETAINED_FONTDUE_CACHE_BYTES, 0);
    let scale = font_outline_scale(font_size);
    let key = font_asset.map_or("bundled", |asset| asset.content_hash.as_str());
    let interactive = crate::render_region::interactive_caches();
    if interactive {
        let parsed = PARSED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, _, font)) = parsed.iter().find(|(k, s, _)| k == key && *s == scale) {
            return Ok(font.clone());
        }
    }
    let settings = fontdue::FontSettings {
        scale: scale as f32,
        ..fontdue::FontSettings::default()
    };
    let font = std::sync::Arc::new(if let Some(asset) = font_asset {
        Font::from_bytes(&*asset.shared_bytes()?, settings)
            .map_err(|error| anyhow::anyhow!("could not load imported font: {error}"))?
    } else {
        Font::from_bytes(epaint_default_fonts::UBUNTU_LIGHT, settings)
            .map_err(|error| anyhow::anyhow!("could not load bundled font: {error}"))?
    });
    if interactive {
        let mut parsed = PARSED.lock().unwrap_or_else(|e| e.into_inner());
        parsed.retain(|(k, s, _)| !(k == key && *s == scale));
        parsed.push((key.to_owned(), scale, font.clone()));
        while parsed.len() > KEPT_PARSED_FONTS {
            parsed.remove(0);
        }
    }
    Ok(font)
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
