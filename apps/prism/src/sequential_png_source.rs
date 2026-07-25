use std::{
    error::Error,
    fmt,
    fs::{File, Metadata, OpenOptions},
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, bail};
use image::{Rgba, RgbaImage};
use serde::Serialize;
use sha2::{Digest, Sha256};
use spectrum_imaging::{
    ExactRegionSource, PixelRegion, RegionReadCapability, RegionReadiness, RegionRequestError,
    RegionSourceDescriptor, RegionSourceInfo, SourceSampleDepth, validate_region_request,
};

use crate::{RasterSourceEpoch, raster_region::decoder_contract_for};

const MAX_PNG_SCANLINE_BYTES: u64 = 64 * 1_024 * 1_024;

#[derive(Clone, Copy, Debug)]
pub struct SequentialPngLimits {
    pub max_encoded_source_bytes: u64,
    pub max_region_pixels: u64,
}

impl Default for SequentialPngLimits {
    fn default() -> Self {
        Self {
            max_encoded_source_bytes: 2 * 1_024 * 1_024 * 1_024,
            max_region_pixels: 4_096 * 4_096,
        }
    }
}

/// Immutable exact-region provider for non-interlaced PNG assets.
///
/// The provider retains the exact opened asset rather than its path. Every
/// decoder receives an independent logical cursor backed by positioned reads,
/// so concurrent compositor workers cannot share or race a file offset.
pub struct SequentialPngSource {
    info: RegionSourceInfo,
    file: Arc<File>,
    encoded_len: u64,
    fingerprint: Mutex<FileFingerprint>,
    source_sha256: String,
    source_epoch: RasterSourceEpoch,
    max_region_pixels: u64,
}

impl SequentialPngSource {
    pub fn open(path: &Path, limits: SequentialPngLimits) -> Result<Self> {
        let file = open_retained_file(path)
            .with_context(|| format!("could not open {}", path.display()))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            bail!("sequential PNG source is not a regular file");
        }
        if metadata.len() > limits.max_encoded_source_bytes {
            bail!("encoded PNG exceeds the sequential source byte limit");
        }
        let fingerprint = FileFingerprint::from_file(&file)?;
        let file = Arc::new(file);
        let first = inspect_retained_png(Arc::clone(&file), metadata.len())?;
        let source_sha256 = hash_retained_file(
            Arc::clone(&file),
            metadata.len(),
            limits.max_encoded_source_bytes,
        )?;
        let confirmed = inspect_retained_png(Arc::clone(&file), metadata.len())?;
        if first != confirmed || fingerprint != FileFingerprint::from_file(&file)? {
            bail!("PNG source changed while its immutable provider was prepared");
        }
        let source_epoch = epoch_for(&source_sha256, &first)?;
        Ok(Self {
            info: RegionSourceInfo {
                descriptor: first,
                capability: RegionReadCapability::SequentialBounded,
                readiness: RegionReadiness::Ready,
            },
            file,
            encoded_len: metadata.len(),
            fingerprint: Mutex::new(fingerprint),
            source_sha256,
            source_epoch,
            max_region_pixels: limits.max_region_pixels,
        })
    }

    pub fn source_epoch(&self) -> &RasterSourceEpoch {
        &self.source_epoch
    }

    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }

    fn begin_read(&self) -> Result<FileFingerprint, SequentialPngReadError> {
        let current =
            FileFingerprint::from_file(&self.file).map_err(SequentialPngReadError::Other)?;
        let mut fingerprint = self
            .fingerprint
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current == *fingerprint {
            return Ok(current);
        }

        let digest = hash_retained_file(Arc::clone(&self.file), self.encoded_len, self.encoded_len)
            .map_err(SequentialPngReadError::Other)?;
        let confirmed =
            FileFingerprint::from_file(&self.file).map_err(SequentialPngReadError::Other)?;
        if current != confirmed || digest != self.source_sha256 {
            return Err(SequentialPngReadError::Changed);
        }
        *fingerprint = confirmed.clone();
        Ok(confirmed)
    }

    fn finish_read(&self, started: &FileFingerprint) -> Result<(), SequentialPngReadError> {
        let current =
            FileFingerprint::from_file(&self.file).map_err(SequentialPngReadError::Other)?;
        if &current != started {
            return Err(SequentialPngReadError::Changed);
        }
        Ok(())
    }
}

