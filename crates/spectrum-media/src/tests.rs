//! Decoding against clips made by the `ffmpeg` command: each frame's
//! brightness encodes its number, so a test can tell exactly which frame
//! it got.
use super::*;
use std::{path::Path, process::Command};

/// Runs `ffmpeg` to make a fixture.
pub(crate) fn make(arguments: &[&str], output: &Path) {
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(arguments)
        .arg(output)
        .status()
        .expect("the ffmpeg command makes media fixtures; install FFmpeg");
    assert!(
        status.success(),
        "ffmpeg could not make {}",
        output.display()
    );
}

/// Two seconds at 29.97 fps, 64x48, frame N with luma 16 + 3N, and a sine
/// tone, losslessly coded with B-frames so decode and display order differ.
fn counting_clip(directory: &Path, extension: &str) -> std::path::PathBuf {
    let path = directory.join(format!("counting.{extension}"));
    make(
        &[
            "-f",
            "lavfi",
            "-i",
            "nullsrc=s=64x48:r=30000/1001:d=2,geq=lum='16+3*N':cb=128:cr=128",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2",
            "-c:v",
            "libx264",
            "-qp",
            "0",
            "-bf",
            "2",
            "-g",
            "12",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
        ],
        &path,
    );
    path
}

fn frame_number(frame: &Frame) -> i64 {
    let luma = frame.image.get_pixel(10, 10).0[1] as f64;
    // Limited-range luma 16 + 3N shows as (3N) * 255 / 219.
    (luma * 219.0 / 255.0 / 3.0).round() as i64
}

#[test]
fn probing_reads_streams_and_exact_rates() {
    let directory = tempfile::tempdir().unwrap();
    let info = probe(&counting_clip(directory.path(), "mp4")).unwrap();
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (64, 48));
    assert_eq!(video.rate, FrameRate::new(30000, 1001));
    assert_eq!(video.frames, 60);
    assert_eq!(video.codec, "h264");
    assert_eq!(info.audio.len(), 1);
    assert_eq!(info.audio[0].sample_rate, 48_000);
    assert!(info.is_footage());
    let text = directory.path().join("notes.txt");
    std::fs::write(&text, b"not media").unwrap();
    assert!(probe(&text).is_err());
}

#[test]
fn every_frame_is_found_exactly_in_any_order() {
    let directory = tempfile::tempdir().unwrap();
    // Matroska rounds timestamps to milliseconds; MP4 keeps them exact.
    for extension in ["mp4", "mkv"] {
        let path = counting_clip(directory.path(), extension);
        let mut reader = FrameReader::open(&path).unwrap();
        let rate = reader.rate();
        for wanted in [0, 1, 2, 13, 59, 30, 31, 5, 47, 46, 12, 11] {
            let frame = reader.frame_at(rate.time_of(wanted), None).unwrap();
            assert_eq!(frame.index, wanted, "{extension}: asked for {wanted}");
            assert_eq!(
                frame_number(&frame),
                wanted,
                "{extension}: pixels of {wanted}"
            );
            // The instant before the next frame still shows this one.
            let late = reader
                .frame_at(rate.time_of(wanted + 1) - Time(1), None)
                .unwrap();
            assert_eq!(late.index, wanted.min(59), "{extension}: end of {wanted}");
        }
        // Past the end shows the last frame; sizes shrink to fit.
        let last = reader.frame_at(Time::from_seconds(9.0), Some(32)).unwrap();
        assert_eq!(last.index, 59);
        assert_eq!(last.image.dimensions(), (32, 24));
    }
}

#[test]
fn sound_decodes_resamples_and_seeks() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tone.wav");
    make(
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1000:sample_rate=44100:duration=1.5",
            "-ac",
            "1",
        ],
        &path,
    );
    let info = probe(&path).unwrap();
    assert!(!info.is_footage());
    assert_eq!(info.audio[0].channels, 1);
    let mut reader = AudioReader::open(&path, 0).unwrap();
    let mut total = 0;
    let mut chunk = vec![0f32; 4096 * CHANNELS];
    loop {
        let frames = reader.read(&mut chunk).unwrap();
        total += frames;
        if frames < 4096 {
            break;
        }
    }
    let expected = 72_000i64;
    assert!((total as i64 - expected).abs() < 200, "{total} samples");
    reader.seek(Time::from_seconds(1.0)).unwrap();
    let mut rest = 0;
    loop {
        let frames = reader.read(&mut chunk).unwrap();
        rest += frames;
        if frames < 4096 {
            break;
        }
    }
    assert!(
        (rest as i64 - 24_000).abs() < 200,
        "{rest} samples after 1 s"
    );
    let peaks = waveform(&path, 0, info.duration, 10).unwrap();
    assert!(peaks.iter().all(|peak| *peak > 0.1), "{peaks:?}");
    assert!(AudioReader::open(&path, 1).is_err());
}

#[test]
fn playback_copies_keep_every_frame_at_its_time() {
    let directory = tempfile::tempdir().unwrap();
    let source = counting_clip(directory.path(), "mkv");
    let copy = directory.path().join("copy.mkv");
    let mut heard = Vec::new();
    make_playback_copy(&source, &copy, &mut |share| {
        heard.push(share);
        true
    })
    .unwrap();
    assert_eq!(heard.last(), Some(&1.0));
    let info = probe(&copy).unwrap();
    let video = info.video.unwrap();
    assert_eq!((video.codec.as_str(), video.frames), ("mpeg2video", 60));
    assert_eq!(video.rate, FrameRate::new(30000, 1001));
    let mut reader = FrameReader::open(&copy).unwrap();
    for wanted in [0, 59, 7, 8, 9, 31, 30, 1] {
        let frame = reader.frame_at(video.rate.time_of(wanted), None).unwrap();
        assert_eq!(frame.index, wanted);
        // Lossy, but each frame is still its own.
        assert!(
            (frame_number(&frame) - wanted).abs() <= 1,
            "copy frame {wanted}"
        );
    }
    // Stopping leaves nothing behind.
    let stopped = directory.path().join("stopped.mkv");
    assert!(make_playback_copy(&source, &stopped, &mut |_| false).is_err());
    assert!(!stopped.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
}
