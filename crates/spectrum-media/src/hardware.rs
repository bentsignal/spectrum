//! Hardware decoding: VideoToolbox on macOS, Direct3D 11 on Windows, and
//! VA-API on Linux. FFmpeg asks which format to decode to; Spectrum takes the
//! hardware one when offered and the software one otherwise, so footage the
//! hardware cannot decode (or a computer without it) still plays. Frames
//! stay on the GPU until one is shown, then are copied out once.
use anyhow::{Context, Result, bail};
use ffmpeg_next::{ffi, util::frame};
use std::ptr;

/// The hardware this platform decodes with, and the frames it produces.
fn platform() -> Option<(ffi::AVHWDeviceType, ffi::AVPixelFormat, &'static str)> {
    use ffi::{AVHWDeviceType::*, AVPixelFormat::*};
    if cfg!(target_os = "macos") {
        Some((
            AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
            AV_PIX_FMT_VIDEOTOOLBOX,
            "VideoToolbox",
        ))
    } else if cfg!(target_os = "windows") {
        Some((AV_HWDEVICE_TYPE_D3D11VA, AV_PIX_FMT_D3D11, "Direct3D 11"))
    } else if cfg!(target_os = "linux") {
        Some((AV_HWDEVICE_TYPE_VAAPI, AV_PIX_FMT_VAAPI, "VA-API"))
    } else {
        None
    }
}

/// Whether hardware decoding is allowed: `SPECTRUM_HARDWARE_DECODING=off`
/// turns it off, to compare or to work around a driver.
fn allowed() -> bool {
    std::env::var("SPECTRUM_HARDWARE_DECODING").map_or(true, |value| value != "off")
}

/// Offers a decoder this platform's hardware before it opens. Returns the
/// hardware's name when a device opened; the decoder still falls back to
/// software for streams the hardware does not support.
///
/// # Safety
/// `context` must be an unopened decoder context.
pub(crate) unsafe fn attach(context: *mut ffi::AVCodecContext) -> Option<&'static str> {
    let (kind, format, name) = platform()?;
    if !allowed() {
        return None;
    }
    let device = shared_device(kind)?;
    unsafe {
        (*context).hw_device_ctx = ffi::av_buffer_ref(device as *mut ffi::AVBufferRef);
        (*context).opaque = format as i32 as usize as *mut _;
        (*context).get_format = Some(choose_format);
    }
    Some(name)
}

/// One device for the whole process, opened the first time it is needed,
/// quietly: a computer without the hardware just decodes in software.
fn shared_device(kind: ffi::AVHWDeviceType) -> Option<usize> {
    static DEVICE: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *DEVICE.get_or_init(|| unsafe {
        let level = ffi::av_log_get_level();
        ffi::av_log_set_level(ffi::AV_LOG_QUIET);
        let mut device = ptr::null_mut();
        let made = ffi::av_hwdevice_ctx_create(&mut device, kind, ptr::null(), ptr::null_mut(), 0);
        ffi::av_log_set_level(level);
        (made >= 0 && !device.is_null()).then_some(device as usize)
    })
}

/// Picks the hardware format when FFmpeg offers it, else the first
/// software one.
unsafe extern "C" fn choose_format(
    context: *mut ffi::AVCodecContext,
    formats: *const ffi::AVPixelFormat,
) -> ffi::AVPixelFormat {
    unsafe {
        let wanted = (*context).opaque as usize as i32;
        let mut software = None;
        let mut at = formats;
        while *at != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            if *at as i32 == wanted {
                return *at;
            }
            let descriptor = ffi::av_pix_fmt_desc_get(*at);
            let hardware = !descriptor.is_null()
                && (*descriptor).flags & ffi::AV_PIX_FMT_FLAG_HWACCEL as u64 != 0;
            if !hardware && software.is_none() {
                software = Some(*at);
            }
            at = at.add(1);
        }
        software.unwrap_or(*formats)
    }
}

/// Whether a decoded frame lives on the GPU.
pub(crate) fn on_gpu(frame: &frame::Video) -> bool {
    unsafe { !(*frame.as_ptr()).hw_frames_ctx.is_null() }
}

/// Copies a GPU frame to memory, keeping its timing and color details.
pub(crate) fn download(frame: &frame::Video) -> Result<frame::Video> {
    let mut copied = frame::Video::empty();
    unsafe {
        if ffi::av_hwframe_transfer_data(copied.as_mut_ptr(), frame.as_ptr(), 0) < 0 {
            bail!("could not copy a hardware-decoded frame");
        }
        if ffi::av_frame_copy_props(copied.as_mut_ptr(), frame.as_ptr()) < 0 {
            bail!("could not copy a hardware-decoded frame's details");
        }
    }
    Ok(copied)
}

/// A frame in memory: `frame` itself, or its copy from the GPU.
pub(crate) fn in_memory<'a>(
    frame: &'a frame::Video,
    copy: &'a mut Option<frame::Video>,
) -> Result<&'a frame::Video> {
    if !on_gpu(frame) {
        return Ok(frame);
    }
    if copy.is_none() {
        *copy = Some(download(frame)?);
    }
    copy.as_ref().context("no frame copied")
}