fn open_retained_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};

        // Keep rename/deletion lifetime semantics, but reject a pre-existing
        // writer and prevent any new writer from opening the retained inode.
        options.share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE);
    }
    options.open(path)
}

impl ExactRegionSource for SequentialPngSource {
    type Error = SequentialPngReadError;

    fn info(&self) -> &RegionSourceInfo {
        &self.info
    }

    fn read_exact_region(&self, region: PixelRegion) -> Result<RgbaImage, Self::Error> {
        validate_region_request(&self.info.descriptor, region, self.max_region_pixels)
            .map_err(SequentialPngReadError::Request)?;
        let read_fingerprint = self.begin_read()?;
        let mut reader = png_reader(Arc::clone(&self.file), self.encoded_len)
            .map_err(SequentialPngReadError::Other)?;
        if descriptor_from_reader(&reader).map_err(SequentialPngReadError::Other)?
            != self.info.descriptor
        {
            return Err(SequentialPngReadError::Changed);
        }
        let (color_type, bit_depth) = reader.output_color_type();
        if bit_depth != png::BitDepth::Eight {
            return Err(SequentialPngReadError::Unsupported(
                "PNG provider requires 8-bit transformed rows",
            ));
        }
        let channels = png_channels(color_type);
        let row_bytes = u64::from(self.info.descriptor.width)
            .checked_mul(channels as u64)
            .ok_or(SequentialPngReadError::LayoutOverflow)?;
        if row_bytes > MAX_PNG_SCANLINE_BYTES {
            return Err(SequentialPngReadError::Unsupported(
                "PNG scanline exceeds the bounded source staging budget",
            ));
        }
        let bottom = region
            .y
            .checked_add(region.height)
            .ok_or(SequentialPngReadError::LayoutOverflow)?;
        let right = region
            .x
            .checked_add(region.width)
            .ok_or(SequentialPngReadError::LayoutOverflow)?;
        let mut output = RgbaImage::new(region.width, region.height);
        for source_y in 0..bottom {
            let row = reader
                .next_row()
                .map_err(SequentialPngReadError::Decode)?
                .ok_or(SequentialPngReadError::Truncated)?;
            if source_y < region.y {
                continue;
            }
            for source_x in region.x..right {
                let offset = usize::try_from(source_x)
                    .ok()
                    .and_then(|x| x.checked_mul(channels))
                    .ok_or(SequentialPngReadError::LayoutOverflow)?;
                let end = offset
                    .checked_add(channels)
                    .ok_or(SequentialPngReadError::LayoutOverflow)?;
                let bytes = row
                    .data()
                    .get(offset..end)
                    .ok_or(SequentialPngReadError::Truncated)?;
                output.put_pixel(
                    source_x - region.x,
                    source_y - region.y,
                    Rgba(png_pixel(bytes, color_type)),
                );
            }
        }
        self.finish_read(&read_fingerprint)?;
        Ok(output)
    }
}

#[derive(Debug)]
pub enum SequentialPngReadError {
    Request(RegionRequestError),
    Io(io::Error),
    Decode(png::DecodingError),
    Other(anyhow::Error),
    Changed,
    Truncated,
    LayoutOverflow,
    Unsupported(&'static str),
}

impl fmt::Display for SequentialPngReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(error) => error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
            Self::Decode(error) => error.fmt(formatter),
            Self::Other(error) => error.fmt(formatter),
            Self::Changed => formatter.write_str("retained PNG source changed after publication"),
            Self::Truncated => {
                formatter.write_str("retained PNG ended before the requested region")
            }
            Self::LayoutOverflow => formatter.write_str("PNG region layout overflows"),
            Self::Unsupported(message) => formatter.write_str(message),
        }
    }
}

impl Error for SequentialPngReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Request(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Decode(error) => Some(error),
            Self::Other(error) => Some(error.as_ref()),
            Self::Changed | Self::Truncated | Self::LayoutOverflow | Self::Unsupported(_) => None,
        }
    }
}

