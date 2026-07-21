use std::{error::Error, fmt};

use image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::PixelRegion;

/// How an encoded source can provide exact pixels for a viewport request.
///
/// This describes the decoder/storage contract, not current availability. In
/// particular, [`Self::DerivedBacking`] still requires a completed derived
/// plane before it can service region reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionReadCapability {
    /// Rows can be decoded sequentially without materializing the full image.
    SequentialBounded,
    /// Exact region reads require an immutable decoded backing plane.
    DerivedBacking,
    /// The format has independently seekable chunks, but no provider exists yet.
    SeekableChunks,
    /// Exact reads currently require decoding the complete source.
    FullDecodeOnly,
}

/// Whether a capability is usable without synchronous preparation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionReadiness {
    Ready,
    NeedsPreparation,
    Unsupported,
}

/// Native sample representation reported by the source decoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSampleDepth {
    EightBit,
    SixteenBit,
    Float32,
    Other(u8),
}

/// Everything that can change the exact RGBA8 pixels produced by a decoder.
///
/// Applications should include this descriptor alongside the encoded-content
/// hash when deriving a backing-cache key. Transforms, adjustments, and layer
/// identity deliberately do not belong here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionSourceDescriptor {
    pub width: u32,
    pub height: u32,
    pub color_encoding: String,
    pub sample_depth: SourceSampleDepth,
    pub frame_index: u32,
    pub page_index: u32,
    pub decoder_contract: String,
}

impl RegionSourceDescriptor {
    pub fn exact_rgba8_plane_bytes(&self) -> Option<u64> {
        u64::from(self.width)
            .checked_mul(u64::from(self.height))?
            .checked_mul(4)
    }

    pub fn supports_exact_rgba8_backing(&self) -> bool {
        self.width > 0
            && self.height > 0
            && matches!(
                self.color_encoding.as_str(),
                "l8" | "la8" | "rgb8" | "rgba8"
            )
            && matches!(
                self.sample_depth,
                SourceSampleDepth::EightBit | SourceSampleDepth::Other(1 | 2 | 4)
            )
            && self.exact_rgba8_plane_bytes().is_some()
    }
}

/// Capability and current availability for one immutable encoded source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionSourceInfo {
    pub descriptor: RegionSourceDescriptor,
    pub capability: RegionReadCapability,
    pub readiness: RegionReadiness,
}

impl RegionSourceInfo {
    pub fn supports_region_reads_now(&self) -> bool {
        self.readiness == RegionReadiness::Ready
            && self.capability != RegionReadCapability::FullDecodeOnly
    }
}

/// Error returned before a provider attempts an out-of-contract region read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionRequestError {
    Empty,
    Overflow,
    OutOfBounds,
    ExceedsPixelLimit,
}

impl fmt::Display for RegionRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "region dimensions must be positive",
            Self::Overflow => "region coordinates overflow",
            Self::OutOfBounds => "region exceeds source dimensions",
            Self::ExceedsPixelLimit => "region exceeds the provider pixel limit",
        })
    }
}

impl Error for RegionRequestError {}

pub fn validate_region_request(
    descriptor: &RegionSourceDescriptor,
    region: PixelRegion,
    max_pixels: u64,
) -> Result<(), RegionRequestError> {
    if region.width == 0 || region.height == 0 {
        return Err(RegionRequestError::Empty);
    }
    let right = region
        .x
        .checked_add(region.width)
        .ok_or(RegionRequestError::Overflow)?;
    let bottom = region
        .y
        .checked_add(region.height)
        .ok_or(RegionRequestError::Overflow)?;
    if right > descriptor.width || bottom > descriptor.height {
        return Err(RegionRequestError::OutOfBounds);
    }
    let pixels = u64::from(region.width)
        .checked_mul(u64::from(region.height))
        .ok_or(RegionRequestError::Overflow)?;
    if pixels > max_pixels {
        return Err(RegionRequestError::ExceedsPixelLimit);
    }
    Ok(())
}

/// A ready source that can return exact source-resolution RGBA8 regions.
pub trait ExactRegionSource: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn info(&self) -> &RegionSourceInfo;
    fn read_exact_region(&self, region: PixelRegion) -> Result<RgbaImage, Self::Error>;
}

/// Object-safe exact-region source used by application-owned provider maps.
///
/// Concrete providers keep their typed [`ExactRegionSource::Error`]. This
/// erased view lets a renderer retain heterogeneous providers without making
/// the neutral imaging crate depend on any application's cache implementation.
pub trait DynExactRegionSource: Send + Sync {
    fn info(&self) -> &RegionSourceInfo;
    fn read_exact_region(
        &self,
        region: PixelRegion,
    ) -> Result<RgbaImage, Box<dyn Error + Send + Sync + 'static>>;
}

