use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::Hash,
};

pub(super) const MAX_PREVIEW_TEXTURE_SIDE: u32 = 8_192;
pub(super) const MAX_PREVIEW_TEXTURE_BYTES: u64 = 64 * 1024 * 1024;
pub(super) const MAX_PREVIEW_TEXTURE_RESIDENT_BYTES: u64 = 256 * 1024 * 1024;
pub(super) const MAX_WORKER_BASE_BYTES: u64 = 256 * 1024 * 1024;

pub(super) fn rgba_bytes(width: u32, height: u32) -> u64 {
    u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(4)
}

pub(super) fn per_layer_texture_bytes(visible_texture_layers: usize) -> u64 {
    let fair_share = MAX_PREVIEW_TEXTURE_RESIDENT_BYTES
        / u64::try_from(visible_texture_layers.max(1)).unwrap_or(u64::MAX);
    fair_share.min(MAX_PREVIEW_TEXTURE_BYTES)
}

pub(super) fn resident_texture_copies(colored_shadow_mask: bool) -> u32 {
    // Reserve one CPU-side image and one GPU allocation per egui texture.
    2 * (1 + u32::from(colored_shadow_mask))
}

pub(super) fn preview_texture_resident_bytes(
    width: u32,
    height: u32,
    colored_shadow_mask: bool,
) -> u64 {
    rgba_bytes(width, height)
        .saturating_mul(u64::from(resident_texture_copies(colored_shadow_mask)))
}

pub(super) fn preview_upload_allowed(
    width: u32,
    height: u32,
    runtime_max_texture_side: usize,
    resident_byte_budget: u64,
    colored_shadow_mask: bool,
) -> bool {
    width as usize <= runtime_max_texture_side
        && height as usize <= runtime_max_texture_side
        && preview_texture_resident_bytes(width, height, colored_shadow_mask)
            <= resident_byte_budget
}

pub(super) fn bounded_preview_max_size(
    width: u32,
    height: u32,
    raster_scale: u32,
    runtime_max_texture_side: usize,
    resident_copies: u32,
    layer_byte_budget: u64,
) -> u32 {
    let width = u64::from(width.max(1)).saturating_mul(u64::from(raster_scale.max(1)));
    let height = u64::from(height.max(1)).saturating_mul(u64::from(raster_scale.max(1)));
    let longest = width.max(height);
    let runtime_side = u64::try_from(runtime_max_texture_side)
        .unwrap_or(u64::MAX)
        .min(u64::from(MAX_PREVIEW_TEXTURE_SIDE))
        .max(1);
    let byte_budget = layer_byte_budget
        .min(MAX_PREVIEW_TEXTURE_BYTES)
        .checked_div(u64::from(resident_copies.max(1)))
        .unwrap_or(0)
        .max(4);
    let pixel_budget = byte_budget / 4;
    let pixels = width.saturating_mul(height);
    let area_scale = if pixels > pixel_budget {
        (pixel_budget as f64 / pixels as f64).sqrt()
    } else {
        1.0
    };
    let side_scale = if longest > runtime_side {
        runtime_side as f64 / longest as f64
    } else {
        1.0
    };
    ((longest as f64 * area_scale.min(side_scale))
        .floor()
        .max(1.0) as u64)
        .min(runtime_side)
        .try_into()
        .unwrap_or(u32::MAX)
}

pub(super) struct BoundedLruCache<K, V> {
    entries: HashMap<K, (V, u64)>,
    order: VecDeque<K>,
    total_bytes: u64,
    max_bytes: u64,
}

