//! Converting decoded pictures to RGBA (for the screen) or YUV (for
//! playback copies) at a chosen size, on several threads, with the colors
//! other players show.
use anyhow::{Context, Result, bail};
use ffmpeg_next::{ffi, format::Pixel, software::scaling::Flags, util::frame};
use image::RgbaImage;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScalerKey {
    pub format: Pixel,
    pub width: u32,
    pub height: u32,
    pub out_width: u32,
    pub out_height: u32,
    pub space: ffi::AVColorSpace,
    pub range: ffi::AVColorRange,
    /// RGBA, BGRA (as GPU uploads want), or YUV 4:2:0 for encoding.
    pub output: Pixel,
}

/// A configured converter for one input layout and output size.
pub(crate) struct Scaler {
    pub key: ScalerKey,
    context: *mut ffi::SwsContext,
}

impl Scaler {
    pub fn new(key: ScalerKey) -> Result<Self> {
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(16));
        let flags = (Flags::BILINEAR | Flags::ACCURATE_RND | Flags::FULL_CHR_H_INT).bits();
        let format: ffi::AVPixelFormat = key.format.into();
        unsafe {
            let context = ffi::sws_alloc_context();
            if context.is_null() {
                bail!("could not convert the picture's colors");
            }
            let mut scaler = Self { key, context };
            let options: [(&std::ffi::CStr, i64); 8] = [
                (c"srcw", i64::from(key.width)),
                (c"srch", i64::from(key.height)),
                (c"src_format", format as i64),
                (c"dstw", i64::from(key.out_width)),
                (c"dsth", i64::from(key.out_height)),
                (c"dst_format", ffi::AVPixelFormat::from(key.output) as i64),
                (c"sws_flags", flags as i64),
                (c"threads", threads as i64),
            ];
            for (name, value) in options {
                if ffi::av_opt_set_int(context as *mut _, name.as_ptr(), value, 0) < 0 {
                    bail!("could not set up color conversion ({name:?})");
                }
            }
            if ffi::sws_init_context(context, std::ptr::null_mut(), std::ptr::null_mut()) < 0 {
                bail!("Spectrum cannot convert {:?} pictures", key.format);
            }
            scaler.set_colorspace();
            Ok(scaler)
        }
    }

    /// Converts one decoded picture.
    pub fn run(&mut self, decoded: &frame::Video) -> Result<frame::Video> {
        let (width, height) = (self.key.out_width, self.key.out_height);
        let mut converted = frame::Video::new(self.key.output, width, height);
        let result =
            unsafe { ffi::sws_scale_frame(self.context, converted.as_mut_ptr(), decoded.as_ptr()) };
        if result < 0 {
            bail!("could not convert the picture's colors");
        }
        Ok(converted)
    }

    /// Converts one decoded picture to packed RGBA or BGRA pixels.
    pub fn image(&mut self, decoded: &frame::Video) -> Result<RgbaImage> {
        let (width, height) = (self.key.out_width, self.key.out_height);
        let rgba = self.run(decoded)?;
        let stride = rgba.stride(0);
        let data = rgba.data(0);
        let row = width as usize * 4;
        let mut pixels = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            pixels.extend_from_slice(&data[y * stride..y * stride + row]);
        }
        RgbaImage::from_raw(width, height, pixels).context("bad frame size")
    }

    /// Tells the converter the picture's YUV matrix and range. Unlabeled HD
    /// and larger pictures use BT.709.
    // The constants' integer type differs between FFmpeg versions.
    #[allow(clippy::unnecessary_cast)]
    fn set_colorspace(&mut self) {
        use ffi::AVColorSpace::*;
        let matrix = match self.key.space {
            AVCOL_SPC_BT709 => ffi::SWS_CS_ITU709,
            AVCOL_SPC_BT470BG | AVCOL_SPC_SMPTE170M => ffi::SWS_CS_ITU601,
            AVCOL_SPC_SMPTE240M => ffi::SWS_CS_SMPTE240M,
            AVCOL_SPC_FCC => ffi::SWS_CS_FCC,
            AVCOL_SPC_BT2020_NCL | AVCOL_SPC_BT2020_CL => ffi::SWS_CS_BT2020,
            _ if self.key.height >= 720 => ffi::SWS_CS_ITU709,
            _ => ffi::SWS_CS_ITU601,
        };
        let rgb_in = is_rgb(self.key.format);
        let rgb_out = is_rgb(self.key.output);
        let full = i32::from(rgb_in || self.key.range == ffi::AVColorRange::AVCOL_RANGE_JPEG);
        // Screens get full-range RGB; encoders get video-range YUV, in the
        // source's matrix for copies and BT.709 for exports from RGB.
        let (out_matrix, out_full) = match (rgb_in, rgb_out) {
            (_, true) => (ffi::SWS_CS_DEFAULT, 1),
            (true, false) => (ffi::SWS_CS_ITU709, 0),
            (false, false) => (matrix, 0),
        };
        unsafe {
            ffi::sws_setColorspaceDetails(
                self.context,
                ffi::sws_getCoefficients(matrix as i32),
                full,
                ffi::sws_getCoefficients(out_matrix as i32),
                out_full,
                0,
                1 << 16,
                1 << 16,
            );
        }
    }
}

impl Drop for Scaler {
    fn drop(&mut self) {
        unsafe { ffi::sws_freeContext(self.context) };
    }
}

fn is_rgb(format: Pixel) -> bool {
    unsafe {
        let descriptor = ffi::av_pix_fmt_desc_get(format.into());
        !descriptor.is_null() && (*descriptor).flags & ffi::AV_PIX_FMT_FLAG_RGB as u64 != 0
    }
}

/// A size no larger than `max` on its long edge, never larger than the
/// source, and at least one pixel.
pub(crate) fn fit(width: u32, height: u32, max: Option<u32>) -> (u32, u32) {
    let Some(max) = max.filter(|max| *max < width.max(height)) else {
        return (width, height);
    };
    let scale = f64::from(max) / f64::from(width.max(height));
    (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    )
}