impl<T> DynExactRegionSource for T
where
    T: ExactRegionSource,
{
    fn info(&self) -> &RegionSourceInfo {
        ExactRegionSource::info(self)
    }

    fn read_exact_region(
        &self,
        region: PixelRegion,
    ) -> Result<RgbaImage, Box<dyn Error + Send + Sync + 'static>> {
        ExactRegionSource::read_exact_region(self, region)
            .map_err(|error| Box::new(error) as Box<dyn Error + Send + Sync + 'static>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestReadError;

    impl fmt::Display for TestReadError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("test provider read failed")
        }
    }

    impl Error for TestReadError {}

    struct TestSource {
        info: RegionSourceInfo,
        fail: bool,
    }

    impl ExactRegionSource for TestSource {
        type Error = TestReadError;

        fn info(&self) -> &RegionSourceInfo {
            &self.info
        }

        fn read_exact_region(&self, region: PixelRegion) -> Result<RgbaImage, Self::Error> {
            if self.fail {
                return Err(TestReadError);
            }
            Ok(RgbaImage::from_pixel(
                region.width,
                region.height,
                image::Rgba([region.x as u8, region.y as u8, 17, 255]),
            ))
        }
    }

    fn descriptor() -> RegionSourceDescriptor {
        RegionSourceDescriptor {
            width: 11,
            height: 7,
            color_encoding: "rgba8".into(),
            sample_depth: SourceSampleDepth::EightBit,
            frame_index: 0,
            page_index: 0,
            decoder_contract: "test:v1".into(),
        }
    }

    #[test]
    fn validates_regions_without_coordinate_wraparound() {
        assert_eq!(
            validate_region_request(
                &descriptor(),
                PixelRegion {
                    x: u32::MAX,
                    y: 0,
                    width: 2,
                    height: 1,
                },
                100,
            ),
            Err(RegionRequestError::Overflow)
        );
        assert_eq!(
            validate_region_request(
                &descriptor(),
                PixelRegion {
                    x: 2,
                    y: 3,
                    width: 6,
                    height: 3,
                },
                17,
            ),
            Err(RegionRequestError::ExceedsPixelLimit)
        );
    }

    #[test]
    fn derived_capability_does_not_imply_readiness() {
        let info = RegionSourceInfo {
            descriptor: descriptor(),
            capability: RegionReadCapability::DerivedBacking,
            readiness: RegionReadiness::NeedsPreparation,
        };
        assert!(!info.supports_region_reads_now());
    }

    #[test]
    fn exact_backings_accept_expanded_subbyte_pixels_but_not_high_precision_layouts() {
        let mut descriptor = descriptor();
        descriptor.color_encoding = "l8".into();
        descriptor.sample_depth = SourceSampleDepth::Other(1);
        assert!(descriptor.supports_exact_rgba8_backing());

        descriptor.sample_depth = SourceSampleDepth::SixteenBit;
        descriptor.color_encoding = "l16".into();
        assert!(!descriptor.supports_exact_rgba8_backing());
    }

    #[test]
    fn concrete_sources_have_an_object_safe_exact_region_view() {
        let source: Box<dyn DynExactRegionSource> = Box::new(TestSource {
            info: RegionSourceInfo {
                descriptor: descriptor(),
                capability: RegionReadCapability::DerivedBacking,
                readiness: RegionReadiness::Ready,
            },
            fail: false,
        });
        let region = PixelRegion {
            x: 2,
            y: 3,
            width: 4,
            height: 2,
        };
        assert_eq!(source.info().descriptor, descriptor());
        assert_eq!(
            source.read_exact_region(region).unwrap(),
            RgbaImage::from_pixel(4, 2, image::Rgba([2, 3, 17, 255]))
        );
    }

    #[test]
    fn object_safe_sources_preserve_concrete_error_context() {
        let source: Box<dyn DynExactRegionSource> = Box::new(TestSource {
            info: RegionSourceInfo {
                descriptor: descriptor(),
                capability: RegionReadCapability::DerivedBacking,
                readiness: RegionReadiness::Ready,
            },
            fail: true,
        });
        let error = source
            .read_exact_region(PixelRegion {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            })
            .unwrap_err();
        assert_eq!(error.to_string(), "test provider read failed");
        assert!(error.downcast_ref::<TestReadError>().is_some());
    }
}
