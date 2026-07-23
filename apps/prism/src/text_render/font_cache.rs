use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::Result;
use fontdue::Font;

use crate::FontAsset;

const MAX_CACHED_FONTS: usize = 64;
const MAX_CACHED_FONT_BYTES: u64 = 96 * 1024 * 1024;
const MIN_FONT_OUTLINE_SCALE: u32 = 64;
const MAX_FONT_OUTLINE_SCALE: u32 = 4_096;
// fontdue owns scale-dependent Vec<Line> geometry for every loaded glyph. Its
// public API does not expose those vector capacities, so tiers above this
// ceiling are rendered at full quality but deliberately not retained.
const MAX_CACHED_OUTLINE_SCALE: u32 = 512;
const FONT_ALLOCATION_OVERHEAD: u64 = 512 * 1024;
const FONT_SOURCE_MEMORY_MULTIPLIER: u64 = 3;
const FONTDUE_REFERENCE_OUTLINE_SCALE: u64 = 40;
const ESTIMATED_GLYPH_RECORD_BYTES: u64 = 128;
const ESTIMATED_CHARACTER_MAP_ENTRY_BYTES: u64 = 40;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
enum FontCacheKey {
    Bundled(u32),
    Imported(String, u32),
}

struct FontCacheEntry {
    font: Arc<Font>,
    estimated_bytes: u64,
}

#[derive(Default)]
struct FontCache {
    entries: HashMap<FontCacheKey, FontCacheEntry>,
    order: VecDeque<FontCacheKey>,
    estimated_bytes: u64,
}

impl FontCache {
    fn get(&mut self, key: &FontCacheKey) -> Option<Arc<Font>> {
        let font = self.entries.get(key)?.font.clone();
        self.touch(key);
        Some(font)
    }

    fn insert(
        &mut self,
        key: FontCacheKey,
        font: Arc<Font>,
        retained_weight: Option<u64>,
    ) -> Arc<Font> {
        if let Some(cached) = self.get(&key) {
            return cached;
        }
        let Some(estimated_bytes) = retained_weight else {
            return font;
        };
        if estimated_bytes > MAX_CACHED_FONT_BYTES {
            return font;
        }
        while self.entries.len() >= MAX_CACHED_FONTS
            || self.estimated_bytes.saturating_add(estimated_bytes) > MAX_CACHED_FONT_BYTES
        {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.estimated_bytes = self.estimated_bytes.saturating_sub(removed.estimated_bytes);
            }
        }
        self.entries.insert(
            key.clone(),
            FontCacheEntry {
                font: font.clone(),
                estimated_bytes,
            },
        );
        self.order.push_back(key);
        self.estimated_bytes = self.estimated_bytes.saturating_add(estimated_bytes);
        font
    }

    fn touch(&mut self, key: &FontCacheKey) {
        self.order.retain(|candidate| candidate != key);
        self.order.push_back(key.clone());
    }
}

pub(crate) fn font_outline_scale(font_size: f32) -> u32 {
    if !font_size.is_finite() {
        return MIN_FONT_OUTLINE_SCALE;
    }
    (font_size
        .ceil()
        .clamp(MIN_FONT_OUTLINE_SCALE as f32, MAX_FONT_OUTLINE_SCALE as f32) as u32)
        .next_power_of_two()
}

fn retained_font_weight(source_bytes: usize, outline_scale: u32, font: &Font) -> Option<u64> {
    if outline_scale > MAX_CACHED_OUTLINE_SCALE {
        return None;
    }
    let source_bytes = u64::try_from(source_bytes).unwrap_or(u64::MAX);
    let geometry_multiplier = u64::from(outline_scale)
        .div_ceil(FONTDUE_REFERENCE_OUTLINE_SCALE)
        .max(FONT_SOURCE_MEMORY_MULTIPLIER);
    let geometry_and_owned_tables = source_bytes.saturating_mul(geometry_multiplier);
    let glyph_records = u64::from(font.glyph_count()).saturating_mul(ESTIMATED_GLYPH_RECORD_BYTES);
    let character_map = u64::try_from(font.chars().len())
        .unwrap_or(u64::MAX)
        .saturating_mul(ESTIMATED_CHARACTER_MAP_ENTRY_BYTES);
    Some(
        geometry_and_owned_tables
            .saturating_add(glyph_records)
            .saturating_add(character_map)
            .saturating_add(FONT_ALLOCATION_OVERHEAD),
    )
}

