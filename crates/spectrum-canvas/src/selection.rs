use std::{collections::VecDeque, sync::Arc};

use anyhow::{Context, Result, bail};
use image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::{Document, render_document};

pub const MAX_COLOR_SELECTION_PIXELS: u64 = 4_096 * 4_096;
const ANTIALIAS_COLOR_BAND: u8 = 16;

/// A persistent document-space pixel selection.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Selection {
    Rectangle {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    ColorMask {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        #[serde(with = "alpha_bytes")]
        alpha: Arc<[u8]>,
    },
}

impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Rectangle {
                    x,
                    y,
                    width,
                    height,
                },
                Self::Rectangle {
                    x: ox,
                    y: oy,
                    width: ow,
                    height: oh,
                },
            ) => (x, y, width, height) == (ox, oy, ow, oh),
            (
                Self::ColorMask {
                    x,
                    y,
                    width,
                    height,
                    alpha,
                },
                Self::ColorMask {
                    x: ox,
                    y: oy,
                    width: ow,
                    height: oh,
                    alpha: other_alpha,
                },
            ) => {
                (x, y, width, height) == (ox, oy, ow, oh)
                    && (Arc::ptr_eq(alpha, other_alpha) || alpha.as_ref() == other_alpha.as_ref())
            }
            _ => false,
        }
    }
}

impl Eq for Selection {}

impl Selection {
    pub fn rectangle(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self::Rectangle {
            x,
            y,
            width,
            height,
        }
    }

    pub fn color_mask(
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        alpha: impl Into<Arc<[u8]>>,
    ) -> Self {
        Self::ColorMask {
            x,
            y,
            width,
            height,
            alpha: alpha.into(),
        }
    }

    pub fn bounds(&self) -> (u32, u32, u32, u32) {
        match self {
            Self::Rectangle {
                x,
                y,
                width,
                height,
            }
            | Self::ColorMask {
                x,
                y,
                width,
                height,
                ..
            } => (*x, *y, *width, *height),
        }
    }

    pub fn alpha(&self) -> Option<&[u8]> {
        match self {
            Self::Rectangle { .. } => None,
            Self::ColorMask { alpha, .. } => Some(alpha),
        }
    }

    pub(crate) fn shared_alpha(&self) -> Option<Arc<[u8]>> {
        match self {
            Self::Rectangle { .. } => None,
            Self::ColorMask { alpha, .. } => Some(Arc::clone(alpha)),
        }
    }

