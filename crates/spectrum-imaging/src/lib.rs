//! App-neutral image adjustment and rendering primitives for Spectrum.

pub mod adjustments;
pub mod render;

pub use adjustments::{
    AdjustmentPatch, Adjustments, ColorGrade, ColorGrading, CropRect, CurvePoint, HslAdjustments,
    HslBand, SpotRemoval, ToneCurve, ToneCurves,
};
pub use render::{
    ExportFormat, PixelRegion, RegionRenderError, RenderOptions, adjusted_image_dimensions,
    render_image, render_image_region,
};
