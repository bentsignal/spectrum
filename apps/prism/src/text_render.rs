//! Compatibility facade for Prism text measurement and rasterization.
//!
//! `LegacyCharV1` remains isolated in `text_layout::legacy`; the versioned
//! shaped layout engine is routed through `text_layout` without changing old
//! project pixels.

#[path = "text_layout/mod.rs"]
mod text_layout;

pub use text_layout::{
    TextGeometry, measure_text, measure_text_geometry, measure_text_geometry_with_typography,
    measure_text_with_typography,
};
pub(crate) use text_layout::{render_text, render_text_region};

#[cfg(test)]
pub(crate) use text_layout::font_outline_scale;
