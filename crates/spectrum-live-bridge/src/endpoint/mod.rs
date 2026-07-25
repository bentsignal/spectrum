#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::{LocalListener, LocalStream, PeerIdentity};
#[cfg(windows)]
pub use windows::{LocalListener, LocalStream, PeerIdentity};
