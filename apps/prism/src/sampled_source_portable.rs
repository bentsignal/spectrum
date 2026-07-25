use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::SampledSourceSnapshot;

pub(crate) fn make_portable(
    source: &mut SampledSourceSnapshot,
    directory: &Path,
    asset_directory: &Path,
) -> Result<()> {
    source.validate_asset()?;
    let canonical = fs::canonicalize(&source.path).with_context(|| {
        format!(
            "could not read Clone Stamp source {}",
            source.path.display()
        )
    })?;
    fs::create_dir_all(asset_directory)?;
    let extension = canonical
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let destination =
        asset_directory.join(format!("sampled-{}.{}", source.content_hash, extension));
    let (_, bytes) = crate::font_source::read_secure_regular_file(
        &canonical,
        crate::revisions::MAX_EMBEDDED_RASTER_BYTES,
        "Clone Stamp raster source",
    )?;
    if sha256_hex(&bytes) != source.content_hash {
        bail!("Clone Stamp source changed before portable publication");
    }
    publish_no_replace(&destination, &bytes, &source.content_hash)?;
    source.path = destination.strip_prefix(directory)?.to_owned();
    Ok(())
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

fn publish_no_replace(destination: &Path, bytes: &[u8], expected_hash: &str) -> Result<()> {
    if destination.symlink_metadata().is_ok() {
        return validate_existing(destination, expected_hash);
    }
    let parent = destination
        .parent()
        .context("portable Clone Stamp destination has no parent")?;
    let name = destination
        .file_name()
        .context("portable Clone Stamp destination has no file name")?
        .to_string_lossy();
    let temporary = parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .context("could not create private Clone Stamp publication temporary")?;
        output.write_all(bytes)?;
        output.sync_all()?;
        drop(output);
        match fs::hard_link(&temporary, destination) {
            Ok(()) => {
                validate_existing(destination, expected_hash)?;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                validate_existing(destination, expected_hash)
            }
            Err(error) => Err(error).context("could not atomically publish Clone Stamp asset"),
        }
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn validate_existing(path: &Path, expected_hash: &str) -> Result<()> {
    let (_, bytes) = crate::font_source::read_secure_regular_file(
        path,
        crate::revisions::MAX_EMBEDDED_RASTER_BYTES,
        "portable Clone Stamp asset",
    )?;
    if sha256_hex(&bytes) != expected_hash {
        bail!("portable Clone Stamp destination already exists with different bytes");
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}
