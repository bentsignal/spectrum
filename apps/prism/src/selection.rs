use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// A persistent document-space pixel selection.
///
/// The tagged enum leaves room for future lasso selections without weakening
/// the rectangle-only contract of the first portable vertical slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Selection {
    Rectangle {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

impl Selection {
    pub fn rectangle(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self::Rectangle {
            x,
            y,
            width,
            height,
        }
    }

    pub fn bounds(self) -> (u32, u32, u32, u32) {
        match self {
            Self::Rectangle {
                x,
                y,
                width,
                height,
            } => (x, y, width, height),
        }
    }

    pub(crate) fn validated(self, canvas_width: u32, canvas_height: u32) -> Result<Self> {
        let (x, y, width, height) = self.bounds();
        if width == 0 || height == 0 {
            bail!("selection width and height must be nonzero");
        }
        if x >= canvas_width || y >= canvas_height {
            bail!("selection must overlap the canvas");
        }
        Ok(Self::rectangle(
            x,
            y,
            width.min(canvas_width - x),
            height.min(canvas_height - y),
        ))
    }

    pub(crate) fn clipped(self, canvas_width: u32, canvas_height: u32) -> Option<Self> {
        self.validated(canvas_width, canvas_height).ok()
    }

    pub(crate) fn cropped(
        self,
        crop_x: u32,
        crop_y: u32,
        crop_width: u32,
        crop_height: u32,
    ) -> Option<Self> {
        let (x, y, width, height) = self.bounds();
        let right = x
            .saturating_add(width)
            .min(crop_x.saturating_add(crop_width));
        let bottom = y
            .saturating_add(height)
            .min(crop_y.saturating_add(crop_height));
        let left = x.max(crop_x);
        let top = y.max(crop_y);
        (right > left && bottom > top)
            .then(|| Self::rectangle(left - crop_x, top - crop_y, right - left, bottom - top))
    }
}
