use std::{
    fs::{self, File, OpenOptions},
    io::{BufReader, Write},
    path::{Path, PathBuf},
    sync::{Condvar, Mutex, OnceLock},
};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, GenericImageView, ImageDecoder};
use spectrum_imaging::RegionSourceDescriptor;

use super::{
    DerivedBackingIdentity, DerivedBackingLimits, DerivedBackingMemoryPlan, decoder_contract_for,
};

/// Full-raster decodes resident at once anywhere in this process.
const DECODE_PERMITS: usize = 3;

struct DecodePermits {
    available: Mutex<usize>,
    freed: Condvar,
}

static FULL_RASTER_DECODE: OnceLock<DecodePermits> = OnceLock::new();

/// Leave to hold `count` decoded full rasters; given back when dropped.
pub(super) struct DecodePermit(usize);

impl Drop for DecodePermit {
    fn drop(&mut self) {
        let permits = decode_permits();
        *permits
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += self.0;
        permits.freed.notify_all();
    }
}

fn decode_permits() -> &'static DecodePermits {
    FULL_RASTER_DECODE.get_or_init(|| DecodePermits {
        available: Mutex::new(DECODE_PERMITS),
        freed: Condvar::new(),
    })
}

/// A source decoded and checked against its identity, still holding its
/// decode permit until its plane is written.
pub(super) struct DecodedSource {
    decoded: DynamicImage,
    plan: DerivedBackingMemoryPlan,
    _permit: DecodePermit,
}
const TIFF_COMPRESSED_SEGMENT_OVERHEAD_BYTES: u64 = 64 * 1_024;

pub(super) struct PreparedPlane {
    pub plane_bytes: u64,
    pub plane_digest: String,
    pub memory_plan: DerivedBackingMemoryPlan,
}

pub(super) fn memory_plan(
    descriptor: &RegionSourceDescriptor,
    limits: DerivedBackingLimits,
) -> Result<DerivedBackingMemoryPlan> {
    if descriptor.width == 0 || descriptor.height == 0 {
        bail!("derived raster backing dimensions must be nonzero");
    }
    let pixels = u64::from(descriptor.width)
        .checked_mul(u64::from(descriptor.height))
        .context("derived raster backing dimensions overflow")?;
    let channels = match descriptor.color_encoding.as_str() {
        "l8" => 1,
        "la8" => 2,
        "rgb8" => 3,
        "rgba8" => 4,
        _ => bail!("derived raster backing has an unsupported decoded pixel layout"),
    };
    let decoded_surface_bytes = pixels
        .checked_mul(channels)
        .context("decoded raster surface size overflows")?;
    let conversion_row_bytes = if channels == 4 {
        0
    } else {
        u64::from(descriptor.width)
            .checked_mul(4)
            .context("derived raster conversion row size overflows")?
    };
    // image-webp 0.2.4 lossless opaque decode allocates a full RGBA scratch
    // plane while image's caller-provided RGB output plane is live. Reserve an
    // RGBA plane for every WebP layout; other dependency-private buffers remain
    // explicitly outside an enforced bound.
    let decoder_scratch_reservation_bytes = if descriptor.decoder_contract.ends_with(":webp") {
        pixels
            .checked_mul(4)
            .context("WebP decoder scratch reservation overflows")?
    } else if descriptor.decoder_contract.ends_with(":tif")
        || descriptor.decoder_contract.ends_with(":tiff")
    {
        // image-tiff retains a complete decoded buffer beside the caller's
        // output. CMYK8 requires four bytes per source pixel before RGB
        // conversion. Its intermediate buffer must also admit an encoded
        // strip: incompressible LZW can approach 1.5 times the raw bytes, while
        // Deflate and PackBits add smaller bounded overhead. Reserve that
        // conservative segment envelope plus a fixed codec/header allowance,
        // and pass the complete reservation to ImageDecoder::set_limits below.
        let decoded_buffer = pixels
            .checked_mul(4)
            .context("TIFF decoded buffer reservation overflows")?;
        let compressed_segment = decoded_buffer
            .checked_add(decoded_buffer.div_ceil(2))
            .and_then(|bytes| bytes.checked_add(TIFF_COMPRESSED_SEGMENT_OVERHEAD_BYTES))
            .context("TIFF compressed segment reservation overflows")?;
        decoded_buffer
            .checked_add(compressed_segment)
            .context("TIFF decoder reservation overflows")?
    } else {
        0
    };
    // image 0.25.10's JPEG decoder constructor reads the entire encoded source
    // into a Vec. Identity preparation already enforces this configured limit.
    let encoded_input_reservation_bytes = if descriptor.decoder_contract.ends_with(":jpg")
        || descriptor.decoder_contract.ends_with(":jpeg")
    {
        limits.max_encoded_source_bytes
    } else {
        0
    };
    let decode_reservation = decoded_surface_bytes
        .checked_add(decoder_scratch_reservation_bytes)
        .and_then(|bytes| bytes.checked_add(encoded_input_reservation_bytes))
        .context("known decoder memory reservation overflows")?;
    let publication_reservation = decoded_surface_bytes
        .checked_add(conversion_row_bytes)
        .context("derived raster publication memory reservation overflows")?;
    Ok(DerivedBackingMemoryPlan {
        decoded_surface_bytes,
        conversion_row_bytes,
        decoder_scratch_reservation_bytes,
        encoded_input_reservation_bytes,
        known_resident_reservation_bytes: decode_reservation.max(publication_reservation),
    })
}