impl From<io::Error> for SequentialPngReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Serialize)]
struct EpochMaterial<'a> {
    source_sha256: &'a str,
    descriptor: &'a RegionSourceDescriptor,
}

fn epoch_for(
    source_sha256: &str,
    descriptor: &RegionSourceDescriptor,
) -> Result<RasterSourceEpoch> {
    let encoded = serde_json::to_vec(&EpochMaterial {
        source_sha256,
        descriptor,
    })?;
    let digest = Sha256::digest(encoded);
    RasterSourceEpoch::new(format!("sequential-png:{}", hex_digest(&digest)))
}

fn inspect_retained_png(file: Arc<File>, encoded_len: u64) -> Result<RegionSourceDescriptor> {
    let reader = png_reader(file, encoded_len)?;
    descriptor_from_reader(&reader)
}

type RetainedPngReader = png::Reader<BufReader<PositionedFileReader>>;

fn png_reader(file: Arc<File>, encoded_len: u64) -> Result<RetainedPngReader> {
    let reader = BufReader::new(PositionedFileReader::new(file, encoded_len));
    let mut decoder = png::Decoder::new_with_limits(
        reader,
        png::Limits {
            bytes: MAX_PNG_SCANLINE_BYTES as usize,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND);
    decoder
        .read_info()
        .context("could not decode retained PNG header")
}

fn descriptor_from_reader(reader: &RetainedPngReader) -> Result<RegionSourceDescriptor> {
    if reader.info().interlaced {
        bail!("interlaced PNG requires a derived backing");
    }
    let sample_depth = match reader.info().bit_depth {
        png::BitDepth::Eight => SourceSampleDepth::EightBit,
        png::BitDepth::Sixteen => bail!("16-bit PNG requires full-decode handling"),
        png::BitDepth::One => SourceSampleDepth::Other(1),
        png::BitDepth::Two => SourceSampleDepth::Other(2),
        png::BitDepth::Four => SourceSampleDepth::Other(4),
    };
    let (color_type, output_depth) = reader.output_color_type();
    if output_depth != png::BitDepth::Eight {
        bail!("PNG transformations did not produce exact 8-bit rows");
    }
    Ok(RegionSourceDescriptor {
        width: reader.info().width,
        height: reader.info().height,
        color_encoding: png_color_encoding(color_type, output_depth),
        sample_depth,
        frame_index: 0,
        page_index: 0,
        decoder_contract: decoder_contract_for(image::ImageFormat::Png),
    })
}

fn hash_retained_file(file: Arc<File>, encoded_len: u64, max_bytes: u64) -> Result<String> {
    if encoded_len > max_bytes {
        bail!("encoded PNG exceeds the sequential source byte limit");
    }
    let mut reader = PositionedFileReader::new(file, encoded_len);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1_024];
    let mut bytes = 0_u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes = bytes
            .checked_add(read as u64)
            .context("encoded PNG byte count overflows")?;
        if bytes > max_bytes {
            bail!("encoded PNG exceeds the sequential source byte limit");
        }
        digest.update(&buffer[..read]);
    }
    if bytes != encoded_len {
        bail!("retained PNG length changed while hashing");
    }
    Ok(hex_digest(&digest.finalize()))
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

struct PositionedFileReader {
    file: Arc<File>,
    cursor: u64,
    len: u64,
}

impl PositionedFileReader {
    fn new(file: Arc<File>, len: u64) -> Self {
        Self {
            file,
            cursor: 0,
            len,
        }
    }
}

impl Read for PositionedFileReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = read_at(&self.file, buffer, self.cursor)?;
        self.cursor = self
            .cursor
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::other("positioned PNG cursor overflows"))?;
        Ok(read)
    }
}

impl Seek for PositionedFileReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let position = match position {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::Current(delta) => i128::from(self.cursor) + i128::from(delta),
            SeekFrom::End(delta) => i128::from(self.len) + i128::from(delta),
        };
        self.cursor = u64::try_from(position)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid PNG seek"))?;
        Ok(self.cursor)
    }
}

#[cfg(unix)]
fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buffer, offset)
}

