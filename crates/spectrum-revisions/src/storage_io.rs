use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{RevisionError, RevisionResult};

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

pub(crate) fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value: OsString = path.as_os_str().to_owned();
    value.push(suffix);
    PathBuf::from(value)
}

pub(crate) fn sync_if_present(path: &Path) -> RevisionResult<()> {
    match fs::OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file.sync_all().map_err(Into::into),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RevisionError::Io(error)),
    }
}