pub(super) fn cached_font(font_asset: Option<&FontAsset>, font_size: f32) -> Result<Arc<Font>> {
    static CACHE: OnceLock<Mutex<FontCache>> = OnceLock::new();
    let outline_scale = font_outline_scale(font_size);
    let key = font_asset.map_or(FontCacheKey::Bundled(outline_scale), |font| {
        FontCacheKey::Imported(font.content_hash.clone(), outline_scale)
    });
    let cache = CACHE.get_or_init(|| Mutex::new(FontCache::default()));
    if let Some(font) = cache
        .lock()
        .map_err(|_| anyhow::anyhow!("text font cache is unavailable"))?
        .get(&key)
    {
        return Ok(font);
    }

    let settings = fontdue::FontSettings {
        scale: outline_scale as f32,
        ..fontdue::FontSettings::default()
    };
    let (font, source_bytes) = if let Some(asset) = font_asset {
        let bytes = asset.bytes()?;
        let source_bytes = bytes.len();
        (
            Font::from_bytes(bytes, settings)
                .map_err(|error| anyhow::anyhow!("could not load imported font: {error}"))?,
            source_bytes,
        )
    } else {
        (
            Font::from_bytes(epaint_default_fonts::UBUNTU_LIGHT, settings)
                .map_err(|error| anyhow::anyhow!("could not load bundled font: {error}"))?,
            epaint_default_fonts::UBUNTU_LIGHT.len(),
        )
    };
    let font = Arc::new(font);
    let mut cache = cache
        .lock()
        .map_err(|_| anyhow::anyhow!("text font cache is unavailable"))?;
    let retained_weight = retained_font_weight(source_bytes, outline_scale, &font);
    Ok(cache.insert(key, font, retained_weight))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_font() -> Arc<Font> {
        Arc::new(
            Font::from_bytes(
                epaint_default_fonts::UBUNTU_LIGHT,
                fontdue::FontSettings::default(),
            )
            .unwrap(),
        )
    }

    #[test]
    fn oversized_font_is_not_retained() {
        let mut cache = FontCache::default();
        let key = FontCacheKey::Imported("oversized".into(), 64);
        let font = test_font();
        let returned = cache.insert(key.clone(), font.clone(), Some(MAX_CACHED_FONT_BYTES + 1));
        assert!(Arc::ptr_eq(&returned, &font));
        assert!(!cache.entries.contains_key(&key));
        assert_eq!(cache.estimated_bytes, 0);
    }

    #[test]
    fn cache_evicts_by_estimated_bytes_and_entry_count() {
        let mut cache = FontCache::default();
        let entry_bytes = MAX_CACHED_FONT_BYTES / 3;
        for index in 0..4 {
            cache.insert(
                FontCacheKey::Imported(format!("font-{index}"), 64),
                test_font(),
                Some(entry_bytes),
            );
        }
        assert!(cache.estimated_bytes <= MAX_CACHED_FONT_BYTES);
        assert_eq!(cache.entries.len(), 3);
        assert!(
            !cache
                .entries
                .contains_key(&FontCacheKey::Imported("font-0".into(), 64))
        );
    }

    #[test]
    fn admitted_tier_weight_uses_parsed_geometry_metadata_and_outline_scale() {
        let font = test_font();
        let source_bytes = epaint_default_fonts::UBUNTU_LIGHT.len();
        let low = retained_font_weight(source_bytes, 64, &font).unwrap();
        let highest_admitted = retained_font_weight(source_bytes, 512, &font).unwrap();
        assert!(highest_admitted > low * 2);
        assert!(
            highest_admitted
                > u64::try_from(source_bytes).unwrap() * FONT_SOURCE_MEMORY_MULTIPLIER
                    + FONT_ALLOCATION_OVERHEAD
        );
    }

    #[test]
    fn high_tier_fonts_render_but_are_never_retained() {
        let outline_scale = 4_096;
        let font = Arc::new(
            Font::from_bytes(
                epaint_default_fonts::UBUNTU_LIGHT,
                fontdue::FontSettings {
                    scale: outline_scale as f32,
                    ..fontdue::FontSettings::default()
                },
            )
            .unwrap(),
        );
        let retained_weight = retained_font_weight(
            epaint_default_fonts::UBUNTU_LIGHT.len(),
            outline_scale,
            &font,
        );
        assert_eq!(retained_weight, None);

        let mut cache = FontCache::default();
        for index in 0..4 {
            let returned = cache.insert(
                FontCacheKey::Imported(format!("high-tier-{index}"), outline_scale),
                font.clone(),
                retained_weight,
            );
            assert!(Arc::ptr_eq(&returned, &font));
        }
        assert!(cache.entries.is_empty());
        assert_eq!(cache.estimated_bytes, 0);
    }

    #[test]
    fn huge_admitted_source_weight_exceeds_the_global_cache_budget() {
        let font = test_font();
        assert!(retained_font_weight(40 * 1024 * 1024, 64, &font).unwrap() > MAX_CACHED_FONT_BYTES);
    }
}