#[cfg(windows)]
fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buffer, offset)
}

#[cfg(not(any(unix, windows)))]
fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::sync::{Mutex, OnceLock};

    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut file = file;
    file.seek(SeekFrom::Start(offset))?;
    file.read(buffer)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileFingerprint {
    len: u64,
    modified: Option<std::time::SystemTime>,
    platform: PlatformFingerprint,
}

impl FileFingerprint {
    fn from_file(file: &File) -> Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            platform: PlatformFingerprint::from_file(file, &metadata)?,
        })
    }
}

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct PlatformFingerprint {
    device: u64,
    inode: u64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[cfg(unix)]
impl PlatformFingerprint {
    fn from_file(_file: &File, metadata: &Metadata) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;

        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        })
    }
}

#[cfg(windows)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct PlatformFingerprint {
    volume: u64,
    file_id: [u8; 16],
    last_write: i64,
    change: i64,
}

#[cfg(windows)]
impl PlatformFingerprint {
    fn from_file(file: &File, _metadata: &Metadata) -> Result<Self> {
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_BASIC_INFO, FILE_ID_INFO, FileBasicInfo, FileIdInfo,
        };

        fn query<T: Default>(file: &File, class: i32) -> io::Result<T> {
            use std::{mem::size_of, os::windows::io::AsRawHandle};
            use windows_sys::Win32::{
                Foundation::HANDLE, Storage::FileSystem::GetFileInformationByHandleEx,
            };

            let mut value = T::default();
            // SAFETY: `file` owns a valid handle for the duration of this call,
            // and `value` is writable storage whose exact size is supplied.
            let succeeded = unsafe {
                GetFileInformationByHandleEx(
                    file.as_raw_handle() as HANDLE,
                    class,
                    (&raw mut value).cast(),
                    size_of::<T>() as u32,
                )
            };
            if succeeded == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(value)
        }

        let basic: FILE_BASIC_INFO = query(file, FileBasicInfo)?;
        let identity: FILE_ID_INFO = query(file, FileIdInfo)?;
        Ok(Self {
            volume: identity.VolumeSerialNumber,
            file_id: identity.FileId.Identifier,
            last_write: basic.LastWriteTime,
            change: basic.ChangeTime,
        })
    }
}

#[cfg(not(any(unix, windows)))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct PlatformFingerprint;

#[cfg(not(any(unix, windows)))]
impl PlatformFingerprint {
    fn from_file(_file: &File, _metadata: &Metadata) -> Result<Self> {
        Ok(Self)
    }
}

fn png_channels(color_type: png::ColorType) -> usize {
    match color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => unreachable!("EXPAND removes indexed PNG output"),
    }
}

fn png_color_encoding(color: png::ColorType, depth: png::BitDepth) -> String {
    let channels = match color {
        png::ColorType::Grayscale => "l",
        png::ColorType::GrayscaleAlpha => "la",
        png::ColorType::Rgb => "rgb",
        png::ColorType::Rgba => "rgba",
        png::ColorType::Indexed => unreachable!("EXPAND removes indexed PNG output"),
    };
    let bits = match depth {
        png::BitDepth::Eight => 8,
        png::BitDepth::Sixteen => 16,
        png::BitDepth::One | png::BitDepth::Two | png::BitDepth::Four => {
            unreachable!("EXPAND promotes sub-byte PNG output")
        }
    };
    format!("{channels}{bits}")
}

fn png_pixel(bytes: &[u8], color_type: png::ColorType) -> [u8; 4] {
    match color_type {
        png::ColorType::Grayscale => [bytes[0], bytes[0], bytes[0], 255],
        png::ColorType::GrayscaleAlpha => [bytes[0], bytes[0], bytes[0], bytes[1]],
        png::ColorType::Rgb => [bytes[0], bytes[1], bytes[2], 255],
        png::ColorType::Rgba => [bytes[0], bytes[1], bytes[2], bytes[3]],
        png::ColorType::Indexed => unreachable!("EXPAND removes indexed PNG output"),
    }
}

#[cfg(test)]
#[path = "sequential_png_source_tests.rs"]
mod tests;