    pub(crate) fn alpha_at(&self, x: u32, y: u32) -> u8 {
        let (origin_x, origin_y, width, height) = self.bounds();
        if x < origin_x
            || y < origin_y
            || x >= origin_x.saturating_add(width)
            || y >= origin_y.saturating_add(height)
        {
            return 0;
        }
        match self {
            Self::Rectangle { .. } => 255,
            Self::ColorMask { alpha, .. } => {
                let local_x = x - origin_x;
                let local_y = y - origin_y;
                alpha[(u64::from(local_y) * u64::from(width) + u64::from(local_x)) as usize]
            }
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
        let clipped_width = width.min(canvas_width - x);
        let clipped_height = height.min(canvas_height - y);
        match self {
            Self::Rectangle { .. } => Ok(Self::rectangle(x, y, clipped_width, clipped_height)),
            Self::ColorMask { alpha, .. } => {
                validate_alpha_length(width, height, &alpha)?;
                if clipped_width == width && clipped_height == height {
                    return Ok(Self::color_mask(x, y, width, height, alpha));
                }
                trim_mask_at(
                    x,
                    y,
                    clipped_width,
                    clipped_height,
                    crop_alpha(&alpha, width, 0, 0, clipped_width, clipped_height),
                )
            }
        }
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
        if right <= left || bottom <= top {
            return None;
        }
        let output_width = right - left;
        let output_height = bottom - top;
        match self {
            Self::Rectangle { .. } => Some(Self::rectangle(
                left - crop_x,
                top - crop_y,
                output_width,
                output_height,
            )),
            Self::ColorMask { alpha, .. } => trim_mask_at(
                left - crop_x,
                top - crop_y,
                output_width,
                output_height,
                crop_alpha(
                    &alpha,
                    width,
                    left - x,
                    top - y,
                    output_width,
                    output_height,
                ),
            )
            .ok(),
        }
    }
}

pub fn magic_wand_selection(
    document: &Document,
    x: u32,
    y: u32,
    tolerance: u8,
    contiguous: bool,
    antialias: bool,
) -> Result<Selection> {
    if x >= document.width || y >= document.height {
        bail!("magic wand point must be inside the canvas");
    }
    let pixels = u64::from(document.width) * u64::from(document.height);
    if pixels > MAX_COLOR_SELECTION_PIXELS {
        bail!(
            "magic wand is bounded to {MAX_COLOR_SELECTION_PIXELS} canvas pixels; resize or crop the document first"
        );
    }
    let image = render_document(document, None)?.into_rgba8();
    magic_wand_from_image(&image, x, y, tolerance, contiguous, antialias)
}

fn magic_wand_from_image(
    image: &RgbaImage,
    x: u32,
    y: u32,
    tolerance: u8,
    contiguous: bool,
    antialias: bool,
) -> Result<Selection> {
    let width = image.width();
    let height = image.height();
    let seed = premultiplied(image.get_pixel(x, y).0);
    let mut alpha = vec![0_u8; usize::try_from(u64::from(width) * u64::from(height))?];
    let distance_at =
        |px: u32, py: u32| color_distance(seed, premultiplied(image.get_pixel(px, py).0));

    if contiguous {
        let mut queue = VecDeque::from([(x, y)]);
        let mut visited = vec![false; alpha.len()];
        visited[pixel_index(width, x, y)?] = true;
        while let Some((px, py)) = queue.pop_front() {
            let index = pixel_index(width, px, py)?;
            if distance_at(px, py) > tolerance {
                continue;
            }
            alpha[index] = 255;
            for_each_neighbor(px, py, width, height, |(nx, ny)| {
                let neighbor = (u64::from(ny) * u64::from(width) + u64::from(nx)) as usize;
                if !visited[neighbor] {
                    visited[neighbor] = true;
                    queue.push_back((nx, ny));
                }
            });
        }
        if antialias {
            let core = alpha.clone();
            for py in 0..height {
                for px in 0..width {
                    let index = pixel_index(width, px, py)?;
                    if core[index] != 0 || !has_selected_neighbor(&core, width, height, px, py) {
                        continue;
                    }
                    alpha[index] = antialias_alpha(distance_at(px, py), tolerance);
                }
            }
        }
    } else {
        for py in 0..height {
            for px in 0..width {
                let distance = distance_at(px, py);
                let index = pixel_index(width, px, py)?;
                alpha[index] = if distance <= tolerance {
                    255
                } else if antialias {
                    antialias_alpha(distance, tolerance)
                } else {
                    0
                };
            }
        }
    }
    trim_mask(width, height, alpha)
}

fn antialias_alpha(distance: u8, tolerance: u8) -> u8 {
    let excess = distance.saturating_sub(tolerance);
    if excess == 0 || excess > ANTIALIAS_COLOR_BAND {
        return 0;
    }
    ((u16::from(ANTIALIAS_COLOR_BAND + 1 - excess) * 255) / u16::from(ANTIALIAS_COLOR_BAND + 1))
        as u8
}

fn trim_mask(width: u32, height: u32, alpha: Vec<u8>) -> Result<Selection> {
    trim_mask_at(0, 0, width, height, alpha)
}

fn trim_mask_at(
    origin_x: u32,
    origin_y: u32,
    width: u32,
    height: u32,
    alpha: Vec<u8>,
) -> Result<Selection> {
    validate_alpha_length(width, height, &alpha)?;
    let mut left = width;
    let mut top = height;
    let mut right = 0;
    let mut bottom = 0;
    for y in 0..height {
        for x in 0..width {
            if alpha[pixel_index(width, x, y)?] == 0 {
                continue;
            }
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    if right <= left || bottom <= top {
        bail!("magic wand produced an empty selection");
    }
    let trimmed = crop_alpha(&alpha, width, left, top, right - left, bottom - top);
    if trimmed.iter().all(|alpha| *alpha == 255) {
        return Ok(Selection::rectangle(
            origin_x + left,
            origin_y + top,
            right - left,
            bottom - top,
        ));
    }
    Ok(Selection::color_mask(
        origin_x + left,
        origin_y + top,
        right - left,
        bottom - top,
        trimmed,
    ))
}

fn premultiplied([r, g, b, a]: [u8; 4]) -> [u8; 4] {
    let premultiply = |channel: u8| ((u16::from(channel) * u16::from(a) + 127) / 255) as u8;
    [premultiply(r), premultiply(g), premultiply(b), a]
}

fn color_distance(left: [u8; 4], right: [u8; 4]) -> u8 {
    left.into_iter()
        .zip(right)
        .map(|(left, right)| left.abs_diff(right))
        .max()
        .unwrap_or(0)
}

fn for_each_neighbor(x: u32, y: u32, width: u32, height: u32, mut visit: impl FnMut((u32, u32))) {
    if x > 0 {
        visit((x - 1, y));
    }
    if x + 1 < width {
        visit((x + 1, y));
    }
    if y > 0 {
        visit((x, y - 1));
    }
    if y + 1 < height {
        visit((x, y + 1));
    }
}

fn has_selected_neighbor(alpha: &[u8], width: u32, height: u32, x: u32, y: u32) -> bool {
    let mut selected = false;
    for_each_neighbor(x, y, width, height, |(nx, ny)| {
        let index = (u64::from(ny) * u64::from(width) + u64::from(nx)) as usize;
        selected |= alpha[index] != 0;
    });
    selected
}

fn pixel_index(width: u32, x: u32, y: u32) -> Result<usize> {
    usize::try_from(u64::from(y) * u64::from(width) + u64::from(x))
        .context("selection pixel index exceeds platform limits")
}

fn validate_alpha_length(width: u32, height: u32, alpha: &[u8]) -> Result<()> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_COLOR_SELECTION_PIXELS {
        bail!("selection alpha mask exceeds the {MAX_COLOR_SELECTION_PIXELS}-pixel limit");
    }
    let expected = usize::try_from(pixels)?;
    if alpha.len() != expected {
        bail!(
            "selection alpha mask has {} bytes; expected {expected}",
            alpha.len()
        );
    }
    if alpha.iter().all(|value| *value == 0) {
        bail!("selection alpha mask cannot be empty");
    }
    Ok(())
}

fn crop_alpha(
    source: &[u8],
    source_width: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let mut output = Vec::with_capacity((u64::from(width) * u64::from(height)) as usize);
    for row in y..y + height {
        let start = (u64::from(row) * u64::from(source_width) + u64::from(x)) as usize;
        output.extend_from_slice(&source[start..start + width as usize]);
    }
    output
}

mod alpha_bytes {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};

    const MAX_ENCODED_BYTES: usize = (super::MAX_COLOR_SELECTION_PIXELS as usize).div_ceil(3) * 4;

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
                "selection alpha mask exceeds the encoded size limit",
            ));
        }
        STANDARD
            .decode(encoded)
            .map(std::sync::Arc::from)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn contiguous_and_global_modes_split_disconnected_islands() {
        let mut image = RgbaImage::from_pixel(5, 3, Rgba([0, 0, 0, 255]));
        image.put_pixel(0, 1, Rgba([200, 10, 10, 255]));
        image.put_pixel(4, 1, Rgba([200, 10, 10, 255]));
        let contiguous = magic_wand_from_image(&image, 0, 1, 0, true, false).unwrap();
        assert_eq!(contiguous.bounds(), (0, 1, 1, 1));
        let global = magic_wand_from_image(&image, 0, 1, 0, false, false).unwrap();
        assert_eq!(global.bounds(), (0, 1, 5, 1));
        assert_eq!(global.alpha().unwrap(), &[255, 0, 0, 0, 255]);
    }

    #[test]
    fn antialiasing_adds_only_a_soft_boundary_without_flood_leakage() {
        let image = RgbaImage::from_raw(
            4,
            1,
            vec![
                100, 0, 0, 255, 105, 0, 0, 255, 112, 0, 0, 255, 120, 0, 0, 255,
            ],
        )
        .unwrap();
        let selection = magic_wand_from_image(&image, 0, 0, 5, true, true).unwrap();
        assert_eq!(selection.bounds(), (0, 0, 3, 1));
        assert_eq!(selection.alpha().unwrap()[..2], [255, 255]);
        assert!(selection.alpha().unwrap()[2] > 0);
    }

    #[test]
    fn color_mask_json_is_compact_base64_and_round_trips() {
        let selection = Selection::color_mask(2, 3, 2, 2, vec![255, 128, 64, 0]);
        let json = serde_json::to_string(&selection).unwrap();
        assert!(json.contains("/4BAAA=="));
        assert_eq!(serde_json::from_str::<Selection>(&json).unwrap(), selection);
    }

    #[test]
    fn uniform_matches_canonicalize_to_a_compact_rectangle() {
        let image = RgbaImage::from_pixel(64, 32, Rgba([20, 40, 60, 255]));
        assert_eq!(
            magic_wand_from_image(&image, 3, 4, 0, true, true).unwrap(),
            Selection::rectangle(0, 0, 64, 32)
        );
    }

    #[test]
    fn crop_through_a_color_mask_gap_clears_the_selection() {
        let selection = Selection::color_mask(10, 20, 5, 1, vec![255, 0, 0, 0, 255]);
        assert_eq!(selection.cropped(12, 20, 2, 1), None);
    }

    #[test]
    fn crop_trims_and_translates_the_remaining_color_mask_island() {
        let selection =
            Selection::color_mask(10, 20, 5, 2, vec![255, 0, 0, 0, 255, 128, 0, 0, 0, 64]);
        let cropped = selection.cropped(13, 19, 2, 4).unwrap();
        assert_eq!(cropped.bounds(), (1, 1, 1, 2));
        assert_eq!(cropped.alpha(), Some([255, 64].as_slice()));
    }

    #[test]
    fn flood_fill_never_wraps_across_rows() {
        let mut image = RgbaImage::from_pixel(3, 2, Rgba([0, 0, 0, 255]));
        image.put_pixel(2, 0, Rgba([200, 0, 0, 255]));
        image.put_pixel(0, 1, Rgba([200, 0, 0, 255]));
        assert_eq!(
            magic_wand_from_image(&image, 2, 0, 0, true, false)
                .unwrap()
                .bounds(),
            (2, 0, 1, 1)
        );
    }

    #[test]
    fn tolerance_uses_premultiplied_rgba_max_channel_distance() {
        let image =
            RgbaImage::from_raw(3, 1, vec![255, 0, 0, 0, 0, 255, 0, 0, 255, 0, 0, 16]).unwrap();
        // Fully transparent hidden RGB is ignored by premultiplication.
        let transparent = magic_wand_from_image(&image, 0, 0, 0, false, false).unwrap();
        assert_eq!(transparent, Selection::rectangle(0, 0, 2, 1));
        // Alpha remains a visible max-channel component, so the semitransparent
        // red sample requires tolerance 16 even though its premultiplied red is 16.
        assert_eq!(
            magic_wand_from_image(&image, 0, 0, 15, false, false).unwrap(),
            Selection::rectangle(0, 0, 2, 1)
        );
        assert_eq!(
            magic_wand_from_image(&image, 0, 0, 16, false, false).unwrap(),
            Selection::rectangle(0, 0, 3, 1)
        );
    }

    #[test]
    fn irregular_4096_mask_and_command_clones_share_one_alpha_plane() {
        let mut alpha = vec![0; 4_096 * 4_096];
        alpha[0] = 255;
        let last = alpha.len() - 1;
        alpha[last] = 128;
        let selection = Selection::color_mask(0, 0, 4_096, 4_096, alpha);
        let cloned = selection.clone();
        let command = crate::Command::SetSelection {
            selection: Some(selection),
        };
        let cloned_command = command.clone();
        let (
            crate::Command::SetSelection {
                selection: Some(Selection::ColorMask { alpha, .. }),
            },
            crate::Command::SetSelection {
                selection:
                    Some(Selection::ColorMask {
                        alpha: cloned_alpha,
                        ..
                    }),
            },
        ) = (&command, &cloned_command)
        else {
            panic!("command should retain the irregular selection");
        };
        assert!(Arc::ptr_eq(alpha, cloned_alpha));
        let Selection::ColorMask {
            alpha: direct_clone,
            ..
        } = &cloned
        else {
            unreachable!()
        };
        assert!(Arc::ptr_eq(alpha, direct_clone));
    }
}
