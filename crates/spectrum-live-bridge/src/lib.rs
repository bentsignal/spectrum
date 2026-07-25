//! App-neutral, authenticated local transport for live Spectrum integrations.
//!
//! The bridge owns discovery, authentication, framing, ordering, and bounded
//! transport state. Applications remain responsible for decoding opaque
//! actions, checking their own command compatibility, and applying mutations.

mod auth;
mod client;
mod discovery;
mod endpoint;
mod error;
mod event_log;
mod framing;
mod limits;
mod protocol;
mod request_cache;
mod server;
#[cfg(windows)]
mod windows_security;

pub use auth::{AuthChallenge, AuthProof, Capability, verify_proof};
pub use client::{BridgeClient, ClientConfig};
pub use discovery::{
    DiscoveryDirectory, DiscoveryLease, DiscoveryRecord, EndpointAddress, PublishedBinding,
};
pub use endpoint::{LocalListener, LocalStream, PeerIdentity};
pub use error::{BridgeError, BridgeResult};
pub use event_log::{EventLog, Subscription};
pub use framing::{FrameReader, read_frame, write_frame};
pub use limits::*;
pub use protocol::*;
pub use request_cache::{CachedResponse, RequestCache, RequestLookup};
pub use server::{BridgeHost, BridgeServer, HostApplyOutcome, ServerConfig};
