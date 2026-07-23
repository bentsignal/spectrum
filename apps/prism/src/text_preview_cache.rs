use std::collections::HashMap;

/// Source geometry retained across transform and zoom frames.
///
/// The GUI invalidates an entry only when the text, typography, or font identity
/// changes, keeping interactive transform frames independent of shaping cost.
pub struct TextPreviewFrameCache<T> {
    entries: HashMap<u64, T>,
}

impl<T> Default for TextPreviewFrameCache<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<T> TextPreviewFrameCache<T> {
    pub fn get(&self, layer_id: u64) -> Option<&T> {
        self.entries.get(&layer_id)
    }

    pub fn insert(&mut self, layer_id: u64, geometry: T) {
        self.entries.insert(layer_id, geometry);
    }

    pub fn remove(&mut self, layer_id: u64) {
        self.entries.remove(&layer_id);
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&u64, &mut T) -> bool) {
        self.entries
            .retain(|layer_id, geometry| keep(layer_id, geometry));
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

pub fn reuse_text_preview_frame(
    interaction_active: bool,
    is_text: bool,
    geometry_cached: bool,
    visual_cached: bool,
    visual_dirty: bool,
) -> bool {
    interaction_active && is_text && geometry_cached && visual_cached && !visual_dirty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_prunes_deleted_layers_without_touching_survivors() {
        let mut cache = TextPreviewFrameCache::default();
        cache.insert(1, "first");
        cache.insert(2, "second");
        cache.retain(|id, _| *id == 2);
        assert_eq!(cache.get(2), Some(&"second"));
        assert_eq!(cache.get(1), None);
    }

    #[test]
    fn interaction_reuse_requires_clean_visual_and_source_geometry() {
        assert!(reuse_text_preview_frame(true, true, true, true, false));
        assert!(!reuse_text_preview_frame(true, true, false, true, false));
        assert!(!reuse_text_preview_frame(true, true, true, true, true));
        assert!(!reuse_text_preview_frame(false, true, true, true, false));
    }
}
