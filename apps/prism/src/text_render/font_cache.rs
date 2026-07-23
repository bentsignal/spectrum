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
const FONT_ALLOCATION_OVERHEAD: u64 = 512 * 1024;
const FONT_SOURCE_MEMORY_MULTIPLIER: u64 = 3;

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

    fn insert(&mut self, key: FontCacheKey, font: Arc<Font>, estimated_bytes: u64) -> Arc<Font> {
        if let Some(cached) = self.get(&key) {
            return cached;
        }
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

fn estimated_font_bytes(source_bytes: usize) -> u64 {
    u64::try_from(source_bytes)
        .unwrap_or(u64::MAX)
        .saturating_mul(FONT_SOURCE_MEMORY_MULTIPLIER)
        .saturating_add(FONT_ALLOCATION_OVERHEAD)
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
    Ok(cache.insert(key, font, estimated_font_bytes(source_bytes)))
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
        let returned = cache.insert(key.clone(), font.clone(), MAX_CACHED_FONT_BYTES + 1);
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
                entry_bytes,
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
    fn huge_source_estimate_exceeds_the_global_cache_budget() {
        assert!(estimated_font_bytes(40 * 1024 * 1024) > MAX_CACHED_FONT_BYTES);
    }
}
