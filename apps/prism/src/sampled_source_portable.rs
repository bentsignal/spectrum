use std::{fs, path::Path};

use anyhow::{Context, Result};

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
    if let Ok(relative) = canonical.strip_prefix(directory) {
        source.path = relative.to_owned();
        return Ok(());
    }
    fs::create_dir_all(asset_directory)?;
    let extension = canonical
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let destination =
        asset_directory.join(format!("sampled-{}.{}", source.content_hash, extension));
    if !destination.exists() {
        fs::copy(&canonical, &destination).with_context(|| {
            format!(
                "could not copy {} into portable Prism assets",
                canonical.display()
            )
        })?;
    }
    source.path = destination.strip_prefix(directory)?.to_owned();
    Ok(())
}
