//! Spectrum's image assets: a source photo and non-destructive edits to it,
//! durable through spectrum-document, and the engine that develops RAW
//! files and renders, previews, and exports edited images.

mod commands;
mod document;
pub mod engine;

pub use commands::{Command, CommandOutput, ImageModel, Workspace};
pub use document::{Image, ImageMetadata, is_raw_image, is_supported_image};
pub use engine::ExportFormat;
pub use spectrum_imaging::{
    AdjustmentPatch, Adjustments, ColorGrade, ColorGrading, CropRect, CurvePoint, HslAdjustments,
    HslBand, SpotRemoval, ToneCurve, ToneCurves,
};
pub use spectrum_imaging::{adjustments, downscale, render};
