//! Durable, app-neutral revision primitives for Spectrum creative projects.
//!
//! The crate owns revision identity, ancestry, attribution, session cursors,
//! opaque versioned payloads, snapshots, previews, and content-addressed
//! assets. Applications retain ownership of commands, state, replay,
//! rendering, and compatibility policy.
//!
//! [`LiveRevisionStore`] keeps SQLite's WAL plumbing in app-owned storage and
//! atomically publishes one self-contained project file after every mutation.

mod collaboration;
mod error;
mod id;
mod identity;
mod live;
mod metadata;
mod model;
mod schema;
mod storage_io;
mod store;
mod store_tracks;

pub use error::{RevisionError, RevisionResult};
pub use id::{AssetId, ChangeSetId, ProjectId, RevisionId, SessionId, TrackId};
pub use identity::local_session_id;
pub use live::{LiveRevisionStore, PublishStats};
pub use model::{
    Actor, ActorKind, AppendRevision, Asset, Collaboration, CollaborationMode, CollaborationStatus,
    CollaborationSync, Encoding, NewProject, NewTrack, Payload, Preview, ProjectInfo, ReplayPlan,
    ReplayStep, Revision, Session, Track,
};
pub use store::{Compatibility, RevisionStore};
