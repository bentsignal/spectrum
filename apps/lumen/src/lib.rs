//! Lumen's catalog and command engine.

pub mod command;
pub mod engine;
mod live_bridge;
mod live_bridge_host;
mod live_bridge_sessions;
pub mod preview;
pub mod project;
mod revisions;

pub use command::{Command, CommandOutput, Workspace};
pub use engine::ExportFormat;
pub use live_bridge::{
    LUMEN_COMMAND_OPERATIONS_VERSION, LUMEN_LIVE_ACTION_FAMILY, LUMEN_LIVE_ACTION_VERSION,
    LUMEN_LIVE_APPLICATION, LumenLiveAction, LumenLiveActionExpectation, LumenLiveApplied,
    LumenLiveCollaborationSync, LumenLiveResult, LumenLiveState, decode_live_action,
    lumen_live_discovery_root,
};
pub use live_bridge_host::{
    LumenLiveDrain, LumenLiveDrainReport, LumenLiveHost, LumenLiveInteractionState,
};
#[cfg(test)]
pub(crate) use live_bridge_sessions::LumenLiveTestFault;
pub use live_bridge_sessions::{LumenLiveApplyError, LumenLiveSessions};
pub use project::{HistoryEntry, Photo, PhotoBatch, PhotoMetadata, PickState, Preset, Project};
pub use revisions::{DurableCatalog, LiveWorkspaceState, ProjectHistory};
pub use spectrum_imaging::{
    AdjustmentPatch, Adjustments, ColorGrade, ColorGrading, CropRect, CurvePoint, HslAdjustments,
    HslBand, SpotRemoval, ToneCurve, ToneCurves,
};
pub use spectrum_imaging::{adjustments, render};

#[cfg(test)]
#[path = "live_bridge_tests.rs"]
mod live_bridge_tests;
