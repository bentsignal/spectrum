//! Layers drawn whole and then placed (shapes and text outside the region
//! sampler) keep their placed image between region renders: the bands of
//! one render, and the next render at the same scale, use it rather than
//! drawing and resizing the whole layer again for each.
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::Result;
use image::RgbaImage;

/// A layer drawn whole and placed: its image and how far placing moved it.
pub(crate) struct Placed {
    pub image: RgbaImage,
    pub offset: (f32, f32),
}

/// Placed images kept at most, by bytes.
const BYTES: usize = 256 << 20;

#[derive(Default)]
struct Cache {
    /// Most recently used last.
    entries: Vec<(u64, Arc<Placed>)>,
    building: HashMap<u64, Arc<Mutex<()>>>,
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The key for a layer drawn with these settings.
pub(crate) fn key(parts: &impl serde::Serialize) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(parts)
        .unwrap_or_default()
        .hash(&mut hasher);
    hasher.finish()
}

fn find(cache: &mut Cache, key: u64) -> Option<Arc<Placed>> {
    let index = cache.entries.iter().position(|(k, _)| *k == key)?;
    let entry = cache.entries.remove(index);
    let found = entry.1.clone();
    cache.entries.push(entry);
    Some(found)
}

/// The placed image for `key`, made once by `make` while other renders of
/// it wait.
pub(crate) fn placed(key: u64, make: impl FnOnce() -> Result<Placed>) -> Result<Arc<Placed>> {
    let building = {
        let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(found) = find(&mut cache, key) {
            return Ok(found);
        }
        cache.building.entry(key).or_default().clone()
    };
    let _building = building.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = find(&mut cache().lock().unwrap_or_else(|e| e.into_inner()), key) {
        return Ok(found);
    }
    let made = Arc::new(make()?);
    let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.building.remove(&key);
    cache.entries.push((key, made.clone()));
    let bytes = |entries: &[(u64, Arc<Placed>)]| {
        entries
            .iter()
            .map(|(_, placed)| placed.image.as_raw().len())
            .sum::<usize>()
    };
    while cache.entries.len() > 1 && bytes(&cache.entries) > BYTES {
        cache.entries.remove(0);
    }
    Ok(made)
}
