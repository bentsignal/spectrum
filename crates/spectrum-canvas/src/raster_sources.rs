use std::{fmt, path::Path, sync::Arc};

use anyhow::{Result, bail};
use spectrum_imaging::DynExactRegionSource;

/// Immutable identity of the exact base pixels returned by a raster provider.
///
/// Content-addressed providers should use their content/decoder-contract key.
/// Layer transforms and adjustments deliberately do not belong in this epoch.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RasterSourceEpoch(Arc<str>);

impl RasterSourceEpoch {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() {
            bail!("raster source epoch must not be empty");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

/// One ready provider resolved from an immutable resolver snapshot.
#[derive(Clone)]
pub struct ResolvedRasterSource {
    source_epoch: RasterSourceEpoch,
    content_sha256: Option<Arc<str>>,
    source: Arc<dyn DynExactRegionSource>,
}

impl ResolvedRasterSource {
    pub fn new(
        source_epoch: RasterSourceEpoch,
        source: Arc<dyn DynExactRegionSource>,
    ) -> Result<Self> {
        let info = source.info();
        if !info.supports_region_reads_now() {
            bail!("resolved raster source is not ready for exact region reads");
        }
        if info.descriptor.width == 0 || info.descriptor.height == 0 {
            bail!("resolved raster source dimensions must be positive");
        }
        Ok(Self {
            source_epoch,
            content_sha256: None,
            source,
        })
    }

    /// Builds a provider authenticated to the exact encoded source bytes.
    pub fn new_authenticated(
        source_epoch: RasterSourceEpoch,
        content_sha256: impl Into<Arc<str>>,
        source: Arc<dyn DynExactRegionSource>,
    ) -> Result<Self> {
        let content_sha256 = content_sha256.into();
        if content_sha256.len() != 64
            || !content_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            bail!("raster source content identity must be lowercase SHA-256");
        }
        let mut resolved = Self::new(source_epoch, source)?;
        resolved.content_sha256 = Some(content_sha256);
        Ok(resolved)
    }

    pub fn source_epoch(&self) -> &RasterSourceEpoch {
        &self.source_epoch
    }

    pub fn source(&self) -> &(dyn DynExactRegionSource + 'static) {
        self.source.as_ref()
    }

    pub fn content_sha256(&self) -> Option<&str> {
        self.content_sha256.as_deref()
    }
}

impl fmt::Debug for ResolvedRasterSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedRasterSource")
            .field("source_epoch", &self.source_epoch)
            .field("content_sha256", &self.content_sha256)
            .field("info", self.source.info())
            .finish_non_exhaustive()
    }
}

/// Memory-only view of all exact raster providers available to one render.
///
/// Implementations must be immutable: resolving the same path repeatedly from
/// one snapshot must return the same source epoch and provider. Applications
/// publish a replacement snapshot, with a new `snapshot_epoch`, when readiness
/// or exact source pixels change. `resolve` must never inspect or open `path`.
pub trait RasterSourceResolver: Send + Sync {
    fn snapshot_epoch(&self) -> u64;
    fn resolve(&self, path: &Path) -> Option<ResolvedRasterSource>;
}
