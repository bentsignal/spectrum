//! Videos in the library: built from footage, photos, and canvases, edited
//! through the service, drawn, exported, copied, and indexed.
use spectrum_assets::Service;
use spectrum_library::AssetKind;
use spectrum_media::{ExportSettings, FrameRate, Time, VideoCodec};
use spectrum_video::{Command, Edge, Placement};
use std::{path::Path, process::Command as Process};

fn make(arguments: &[&str], output: &Path) {
    let status = Process::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(arguments)
        .arg(output)
        .status()
        .expect("the ffmpeg command makes media fixtures; install FFmpeg");
    assert!(status.success());
}

#[test]
fn videos_compose_the_library_and_export() {
    let tmp = tempfile::tempdir().unwrap();
    // Two seconds of solid red at 30 fps with a tone.
    let clip = tmp.path().join("red.mp4");
    make(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=320x180:r=30:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
        ],
        &clip,
    );
    let photo = tmp.path().join("blue.png");
    image::RgbaImage::from_pixel(160, 90, image::Rgba([0, 0, 255, 255]))
        .save(&photo)
        .unwrap();
    let root = tmp.path().join("library");
    let mut service = Service::open(&root).unwrap();
    let imported = service.import(vec![clip, photo]).unwrap();
    let (footage, still) = (imported[0].id, imported[1].id);
    let canvas = service.create_canvas("Title".into(), 320, 180).unwrap();

    let rate = FrameRate::new(30, 1);
    let video = service.create_video("Cut", 320, 180, rate).unwrap();
    assert_eq!(video.kind, AssetKind::Video);
    let made = service
        .add_to_video(video.id, footage, Time::ZERO, None, Placement::Overwrite)
        .unwrap();
    assert_eq!(made.len(), 2, "the footage's picture and its sound");
    service
        .add_to_video(
            video.id,
            still,
            rate.time_of(60),
            None,
            Placement::Overwrite,
        )
        .unwrap();
    service
        .add_to_video(
            video.id,
            canvas.id,
            rate.time_of(210),
            None,
            Placement::Overwrite,
        )
        .unwrap();
    // The photo shows for five seconds; trim it to one.
    let edit = service.video(video.id).unwrap();
    let photo_clip = edit.track_named("V1").unwrap().clips[1].id;
    service
        .edit_video(
            video.id,
            vec![Command::Trim {
                clip: photo_clip,
                edge: Edge::End,
                by: rate.time_of(-120),
                ripple: true,
            }],
        )
        .unwrap();
    let edit = service.video(video.id).unwrap();
    assert_eq!(edit.duration(), rate.time_of(240));
    // The canvas moved up behind the trimmed photo.
    assert_eq!(
        edit.track_named("V1").unwrap().clips[2].start,
        rate.time_of(90)
    );

    let red = service
        .video_frame(video.id, rate.time_of(10), (64, 36))
        .unwrap();
    let [r, g, b, _] = red.get_pixel(32, 18).0;
    assert!(r > 200 && g < 40 && b < 40, "{r} {g} {b}");
    let blue = service
        .video_frame(video.id, rate.time_of(75), (64, 36))
        .unwrap();
    assert!(blue.get_pixel(32, 18).0[2] > 200);

    // The library knows what the video uses.
    let users = service.library.dependents(footage).unwrap();
    assert_eq!(users, vec![video.id]);
    assert_eq!(
        service.library.dependents(canvas.id).unwrap(),
        vec![video.id]
    );

    let out = tmp.path().join("cut.mp4");
    let settings = ExportSettings {
        codec: VideoCodec::H264,
        width: 320,
        height: 180,
        rate,
        bitrate: None,
        sound: true,
    };
    let mut shares = Vec::new();
    service
        .export_video(video.id, &out, settings, &mut |share| {
            shares.push(share);
            true
        })
        .unwrap();
    let info = spectrum_media::probe(&out).unwrap();
    assert_eq!(info.video.as_ref().unwrap().frames, 240);
    assert_eq!(info.audio.len(), 1);
    assert_eq!(shares.last(), Some(&1.0));
    assert!(
        service
            .export_video(video.id, &root.join("x.mp4"), settings, &mut |_| true)
            .is_err()
    );

    // A copy gets its own photo and canvas, and shares the footage.
    let copy = service.copy(video.id).unwrap();
    let copied = service.video(copy.id).unwrap();
    let assets: Vec<_> = copied
        .track_named("V1")
        .unwrap()
        .clips
        .iter()
        .map(|c| c.source.asset())
        .collect();
    assert_eq!(assets[0], footage);
    assert_ne!(assets[1], still);
    assert_ne!(assets[2], canvas.id);
    assert!(service.thumbnail(video.id, 128).unwrap().exists());
    assert_eq!(service.history(video.id).unwrap().revisions.len(), 5);
}

#[test]
fn velocity_shows_the_right_source_frames() {
    let tmp = tempfile::tempdir().unwrap();
    // Frame N has luma 16 + 3N, so a frame says which it is.
    let clip = tmp.path().join("count.mkv");
    make(
        &[
            "-f",
            "lavfi",
            "-i",
            "nullsrc=s=64x48:r=30:d=2,geq=lum='16+3*N':cb=128:cr=128",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-c:v",
            "libx264",
            "-qp",
            "0",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
        ],
        &clip,
    );
    let mut service = Service::open(&tmp.path().join("library")).unwrap();
    let footage = service.import(vec![clip]).unwrap()[0].id;
    let rate = FrameRate::new(30, 1);
    let video = service.create_video("Slow", 64, 48, rate).unwrap();
    let made = service
        .add_to_video(video.id, footage, Time::ZERO, None, Placement::Overwrite)
        .unwrap();
    let number = |service: &Service, frame: i64| {
        let image = service
            .video_frame(video.id, rate.time_of(frame), (64, 48))
            .unwrap();
        f64::from(image.get_pixel(32, 24).0[1]) * 219. / 255. / 3.
    };
    let speed = |percent: f64| Command::SetSpeed {
        clip: made[0],
        speed: spectrum_video::Speed::Velocity {
            keys: vec![spectrum_video::VelocityKey {
                at: Time::ZERO,
                percent,
                curve: spectrum_video::Curve::Linear,
            }],
            origin: Time::ZERO,
        },
    };
    service.edit_video(video.id, vec![speed(50.)]).unwrap();
    assert!(
        (number(&service, 20) - 10.).abs() < 0.6,
        "{}",
        number(&service, 20)
    );
    // A quarter speed lands between frames: made from the two nearest.
    service.edit_video(video.id, vec![speed(25.)]).unwrap();
    let between = number(&service, 2);
    assert!(between > 0.2 && between < 0.8, "{between}");
    // The clip keeps its length, and its sound plays slowed down with it.
    let edit = service.video(video.id).unwrap();
    assert_eq!(edit.duration(), rate.time_of(60));
    let out = tmp.path().join("slow.mp4");
    let settings = ExportSettings {
        codec: VideoCodec::H264,
        width: 64,
        height: 48,
        rate,
        bitrate: None,
        sound: true,
    };
    service
        .export_video(video.id, &out, settings, &mut |_| true)
        .unwrap();
    let info = spectrum_media::probe(&out).unwrap();
    assert_eq!(info.video.unwrap().frames, 60);
}
