use std::collections::HashMap;

use crate::{Layer, LayerKind};

pub enum LayerPreviewSchedule<T> {
    ReuseCachedTextVisual,
    Resolve(T),
}

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

    /// Runs the same scheduling gate as the GUI before visual-key cloning,
    /// font lookup, text shaping, or preview-size resolution.
    pub fn schedule_layer<R>(
        &self,
        layer: &Layer,
        interaction_active: bool,
        visual_cached: bool,
        visual_dirty: bool,
        resolve: impl FnOnce() -> R,
    ) -> LayerPreviewSchedule<R> {
        if interaction_active
            && matches!(layer.kind, LayerKind::Text { .. })
            && self.entries.contains_key(&layer.id)
            && visual_cached
            && !visual_dirty
        {
            LayerPreviewSchedule::ReuseCachedTextVisual
        } else {
            LayerPreviewSchedule::Resolve(resolve())
        }
    }
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
    fn scheduling_gate_skips_expensive_resolution_only_for_clean_cached_text() {
        let layer = Layer {
            id: 7,
            kind: LayerKind::Text {
                text: "Long transform text ".repeat(10_000),
                font_size: 72.0,
                color: [255; 4],
                typography: Default::default(),
            },
            ..Layer::default()
        };
        let mut cache = TextPreviewFrameCache::default();
        cache.insert(layer.id, "geometry");
        let mut resolved = false;
        let schedule = cache.schedule_layer(&layer, true, true, false, || {
            resolved = true;
        });
        assert!(matches!(
            schedule,
            LayerPreviewSchedule::ReuseCachedTextVisual
        ));
        assert!(!resolved);

        let schedule = cache.schedule_layer(&layer, true, true, true, || 42);
        assert!(matches!(schedule, LayerPreviewSchedule::Resolve(42)));
    }
}
