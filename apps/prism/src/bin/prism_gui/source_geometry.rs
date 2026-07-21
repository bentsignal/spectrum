use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct LayerSourceOverride {
    kind: LayerKind,
    geometry: LayerSourceGeometry,
}

impl LayerSourceOverride {
    pub(super) fn new(kind: LayerKind, geometry: LayerSourceGeometry) -> Self {
        Self { kind, geometry }
    }
}

pub(super) fn current_layer_source_geometry(
    layer: &Layer,
    source_override: Option<&LayerSourceOverride>,
    cached: Option<(&LayerVisualKey, LayerSourceGeometry)>,
    font_asset: Option<&prism_core::FontAsset>,
) -> Option<LayerSourceGeometry> {
    current_layer_source_geometry_with_resolver(layer, source_override, cached, || {
        source_geometry_before_preview(layer, font_asset)
    })
}

pub(super) fn current_layer_source_geometry_with_resolver(
    layer: &Layer,
    source_override: Option<&LayerSourceOverride>,
    cached: Option<(&LayerVisualKey, LayerSourceGeometry)>,
    resolver: impl FnOnce() -> Option<LayerSourceGeometry>,
) -> Option<LayerSourceGeometry> {
    source_override
        .filter(|source_override| source_override.kind == layer.kind)
        .map(|source_override| source_override.geometry)
        .or_else(|| {
            cached
                .filter(|(key, _)| key.kind == layer.kind)
                .map(|(_, geometry)| geometry)
        })
        .or_else(resolver)
}
