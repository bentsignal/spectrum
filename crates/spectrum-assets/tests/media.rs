//! Footage and audio in the library: copied into the media store once,
//! with posters, frames, copies, export, and purge.
use spectrum_assets::Service;
use spectrum_library::AssetKind;
use std::{path::Path, process::Command};

fn make(arguments: &[&str], output: &Path) {
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(arguments)
        .arg(output)
        .status()
        .expect("the ffmpeg command makes media fixtures; install FFmpeg");
    assert!(status.success());
}

#[test]
fn footage_and_audio_import_into_the_media_store() {
    let tmp = tempfile::tempdir().unwrap();
    let clip = tmp.path().join("Clip 0001.MP4");
    make(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x180:r=60:d=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
        ],
        &clip,
    );
    let song = tmp.path().join("song.flac");
    make(
        &["-f", "lavfi", "-i", "sine=frequency=220:duration=2"],
        &song,
    );
    let photo = tmp.path().join("photo.png");
    image::RgbaImage::from_pixel(4, 4, image::Rgba([9, 9, 9, 255]))
        .save(&photo)
        .unwrap();

    let root = tmp.path().join("library");
    let mut service = Service::open(&root).unwrap();
    let (batch, assets) = service
        .import_into(vec![clip.clone(), song.clone(), photo], None)
        .unwrap();
    assert_eq!(batch.assets.len(), 3);
    let kinds: Vec<_> = assets.iter().map(|asset| asset.kind).collect();
    assert_eq!(
        kinds,
        [AssetKind::Footage, AssetKind::Audio, AssetKind::Image]
    );
    let (footage, audio) = (&assets[0], &assets[1]);
    assert_eq!(footage.name, "Clip 0001");
    // The card can be wiped: the library has its own copy.
    std::fs::remove_file(&clip).unwrap();
    std::fs::remove_file(&song).unwrap();
    let (media, file) = service.media(footage.id).unwrap();
    assert!(file.starts_with(service.library.root().join("media")));
    assert_eq!(media.original_name, "Clip 0001.MP4");
    assert_eq!(media.info.video.as_ref().unwrap().frames, 60);

    let frame = service
        .footage_frame(
            footage.id,
            spectrum_media::Time::from_seconds(0.5),
            Some(160),
        )
        .unwrap();
    assert_eq!((frame.index, frame.image.width()), (30, 160));
    for asset in [footage, audio] {
        let thumbnail = service.thumbnail(asset.id, 128).unwrap();
        let (width, _) = image::image_dimensions(thumbnail).unwrap();
        assert_eq!(width, 128);
    }
    assert!(
        service
            .footage_frame(audio.id, Default::default(), None)
            .is_err()
    );

    // Small 8-bit footage plays as it is; 10-bit footage gets a copy.
    assert!(!service.needs_playback_copy(footage.id).unwrap());
    assert_eq!(
        service
            .make_playback_copy(footage.id, &mut |_| true)
            .unwrap(),
        None
    );
    let deep = tmp.path().join("deep.mov");
    make(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x180:r=25:d=1",
            "-c:v",
            "libx265",
            "-pix_fmt",
            "yuv422p10le",
            "-x265-params",
            "log-level=error",
        ],
        &deep,
    );
    let deep = service.import(vec![deep]).unwrap().remove(0);
    assert!(service.needs_playback_copy(deep.id).unwrap());
    let copy = service
        .make_playback_copy(deep.id, &mut |_| true)
        .unwrap()
        .unwrap();
    assert!(!service.needs_playback_copy(deep.id).unwrap());
    assert_eq!(service.playback_copy(deep.id).unwrap(), Some(copy.clone()));
    service.delete(deep.id).unwrap();
    service.purge(deep.id).unwrap();
    assert!(!copy.exists(), "a purged file's playback copy goes with it");

    // Copies share the stored file; it goes only with its last asset.
    let copy = service.copy(footage.id).unwrap();
    assert_eq!(service.media(copy.id).unwrap().1, file);
    let exported = tmp.path().join("out.mp4");
    service.export(copy.id, &exported).unwrap();
    assert_eq!(
        std::fs::read(&exported).unwrap(),
        std::fs::read(&file).unwrap()
    );
    assert!(service.export(copy.id, &root.join("inside.mp4")).is_err());
    service.delete(footage.id).unwrap();
    service.purge(footage.id).unwrap();
    assert!(file.exists());
    service.delete(copy.id).unwrap();
    service.purge(copy.id).unwrap();
    assert!(!file.exists());
    let (_, audio_file) = service.media(audio.id).unwrap();
    assert!(audio_file.exists());

    // Something that is neither imports nothing.
    let notes = tmp.path().join("notes.txt");
    std::fs::write(&notes, b"text").unwrap();
    let before = service.library.list().unwrap().len();
    assert!(service.import(vec![notes]).is_err());
    assert_eq!(service.library.list().unwrap().len(), before);
}
