//! App-neutral image adjustment and rendering primitives for Spectrum.

pub mod adjustments;
pub mod region_source;
pub mod render;

pub use adjustments::{
    AdjustmentPatch, Adjustments, ColorGrade, ColorGrading, CropRect, CurvePoint, HslAdjustments,
    HslBand, SpotRemoval, ToneCurve, ToneCurves,
};
pub use region_source::{
    DynExactRegionSource, ExactRegionSource, RegionReadCapability, RegionReadiness,
    RegionRequestError, RegionSourceDescriptor, RegionSourceInfo, SourceSampleDepth,
    validate_region_request,
};
pub use render::{
    AdjustedPixelSourceMapper, ExportFormat, PixelRegion, RegionRenderError, RenderOptions,
    adjusted_image_dimensions, render_image, render_image_region_at_source_resolution,
    render_image_region_at_source_resolution_bounded,
};
