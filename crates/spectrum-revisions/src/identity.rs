use std::{fs, path::Path};

use fs2::FileExt;

use crate::{RevisionError, RevisionResult, SessionId};

/// Returns the stable local session identity shared by Spectrum applications.
pub fn local_session_id(directory: &Path) -> RevisionResult<SessionId> {
    fs::create_dir_all(directory)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("local-session.lock"))?;
    lock.lock_exclusive()?;
    let path = directory.join("local-session-id");
    match fs::read(&path) {
        Ok(bytes) => {
            let bytes: [u8; 16] = bytes.try_into().map_err(|_| {
                RevisionError::Corrupt("local session identity has the wrong length".into())
            })?;
            Ok(SessionId::from_bytes(bytes))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let session = SessionId::new();
            let temporary = directory.join(format!(".local-session-{session}.tmp"));
            let written = (|| -> std::io::Result<()> {
                fs::write(&temporary, session.as_bytes())?;
                fs::File::open(&temporary)?.sync_all()?;
                fs::rename(&temporary, path)
            })();
            if let Err(error) = written {
                let _ = fs::remove_file(temporary);
                return Err(error.into());
            }
            Ok(session)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_survives_restarts() {
        let directory = tempfile::tempdir().unwrap();
        let first = local_session_id(directory.path()).unwrap();
        let second = local_session_id(directory.path()).unwrap();
        assert_eq!(second, first);
        assert_eq!(
            fs::read(directory.path().join("local-session-id"))
                .unwrap()
                .len(),
            16
        );
    }
}
