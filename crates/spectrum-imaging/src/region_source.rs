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
            && matches!(self.sample_depth, SourceSampleDepth::EightBit)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
