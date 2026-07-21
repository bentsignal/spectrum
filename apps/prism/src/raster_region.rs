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

pub fn inspect_raster_region_source(path: &Path) -> Result<RasterRegionInspection> {
    let reader = image::ImageReader::open(path)
        .with_context(|| format!("could not open {}", path.display()))?
        .with_guessed_format()
        .with_context(|| format!("could not identify {}", path.display()))?;
    let format = reader
        .format()
        .with_context(|| format!("could not identify {}", path.display()))?;
    let decoder = reader
        .into_decoder()
        .with_context(|| format!("could not inspect {}", path.display()))?;
    let (width, height) = decoder.dimensions();
    let color = decoder.color_type();
    let mut sample_depth = image_sample_depth(color);
    let capability = match format {
        ImageFormat::Png => {
            let png = inspect_png(path)?;
            sample_depth = png.sample_depth;
            if sample_depth == SourceSampleDepth::SixteenBit {
                RegionReadCapability::FullDecodeOnly
            } else if png.interlaced && sample_depth == SourceSampleDepth::EightBit {
                RegionReadCapability::DerivedBacking
            } else if png.interlaced {
                RegionReadCapability::FullDecodeOnly
            } else {
                RegionReadCapability::SequentialBounded
            }
        }
        ImageFormat::Jpeg | ImageFormat::WebP if sample_depth == SourceSampleDepth::EightBit => {
            RegionReadCapability::DerivedBacking
        }
        // Container-level capability only. Readiness stays Unsupported until
        // Prism inspects and reads the file's actual strip/tile organization.
        ImageFormat::Tiff if sample_depth == SourceSampleDepth::EightBit => {
            RegionReadCapability::SeekableChunks
        }
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
    interlaced: bool,
    sample_depth: SourceSampleDepth,
}

fn inspect_png(path: &Path) -> Result<PngInspection> {
    let file = File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let decoder = png::Decoder::new_with_limits(
        BufReader::new(file),
        png::Limits {
            bytes: PNG_HEADER_LIMIT_BYTES,
        },
    );
    let reader = decoder
        .read_info()
        .with_context(|| format!("could not inspect PNG {}", path.display()))?;
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
    Ok(PngInspection {
        interlaced: reader.info().interlaced,
        sample_depth,
    })
}
