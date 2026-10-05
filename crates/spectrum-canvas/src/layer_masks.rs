use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerMask {
    pub enabled: bool,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub invert: bool,
}

impl Default for LayerMask {
    fn default() -> Self {
        Self {
            enabled: false,
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            invert: false,
        }
    }
}

impl LayerMask {
    pub(crate) fn sanitized(self) -> Self {
        let x = self.x.clamp(0.0, 1.0);
        let y = self.y.clamp(0.0, 1.0);
        Self {
            enabled: self.enabled,
            x,
            y,
            width: self.width.clamp(0.001, 1.0 - x),
            height: self.height.clamp(0.001, 1.0 - y),
            invert: self.invert,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PixelMask {
    pub width: u32,
    pub height: u32,
    pub content_hash: [u8; 32],
    #[serde(with = "pixel_mask_bytes")]
    pub alpha: Arc<[u8]>,
}

impl PixelMask {
    pub fn new(width: u32, height: u32, alpha: impl Into<Arc<[u8]>>) -> Self {
        let alpha = alpha.into();
        let content_hash = Sha256::digest(alpha.as_ref()).into();
        Self {
            width,
            height,
            content_hash,
            alpha,
        }
    }

    pub fn identity(&self) -> [u8; 32] {
        self.content_hash
    }

    pub(crate) fn has_valid_identity(&self) -> bool {
        self.content_hash == <[u8; 32]>::from(Sha256::digest(self.alpha.as_ref()))
    }
}

impl PartialEq for PixelMask {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.content_hash == other.content_hash
            && (Arc::ptr_eq(&self.alpha, &other.alpha)
                || self.alpha.as_ref() == other.alpha.as_ref())
    }
}

impl Eq for PixelMask {}

mod pixel_mask_bytes {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};

    const MAX_ENCODED_BYTES: usize = (crate::MAX_COLOR_SELECTION_PIXELS as usize).div_ceil(3) * 4;

    pub fn serialize<S: Serializer>(
        bytes: &std::sync::Arc<[u8]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes.as_ref()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<std::sync::Arc<[u8]>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() > MAX_ENCODED_BYTES {
            return Err(serde::de::Error::custom(
                "pixel mask exceeds the encoded size limit",
            ));
        }
        STANDARD
            .decode(encoded)
            .map(std::sync::Arc::from)
            .map_err(serde::de::Error::custom)
    }
}