impl<K: Copy + Eq + Hash, V> BoundedLruCache<K, V> {
    pub(super) fn new(max_bytes: u64) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            total_bytes: 0,
            max_bytes,
        }
    }

    pub(super) fn get(&mut self, key: K) -> Option<&V> {
        if !self.entries.contains_key(&key) {
            return None;
        }
        self.touch(key);
        self.entries.get(&key).map(|(value, _)| value)
    }

    pub(super) fn insert(&mut self, key: K, value: V, bytes: u64) {
        self.remove(key);
        if bytes > self.max_bytes {
            return;
        }
        while self.total_bytes.saturating_add(bytes) > self.max_bytes {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some((_, removed_bytes)) = self.entries.remove(&oldest) {
                self.total_bytes = self.total_bytes.saturating_sub(removed_bytes);
            }
        }
        self.entries.insert(key, (value, bytes));
        self.order.push_back(key);
        self.total_bytes = self.total_bytes.saturating_add(bytes);
    }

    pub(super) fn retain(&mut self, active: &HashSet<K>) {
        let removed = self
            .entries
            .keys()
            .copied()
            .filter(|key| !active.contains(key))
            .collect::<Vec<_>>();
        for key in removed {
            self.remove(key);
        }
    }

    fn touch(&mut self, key: K) {
        self.order.retain(|candidate| *candidate != key);
        self.order.push_back(key);
    }

    fn remove(&mut self, key: K) {
        self.order.retain(|candidate| *candidate != key);
        if let Some((_, bytes)) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(bytes);
        }
    }

    #[cfg(test)]
    fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    #[cfg(test)]
    fn contains_key(&self, key: K) -> bool {
        self.entries.contains_key(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_8k_request_is_reduced_to_the_pixel_budget() {
        let max_size =
            bounded_preview_max_size(8_192, 8_192, 1, 16_384, 1, MAX_PREVIEW_TEXTURE_BYTES);
        assert_eq!(max_size, 4_096);
        assert!(rgba_bytes(max_size, max_size) <= MAX_PREVIEW_TEXTURE_BYTES);
    }

    #[test]
    fn runtime_device_limit_always_wins_before_upload() {
        let max_size =
            bounded_preview_max_size(8_192, 2_048, 1, 2_048, 1, MAX_PREVIEW_TEXTURE_BYTES);
        assert_eq!(max_size, 2_048);
        assert!(!preview_upload_allowed(
            2_049,
            1_024,
            2_048,
            MAX_PREVIEW_TEXTURE_BYTES,
            false,
        ));
        assert!(preview_upload_allowed(
            2_048,
            1_024,
            2_048,
            MAX_PREVIEW_TEXTURE_BYTES,
            false,
        ));
    }

    #[test]
    fn visible_layers_share_one_aggregate_budget() {
        let layer_budget = per_layer_texture_bytes(8);
        assert_eq!(layer_budget * 8, MAX_PREVIEW_TEXTURE_RESIDENT_BYTES);
        let copies = resident_texture_copies(false);
        let max_size = bounded_preview_max_size(8_192, 8_192, 1, 8_192, copies, layer_budget);
        assert!(
            rgba_bytes(max_size, max_size) * u64::from(copies) * 8
                <= MAX_PREVIEW_TEXTURE_RESIDENT_BYTES
        );
    }

    #[test]
    fn colored_shadow_accounts_for_cpu_and_gpu_residency_of_both_textures() {
        let copies = resident_texture_copies(true);
        assert_eq!(copies, 4);
        let max_size =
            bounded_preview_max_size(8_192, 8_192, 1, 8_192, copies, MAX_PREVIEW_TEXTURE_BYTES);
        assert!(rgba_bytes(max_size, max_size) * u64::from(copies) <= MAX_PREVIEW_TEXTURE_BYTES);
    }

    #[test]
    fn aggregate_worker_ui_and_upload_peak_has_an_explicit_ceiling() {
        let pipeline_peak =
            MAX_WORKER_BASE_BYTES + MAX_PREVIEW_TEXTURE_RESIDENT_BYTES + MAX_PREVIEW_TEXTURE_BYTES;
        assert_eq!(pipeline_peak, 576 * 1024 * 1024);
    }

    #[test]
    fn worker_cache_evicts_and_releases_deleted_layers() {
        let mut cache = BoundedLruCache::new(12);
        cache.insert((1, 1), "first", 8);
        cache.insert((1, 2), "second", 8);
        assert!(!cache.contains_key((1, 1)));
        assert!(cache.contains_key((1, 2)));
        assert_eq!(cache.total_bytes(), 8);
        cache.retain(&HashSet::from([(1, 2)]));
        assert_eq!(cache.total_bytes(), 8);
    }

    #[test]
    fn worker_cache_releases_closed_tabs() {
        let mut cache = BoundedLruCache::new(12);
        cache.insert((1, 2), "first tab", 8);
        cache.insert((2, 1), "other tab", 4);
        cache.retain(&HashSet::from([(2, 1)]));
        assert!(!cache.contains_key((1, 2)));
        assert!(cache.contains_key((2, 1)));
        assert_eq!(cache.total_bytes(), 4);
    }
}
