//! Canvas text measurement and rasterization.

#[path = "text_layout/mod.rs"]
mod text_layout;

pub use text_layout::{
    TextGeometry, measure_text, measure_text_geometry, measure_text_geometry_with_typography,
    measure_text_with_typography,
};
pub(crate) use text_layout::{render_text, render_text_region};
