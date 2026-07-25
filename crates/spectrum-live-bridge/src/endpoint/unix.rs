use std::{
    fs,
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{FileTypeExt, MetadataExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{BridgeError, BridgeResult, EndpointAddress};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerIdentity {
    pub user_id: u32,
}

pub struct LocalListener {
    inner: UnixListener,
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl LocalListener {
    pub fn bind(address: &EndpointAddress) -> BridgeResult<Self> {
        let EndpointAddress::Unix { path } = address else {
            return Err(BridgeError::Unsupported("non-Unix endpoint"));
        };
        let parent = path
            .parent()
            .ok_or_else(|| BridgeError::Protocol("socket has no parent".into()))?;
        let metadata = fs::symlink_metadata(parent)?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o700
        {
            return Err(BridgeError::Authentication(
                "socket parent must be user-owned mode 0700".into(),
            ));
        }
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(existing) => {
                if !existing.file_type().is_socket()
                    || existing.uid() != unsafe { libc::geteuid() }
                    || existing.nlink() != 1
                    || existing.mode() & 0o777 != 0o600
                {
                    return Err(BridgeError::Authentication(
                        "refusing to replace an untrusted existing endpoint".into(),
                    ));
                }
                match UnixStream::connect(path) {
                    Ok(_) => {
                        return Err(BridgeError::Authentication(
                            "refusing to replace a live endpoint".into(),
                        ));
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                        ) =>
                    {
                        let confirmed = fs::symlink_metadata(path)?;
                        if confirmed.dev() != existing.dev()
                            || confirmed.ino() != existing.ino()
                            || !confirmed.file_type().is_socket()
                        {
                            return Err(BridgeError::Authentication(
                                "endpoint identity changed during stale cleanup".into(),
                            ));
                        }
                        fs::remove_file(path)?;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            Err(error) => return Err(error.into()),
        }
        let inner = UnixListener::bind(path)?;
        fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        let socket = fs::symlink_metadata(path)?;
        if !socket.file_type().is_socket()
            || socket.uid() != unsafe { libc::geteuid() }
            || socket.nlink() != 1
            || socket.mode() & 0o777 != 0o600
        {
            let _ = fs::remove_file(path);
            return Err(BridgeError::Authentication(
                "created socket failed ownership checks".into(),
            ));
        }
        Ok(Self {
            inner,
            path: path.clone(),
            device: socket.dev(),
            inode: socket.ino(),
        })
    }

    pub fn accept(&self) -> BridgeResult<(LocalStream, PeerIdentity)> {
        let (inner, _) = self.inner.accept()?;
        let identity = peer_identity(&inner)?;
        if identity.user_id != unsafe { libc::geteuid() } {
            return Err(BridgeError::Authentication(
                "local peer belongs to another user".into(),
            ));
        }
        Ok((LocalStream { inner }, identity))
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> BridgeResult<()> {
        self.inner.set_nonblocking(nonblocking)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for LocalListener {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path)
            && metadata.file_type().is_socket()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.nlink() == 1
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub struct LocalStream {
    inner: UnixStream,
}

impl LocalStream {
    pub fn connect(address: &EndpointAddress) -> BridgeResult<Self> {
        let EndpointAddress::Unix { path } = address else {
            return Err(BridgeError::Unsupported("non-Unix endpoint"));
        };
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o777 != 0o600
        {
            return Err(BridgeError::Authentication(
                "endpoint failed ownership checks".into(),
            ));
        }
        let inner = UnixStream::connect(path)?;
        let identity = peer_identity(&inner)?;
        if identity.user_id != unsafe { libc::geteuid() } {
            return Err(BridgeError::Authentication(
                "server belongs to another user".into(),
            ));
        }
        Ok(Self { inner })
    }

    pub fn try_clone(&self) -> BridgeResult<Self> {
        Ok(Self {
            inner: self.inner.try_clone()?,
        })
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> BridgeResult<()> {
        self.inner.set_read_timeout(timeout)?;
        Ok(())
    }

    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> BridgeResult<()> {
        self.inner.set_write_timeout(timeout)?;
        Ok(())
    }
}

impl Read for LocalStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl Write for LocalStream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_identity(stream: &UnixStream) -> BridgeResult<PeerIdentity> {
    let mut credential = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credential).cast(),
            &raw mut length,
        )
    };
    if result != 0 || length as usize != std::mem::size_of::<libc::ucred>() {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(PeerIdentity {
        user_id: credential.uid,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn peer_identity(stream: &UnixStream) -> BridgeResult<PeerIdentity> {
    let mut user = 0;
    let mut group = 0;
    let result = unsafe { libc::getpeereid(stream.as_raw_fd(), &raw mut user, &raw mut group) };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(PeerIdentity { user_id: user })
}

#[cfg(test)]
mod tests {
    use std::{process::Command, thread, time::Duration};

    use super::*;

    #[test]
    fn socket_is_private_and_peer_is_current_user() {
        let temporary = tempfile::tempdir().unwrap();
        fs::set_permissions(
            temporary.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let address = EndpointAddress::Unix {
            path: temporary.path().join("bridge.sock"),
        };
        let listener = LocalListener::bind(&address).unwrap();
        let thread = std::thread::spawn({
            let address = address.clone();
            move || {
                let mut stream = LocalStream::connect(&address).unwrap();
                stream.write_all(b"x").unwrap();
            }
        });
        let (mut stream, identity) = listener.accept().unwrap();
        assert_eq!(identity.user_id, unsafe { libc::geteuid() });
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        assert_eq!(byte, *b"x");
        thread.join().unwrap();
    }

    #[test]
    fn existing_endpoint_is_never_replaced() {
        let temporary = tempfile::tempdir().unwrap();
        fs::set_permissions(
            temporary.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let path = temporary.path().join("bridge.sock");
        fs::write(&path, b"do not replace").unwrap();
        let address = EndpointAddress::Unix { path };
        assert!(LocalListener::bind(&address).is_err());
    }

    #[test]
    fn killed_listener_residual_is_replaced_without_cleanup_assistance() {
        let temporary = tempfile::tempdir().unwrap();
        fs::set_permissions(
            temporary.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let socket = temporary.path().join("residual.sock");
        let ready = temporary.path().join("ready");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("crashed_listener_process_helper")
            .arg("--nocapture")
            .env("SPECTRUM_BRIDGE_CRASH_SOCKET", &socket)
            .env("SPECTRUM_BRIDGE_CRASH_READY", &ready)
            .spawn()
            .unwrap();
        for _ in 0..200 {
            if ready.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            ready.exists(),
            "crash helper did not publish its ready marker"
        );
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(socket.exists(), "SIGKILL unexpectedly cleaned the socket");

        let listener = LocalListener::bind(&EndpointAddress::Unix {
            path: socket.clone(),
        })
        .unwrap();
        assert_eq!(listener.path(), socket);
    }

    #[test]
    fn crashed_listener_process_helper() {
        let Ok(socket) = std::env::var("SPECTRUM_BRIDGE_CRASH_SOCKET") else {
            return;
        };
        let ready = std::env::var("SPECTRUM_BRIDGE_CRASH_READY").unwrap();
        let address = EndpointAddress::Unix {
            path: PathBuf::from(socket),
        };
        let listener = LocalListener::bind(&address).unwrap();
        fs::write(ready, b"ready").unwrap();
        std::hint::black_box(&listener);
        loop {
            thread::park();
        }
    }
}
