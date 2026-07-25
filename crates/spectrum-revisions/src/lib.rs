//! Durable, app-neutral revision primitives for Spectrum creative projects.
//!
//! The crate owns revision identity, ancestry, attribution, session cursors,
//! opaque versioned payloads, snapshots, previews, and content-addressed
//! assets. Applications retain ownership of commands, state, replay,
//! rendering, and compatibility policy.
//!
//! [`LiveRevisionStore`] keeps SQLite's WAL plumbing in app-owned storage and
//! atomically publishes one self-contained project file after every mutation.

#[cfg(feature = "storage")]
mod collaboration;
#[cfg(feature = "storage")]
mod error;
mod id;
#[cfg(feature = "storage")]
mod identity;
#[cfg(feature = "storage")]
mod live;
#[cfg(feature = "storage")]
mod metadata;
#[cfg(feature = "storage")]
mod model;
#[cfg(feature = "storage")]
mod publish;
#[cfg(feature = "storage")]
mod schema;
#[cfg(feature = "storage")]
mod storage_io;
#[cfg(feature = "storage")]
mod store;
#[cfg(feature = "storage")]
mod store_payloads;
#[cfg(feature = "storage")]
mod store_tracks;

#[cfg(feature = "storage")]
pub use error::{RevisionError, RevisionResult};
pub use id::{AssetId, ChangeSetId, ProjectId, RevisionId, SessionId, TrackId};
#[cfg(feature = "storage")]
pub use identity::local_session_id;
#[cfg(feature = "storage")]
pub use live::{LiveRevisionStore, PublishStats, PublishStrategy, PublishTimings};
#[cfg(feature = "storage")]
pub use model::{
    Actor, ActorKind, AppendRevision, Asset, Collaboration, CollaborationMode, CollaborationStatus,
    CollaborationSync, Encoding, NewProject, NewTrack, Payload, Preview, ProjectInfo, ReplayPlan,
    ReplayStep, Revision, Session, Track,
};
#[cfg(feature = "storage")]
pub use publish::publish_noreplace;
#[cfg(feature = "storage")]
pub use store::{Compatibility, RevisionStore};
