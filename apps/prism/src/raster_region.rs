use std::{fs::File, io::BufReader, path::Path};

use anyhow::{Context, Result};
use image::{ColorType, ImageDecoder, ImageFormat};
use spectrum_imaging::{
    RegionReadCapability, RegionReadiness, RegionSourceDescriptor, RegionSourceInfo,
    SourceSampleDepth,
};

const DECODER_CONTRACT_REVISION: &str = "spectrum-prism:image-0.25.10:rgba8:v1";
const PNG_HEADER_LIMIT_BYTES: usize = 64 * 1_024 * 1_024;

#[derive(Clone, Debug)]
pub struct RasterRegionInspection {
    pub info: RegionSourceInfo,
    pub format: ImageFormat,
}

/// Inspects one raster without decoding pixels.
///
/// PNG reuses the signature reader for one bounded PNG header parse. The GUI's
/// future source registry should cache this result by immutable content identity
/// or linked-file generation rather than calling it on every paint frame.
pub fn inspect_raster_region_source(path: &Path) -> Result<RasterRegionInspection> {
    let reader = image::ImageReader::open(path)
        .with_context(|| format!("could not open {}", path.display()))?
        .with_guessed_format()
        .with_context(|| format!("could not identify {}", path.display()))?;
    let format = reader
        .format()
        .with_context(|| format!("could not identify {}", path.display()))?;
    if format == ImageFormat::Png {
        return inspect_png(reader.into_inner(), format, path);
    }
    let decoder = reader
        .into_decoder()
        .with_context(|| format!("could not inspect {}", path.display()))?;
    let (width, height) = decoder.dimensions();
    let color = decoder.color_type();
    let sample_depth = image_sample_depth(color);
    let capability = match format {
        ImageFormat::Jpeg | ImageFormat::WebP if sample_depth == SourceSampleDepth::EightBit => {
            RegionReadCapability::DerivedBacking
        }
        // TIFF is FullDecodeOnly until a provider inspects and proves its
        // concrete strip/tile organization is independently seekable.
        ImageFormat::Tiff => RegionReadCapability::FullDecodeOnly,
        _ => RegionReadCapability::FullDecodeOnly,
    };
    let readiness = match capability {
        RegionReadCapability::SequentialBounded => RegionReadiness::Ready,
        RegionReadCapability::DerivedBacking => RegionReadiness::NeedsPreparation,
        RegionReadCapability::SeekableChunks | RegionReadCapability::FullDecodeOnly => {
            RegionReadiness::Unsupported
        }
    };
    Ok(RasterRegionInspection {
        info: RegionSourceInfo {
            descriptor: RegionSourceDescriptor {
                width,
                height,
                color_encoding: format!("{color:?}").to_ascii_lowercase(),
                sample_depth,
                frame_index: 0,
                page_index: 0,
                decoder_contract: decoder_contract_for(format),
            },
            capability,
            readiness,
        },
        format,
    })
}

pub(crate) fn decoder_contract_for(format: ImageFormat) -> String {
    format!(
        "{DECODER_CONTRACT_REVISION}:{}",
        format
            .extensions_str()
            .first()
            .copied()
            .unwrap_or("unknown")
    )
}

fn image_sample_depth(color: ColorType) -> SourceSampleDepth {
    match color {
        ColorType::L8 | ColorType::La8 | ColorType::Rgb8 | ColorType::Rgba8 => {
            SourceSampleDepth::EightBit
        }
        ColorType::L16 | ColorType::La16 | ColorType::Rgb16 | ColorType::Rgba16 => {
            SourceSampleDepth::SixteenBit
        }
        ColorType::Rgb32F | ColorType::Rgba32F => SourceSampleDepth::Float32,
        _ => SourceSampleDepth::Other(u8::try_from(color.bits_per_pixel()).unwrap_or(u8::MAX)),
    }
}

struct PngInspection {
    width: u32,
    height: u32,
    color_encoding: String,
    interlaced: bool,
    sample_depth: SourceSampleDepth,
}

fn inspect_png(
    file: BufReader<File>,
    format: ImageFormat,
    path: &Path,
) -> Result<RasterRegionInspection> {
    let mut decoder = png::Decoder::new_with_limits(
        file,
        png::Limits {
            bytes: PNG_HEADER_LIMIT_BYTES,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND);
    let reader = decoder
        .read_info()
        .with_context(|| format!("could not inspect PNG {}", path.display()))?;
    let (output_color, output_depth) = reader.output_color_type();
    let sample_depth = match reader.info().bit_depth {
        png::BitDepth::Eight => SourceSampleDepth::EightBit,
        png::BitDepth::Sixteen => SourceSampleDepth::SixteenBit,
        depth => SourceSampleDepth::Other(match depth {
            png::BitDepth::One => 1,
            png::BitDepth::Two => 2,
            png::BitDepth::Four => 4,
            png::BitDepth::Eight | png::BitDepth::Sixteen => unreachable!(),
        }),
    };
    let inspection = PngInspection {
        width: reader.info().width,
        height: reader.info().height,
        color_encoding: png_color_encoding(output_color, output_depth),
        interlaced: reader.info().interlaced,
        sample_depth,
    };
    let capability = if inspection.sample_depth == SourceSampleDepth::SixteenBit {
        RegionReadCapability::FullDecodeOnly
    } else if inspection.interlaced && inspection.sample_depth == SourceSampleDepth::EightBit {
        RegionReadCapability::DerivedBacking
    } else if inspection.interlaced {
        // Adam7 1/2/4-bit output can become exact RGBA8 after EXPAND, but the
        // current key model intentionally defers that native/output-depth split.
        RegionReadCapability::FullDecodeOnly
    } else {
        RegionReadCapability::SequentialBounded
    };
    let readiness = match capability {
        RegionReadCapability::SequentialBounded => RegionReadiness::Ready,
        RegionReadCapability::DerivedBacking => RegionReadiness::NeedsPreparation,
        RegionReadCapability::SeekableChunks | RegionReadCapability::FullDecodeOnly => {
            RegionReadiness::Unsupported
        }
    };
    Ok(RasterRegionInspection {
        info: RegionSourceInfo {
            descriptor: RegionSourceDescriptor {
                width: inspection.width,
                height: inspection.height,
                color_encoding: inspection.color_encoding,
                sample_depth: inspection.sample_depth,
                frame_index: 0,
                page_index: 0,
                decoder_contract: decoder_contract_for(format),
            },
            capability,
            readiness,
        },
        format,
    })
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