/// Decoder construction, decode, and publication share a process-wide permit
/// so at most [`DECODE_PERMITS`] dependency full-raster workloads are
/// resident anywhere in Spectrum.
pub(super) fn prepare_exact_rgba8_plane(
    source: &Path,
    identity: &DerivedBackingIdentity,
    limits: DerivedBackingLimits,
    plane_path: &Path,
) -> Result<PreparedPlane> {
    write_plane(
        decode_source(source, identity, limits)?,
        identity,
        plane_path,
    )
}

/// Writes a decoded source's plane, then gives back its decode permit.
pub(super) fn write_plane(
    decoded: DecodedSource,
    identity: &DerivedBackingIdentity,
    plane_path: &Path,
) -> Result<PreparedPlane> {
    let DecodedSource {
        decoded,
        plan,
        _permit,
    } = decoded;
    write_decoded_surface(decoded, identity, plane_path, plan)
}

/// Decodes a source, checking it against its identity before and after.
pub(super) fn decode_source(
    source: &Path,
    identity: &DerivedBackingIdentity,
    limits: DerivedBackingLimits,
) -> Result<DecodedSource> {
    let mut source_file =
        File::open(source).with_context(|| format!("could not open {}", source.display()))?;
    if !source_file.metadata()?.is_file() {
        bail!("encoded raster source is not a regular file");
    }
    // The source was hashed into `identity` just before; its stamp shows it
    // is still that file, before and after its decode.
    let before = super::file_stamp(&source_file.metadata()?);
    if identity.stamp.is_some_and(|stamp| stamp != before) {
        bail!("raster source changed before its derived backing was prepared");
    }
    if source_file.metadata()?.len() > limits.max_encoded_source_bytes {
        bail!("encoded raster exceeds the derived backing source byte limit");
    }

    let reader = image::ImageReader::new(BufReader::new(&mut source_file))
        .with_guessed_format()
        .with_context(|| format!("could not identify {}", source.display()))?;
    let format = reader
        .format()
        .with_context(|| format!("could not identify {}", source.display()))?;
    if decoder_contract_for(format) != identity.descriptor.decoder_contract {
        bail!("raster decoder contract changed before backing preparation");
    }
    let expected_plan = memory_plan(&identity.descriptor, limits)?;
    let mut image_limits = image::Limits::default();
    image_limits.max_image_width = Some(identity.descriptor.width);
    image_limits.max_image_height = Some(identity.descriptor.height);
    image_limits.max_alloc = Some(
        if identity.descriptor.decoder_contract.ends_with(":tif")
            || identity.descriptor.decoder_contract.ends_with(":tiff")
        {
            expected_plan.decoder_scratch_reservation_bytes
        } else {
            limits.max_plane_bytes
        },
    );
    // Decoder construction is inside the process-wide permit: image 0.25.10's
    // JPEG constructor buffers the complete encoded stream before read_image.
    let permit = acquire_full_raster_decode_permit(1);
    let mut decoder = reader
        .into_decoder()
        .with_context(|| format!("could not inspect {}", source.display()))?;
    if decoder.dimensions() != (identity.descriptor.width, identity.descriptor.height)
        || format!("{:?}", decoder.color_type()).to_ascii_lowercase()
            != identity.descriptor.color_encoding
    {
        bail!("raster decoder metadata changed before backing preparation");
    }
    decoder.set_limits(image_limits)?;
    let decoded = DynamicImage::from_decoder(decoder)
        .with_context(|| format!("could not decode {}", source.display()))?;

    // A file changed while it was decoded is rejected before any cache
    // entry can be published.
    if super::file_stamp(&fs::metadata(source)?) != before {
        bail!("raster source changed while its derived backing was prepared");
    }
    if decoded.dimensions() != (identity.descriptor.width, identity.descriptor.height)
        || format!("{:?}", decoded.color()).to_ascii_lowercase()
            != identity.descriptor.color_encoding
    {
        bail!("decoded raster changed its backing pixel layout");
    }

    Ok(DecodedSource {
        decoded,
        plan: expected_plan,
        _permit: permit,
    })
}

fn write_decoded_surface(
    decoded: DynamicImage,
    identity: &DerivedBackingIdentity,
    plane_path: &Path,
    memory_plan: DerivedBackingMemoryPlan,
) -> Result<PreparedPlane> {
    let expected_bytes = identity
        .descriptor
        .exact_rgba8_plane_bytes()
        .context("derived raster backing dimensions overflow")?;
    let row_pixels = usize::try_from(identity.descriptor.width)
        .context("derived raster row width does not fit memory")?;
    let mut sink = PlaneSink::create(plane_path)?;
    match decoded {
        DynamicImage::ImageRgba8(image) => {
            let row_bytes = row_pixels
                .checked_mul(4)
                .context("derived raster row size overflows")?;
            let raw = image.into_raw();
            for row in raw.chunks_exact(row_bytes) {
                sink.write_row(row)?;
            }
        }
        DynamicImage::ImageRgb8(image) => {
            let raw = image.into_raw();
            write_converted_rows(&mut sink, &raw, row_pixels, 3, |pixel, rgba| {
                rgba[..3].copy_from_slice(pixel);
                rgba[3] = u8::MAX;
            })?;
        }
        DynamicImage::ImageLumaA8(image) => {
            let raw = image.into_raw();
            write_converted_rows(&mut sink, &raw, row_pixels, 2, |pixel, rgba| {
                rgba[..3].fill(pixel[0]);
                rgba[3] = pixel[1];
            })?;
        }
        DynamicImage::ImageLuma8(image) => {
            let raw = image.into_raw();
            write_converted_rows(&mut sink, &raw, row_pixels, 1, |pixel, rgba| {
                rgba[..3].fill(pixel[0]);
                rgba[3] = u8::MAX;
            })?;
        }
        _ => bail!("decoder produced an unsupported dynamic pixel layout"),
    }
    let (plane_bytes, plane_digest) = sink.finish()?;
    if plane_bytes != expected_bytes {
        bail!("decoded raster byte count does not match its backing descriptor");
    }
    Ok(PreparedPlane {
        plane_bytes,
        plane_digest,
        memory_plan,
    })
}

fn write_converted_rows(
    sink: &mut PlaneSink,
    source: &[u8],
    row_pixels: usize,
    channels: usize,
    mut convert: impl FnMut(&[u8], &mut [u8]),
) -> Result<()> {
    let source_row_bytes = row_pixels
        .checked_mul(channels)
        .context("decoded raster row size overflows")?;
    let output_row_bytes = row_pixels
        .checked_mul(4)
        .context("derived raster row size overflows")?;
    let mut output = vec![0_u8; output_row_bytes];
    for source_row in source.chunks_exact(source_row_bytes) {
        for (pixel, rgba) in source_row
            .chunks_exact(channels)
            .zip(output.as_chunks_mut::<4>().0.iter_mut())
        {
            convert(pixel, rgba);
        }
        sink.write_row(&output)?;
    }
    Ok(())
}

struct PlaneSink {
    file: std::io::BufWriter<File>,
    path: PathBuf,
    bytes: u64,
}

impl PlaneSink {
    fn create(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path)?;
        Ok(Self {
            file: std::io::BufWriter::with_capacity(1 << 20, file),
            path: path.to_owned(),
            bytes: 0,
        })
    }

    fn write_row(&mut self, row: &[u8]) -> Result<()> {
        self.file.write_all(row)?;
        self.bytes = self
            .bytes
            .checked_add(row.len() as u64)
            .context("derived raster plane byte count overflows")?;
        Ok(())
    }

    /// The plane's length and digest. Not flushed: see `write_immutable_file`.
    fn finish(self) -> Result<(u64, String)> {
        let file = self.file.into_inner().map_err(|error| error.into_error())?;
        let digest = super::plane_digest(&file, self.bytes)?;
        let mut permissions = file.metadata()?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&self.path, permissions)?;
        Ok((self.bytes, digest))
    }
}

fn acquire_full_raster_decode_permit(count: usize) -> DecodePermit {
    let permits = decode_permits();
    let mut available = permits
        .available
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    while *available < count {
        available = permits
            .freed
            .wait(available)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
    *available -= count;
    DecodePermit(count)
}

/// Runs `operation` holding every decode permit, so no decode can start.
#[cfg(test)]
pub(crate) fn with_full_raster_decode_permit_for_test<T>(operation: impl FnOnce() -> T) -> T {
    let _permit = acquire_full_raster_decode_permit(DECODE_PERMITS);
    operation()
}
