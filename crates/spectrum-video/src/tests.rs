use super::*;
use spectrum_library::AssetId;

const RATE: FrameRate = FrameRate::new(30, 1);

fn frames(n: i64) -> Time {
    RATE.time_of(n)
}

fn video() -> Video {
    Video::new("Edit", 1920, 1080, RATE)
}

fn footage(video: &mut Video, at: i64, length: i64, source_frames: i64) -> (u64, u64) {
    // Always onto V1 and A1, overwriting what is there.
    let (picture, sound) = (
        video.track_named("V1").unwrap().id,
        video.track_named("A1").unwrap().id,
    );
    let out = apply(
        video,
        Command::AddClip {
            source: Source::Footage {
                asset: AssetId::new_v4(),
            },
            track: Some(picture),
            at: frames(at),
            source_in: Time::ZERO,
            duration: frames(length),
            source_duration: Some(frames(source_frames)),
            name: "Clip".into(),
            placement: Placement::Overwrite,
            sound: true,
            sound_track: Some(sound),
        },
    )
    .unwrap();
    (out.clip_ids[0], out.clip_ids[1])
}

fn span(video: &Video, id: u64) -> (i64, i64, i64) {
    let (_, clip) = video.clip(id).unwrap();
    (
        RATE.frame_at(clip.start),
        RATE.frame_at(clip.duration),
        RATE.frame_at(clip.source_in),
    )
}

#[test]
fn footage_brings_its_sound_linked() {
    let mut video = video();
    let kinds: Vec<_> = video
        .tracks
        .iter()
        .map(|t| (t.name.clone(), t.kind))
        .collect();
    // From the bottom up; the timeline shows them reversed: V2, V1, A1, A2.
    let names: Vec<_> = kinds.iter().map(|k| k.0.as_str()).collect();
    assert_eq!(names, ["A2", "A1", "V1", "V2"]);
    assert_eq!(kinds[1], ("A1".into(), TrackKind::Audio));
    // Asking for more than the footage has gives all of it.
    let (picture, sound) = footage(&mut video, 10, 500, 90);
    assert_eq!(span(&video, picture), (10, 90, 0));
    assert_eq!(span(&video, sound), (10, 90, 0));
    assert_eq!(video.linked(picture).len(), 2);
    assert_eq!(video.track_named("V1").unwrap().clips.len(), 1);
    assert_eq!(video.track_named("A1").unwrap().clips.len(), 1);
    assert_eq!(video.duration(), frames(100));
    // A clip off the frame grid lands on a frame.
    let video_snap = video.snap(Time(frames(5).0 + 7));
    assert_eq!(video_snap, frames(5));
}

#[test]
fn overwrite_cuts_into_clips_and_insert_pushes_them_along() {
    let mut video = video();
    let (a, _) = footage(&mut video, 0, 100, 1000);
    // Overwriting the middle splits A around the new clip.
    footage(&mut video, 40, 20, 1000);
    let starts: Vec<_> = video
        .track_named("V1")
        .unwrap()
        .clips
        .iter()
        .map(|c| RATE.frame_at(c.start))
        .collect();
    assert_eq!(starts, [0, 40, 60]);
    assert_eq!(span(&video, a), (0, 40, 0));
    let right = &video.track_named("V1").unwrap().clips[2];
    assert_eq!(RATE.frame_at(right.source_in), 60);
    // Inserting pushes everything after the point, on every track.
    let v1 = video.track_named("V1").unwrap().id;
    apply(
        &mut video,
        Command::AddClip {
            source: Source::Image {
                asset: AssetId::new_v4(),
            },
            track: Some(v1),
            at: frames(40),
            source_in: Time::ZERO,
            duration: frames(30),
            source_duration: None,
            name: "Still".into(),
            placement: Placement::Insert,
            sound: true,
            sound_track: None,
        },
    )
    .unwrap();
    let starts: Vec<_> = video
        .track_named("V1")
        .unwrap()
        .clips
        .iter()
        .map(|c| RATE.frame_at(c.start))
        .collect();
    assert_eq!(starts, [0, 40, 70, 90]);
    let sound: Vec<_> = video
        .track_named("A1")
        .unwrap()
        .clips
        .iter()
        .map(|c| RATE.frame_at(c.start))
        .collect();
    assert_eq!(sound, [0, 70, 90]);
}

#[test]
fn moving_takes_linked_sound_and_overwrites_where_it_lands() {
    let mut video = video();
    let (a, a_sound) = footage(&mut video, 0, 30, 300);
    let (b, _) = footage(&mut video, 100, 30, 300);
    video.move_clips(&[a], frames(90), 1).unwrap();
    // The picture moved up to V2; its sound stayed on A1, moved in time,
    // and overwrote B's sound where they now overlap.
    assert_eq!(
        video.track_of(a).unwrap(),
        video.track_named("V2").unwrap().id
    );
    assert_eq!(span(&video, a_sound).0, 90);
    let b_sound = video
        .track_named("A1")
        .unwrap()
        .clips
        .iter()
        .find(|c| c.id != a_sound)
        .unwrap();
    assert_eq!(RATE.frame_at(b_sound.start), 120);
    assert_eq!(span(&video, b).0, 100);
    // Nothing moves before the start.
    video.move_clips(&[a], frames(-500), 0).unwrap();
    assert_eq!(span(&video, a).0, 0);
}

#[test]
fn trims_stop_at_the_source_and_neighbors_unless_rippling() {
    let mut video = video();
    let (a, a_sound) = footage(&mut video, 0, 30, 40);
    let (b, _) = footage(&mut video, 30, 30, 300);
    // A cannot grow past its source's end or into B.
    assert_eq!(
        video.trim(a, Edge::End, frames(20), false).unwrap(),
        Time::ZERO
    );
    video.trim(a, Edge::End, frames(-10), false).unwrap();
    assert_eq!(span(&video, a), (0, 20, 0));
    assert_eq!(span(&video, a_sound), (0, 20, 0));
    // Rippling the end back out pushes B along.
    let moved = video.trim(a, Edge::End, frames(30), true).unwrap();
    assert_eq!(moved, frames(20));
    assert_eq!(span(&video, b).0, 50);
    // Trimming the start shows later source in the same place.
    video.trim(b, Edge::Start, frames(5), false).unwrap();
    assert_eq!(span(&video, b), (55, 25, 5));
    // A ripple start trim keeps the clip in place and pulls what follows.
    let (c, _) = footage(&mut video, 90, 10, 300);
    video.trim(b, Edge::Start, frames(5), true).unwrap();
    assert_eq!(span(&video, b), (55, 20, 10));
    assert_eq!(span(&video, c).0, 85);
    // Never shorter than a frame.
    video.trim(c, Edge::End, frames(-100), false).unwrap();
    assert_eq!(span(&video, c).1, 1);
}

#[test]
fn roll_slip_and_slide_move_cuts_and_content() {
    let mut video = video();
    let (a, _) = footage(&mut video, 0, 30, 300);
    let (b, _) = footage(&mut video, 30, 30, 300);
    let (c, _) = footage(&mut video, 60, 30, 300);
    video.roll(a, frames(10)).unwrap();
    assert_eq!(span(&video, a), (0, 40, 0));
    assert_eq!(span(&video, b), (40, 20, 10));
    video.slip(b, frames(15)).unwrap();
    assert_eq!(span(&video, b), (40, 20, 25));
    video.slip(b, frames(-100)).unwrap();
    assert_eq!(span(&video, b).2, 0);
    video.slide(b, frames(5)).unwrap();
    assert_eq!(span(&video, a).1, 45);
    assert_eq!(span(&video, b), (45, 20, 0));
    assert_eq!(span(&video, c), (65, 25, 5));
    // The roll stops where the clip after runs out of frames before it.
    let by = video.roll(b, frames(-100)).unwrap();
    assert_eq!(by, frames(-5));
}

#[test]
fn razor_and_ripple_delete() {
    let mut video = video();
    let (a, a_sound) = footage(&mut video, 0, 60, 300);
    let (b, _) = footage(&mut video, 60, 30, 300);
    let made = video.split(frames(20), &[]).unwrap();
    assert_eq!(made.len(), 2);
    assert_eq!(span(&video, a).1, 20);
    assert_eq!(span(&video, a_sound).1, 20);
    // The right halves stay linked to each other, not to the left halves.
    assert_eq!(video.linked(made[0]).len(), 2);
    assert!(!video.linked(made[0]).contains(&a));
    assert_eq!(span(&video, made[0]), (20, 40, 20));
    video.delete(&[made[0]], true).unwrap();
    assert_eq!(span(&video, b).0, 20);
    assert_eq!(video.track_named("A1").unwrap().clips.len(), 2);
    video.delete(&[a], false).unwrap();
    assert_eq!(span(&video, b).0, 20);
    assert!(video.split(frames(1000), &[]).unwrap().is_empty());
}

#[test]
fn new_clips_go_on_a_free_track() {
    let mut video = video();
    let added = |video: &mut Video, at: i64| {
        apply(
            video,
            Command::AddClip {
                source: Source::Footage {
                    asset: AssetId::new_v4(),
                },
                track: None,
                at: frames(at),
                source_in: Time::ZERO,
                duration: frames(30),
                source_duration: None,
                name: "Clip".into(),
                placement: Placement::Overwrite,
                sound: true,
                sound_track: None,
            },
        )
        .unwrap()
        .clip_ids
    };
    let first = added(&mut video, 0);
    let second = added(&mut video, 10);
    let third = added(&mut video, 20);
    let tracks: Vec<_> = [first[0], second[0], third[0], third[1]]
        .iter()
        .map(|id| video.clip(*id).unwrap().0.name.clone())
        .collect();
    // V1, then V2, then a new V3; their sound likewise.
    assert_eq!(tracks, ["V1", "V2", "V3", "A3"]);
    assert_eq!(video.tracks.len(), 6);
}

#[test]
fn locked_tracks_do_not_change() {
    let mut video = video();
    let (a, _) = footage(&mut video, 0, 30, 300);
    let audio = video.track_named("A1").unwrap().id;
    apply(
        &mut video,
        Command::SetTrack {
            track: audio,
            name: None,
            muted: None,
            locked: Some(true),
        },
    )
    .unwrap();
    let moved = Command::Move {
        clips: vec![a],
        by: frames(5),
        tracks: 0,
        crossfade: false,
    };
    assert!(apply(&mut video, moved).is_err());
    let deleted = Command::Delete {
        clips: vec![a],
        ripple: false,
    };
    assert!(apply(&mut video, deleted).is_err());
    assert_eq!(span(&video, a).0, 0);
}

#[test]
fn edits_are_revisions() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("edit.spectrum");
    let person = spectrum_document::Actor {
        id: "person:test".into(),
        display_name: "Tester".into(),
        kind: spectrum_document::ActorKind::Human,
    };
    let session = spectrum_document::SessionId::new();
    let mut workspace = Workspace::create(&path, video(), person.clone(), session).unwrap();
    let mut picture = video();
    let (a, _) = footage(&mut picture, 0, 30, 300);
    let added = workspace
        .execute(Command::AddClip {
            source: picture.clip(a).unwrap().1.source,
            track: None,
            at: frames(0),
            source_in: Time::ZERO,
            duration: frames(30),
            source_duration: Some(frames(300)),
            name: "Clip".into(),
            placement: Placement::Overwrite,
            sound: true,
            sound_track: None,
        })
        .unwrap();
    let id = added.clip_ids[0];
    workspace
        .execute(Command::Trim {
            clip: id,
            edge: Edge::End,
            by: frames(-10),
            ripple: false,
            crossfade: false,
        })
        .unwrap();
    workspace.execute(Command::Undo).unwrap();
    drop(workspace);
    let reopened = Workspace::open(&path, person, session).unwrap();
    assert_eq!(
        RATE.frame_at(reopened.document.clip(id).unwrap().1.duration),
        30
    );
    assert_eq!(reopened.document.uses().len(), 2);
    let json = serde_json::to_string(&Command::Split {
        at: frames(3),
        clips: vec![],
    })
    .unwrap();
    assert!(json.contains("\"command\":\"split\""));
}

struct Solid;

impl render::Pictures for Solid {
    fn picture(
        &mut self,
        clip: &Clip,
        _: f64,
        (width, height): (u32, u32),
        _: render::Quality,
    ) -> anyhow::Result<Option<std::sync::Arc<image::RgbaImage>>> {
        // Footage fills the frame in red; stills are a small blue square.
        Ok(Some(std::sync::Arc::new(match clip.source {
            Source::Footage { .. } => {
                image::RgbaImage::from_pixel(width, height, image::Rgba([200, 0, 0, 255]))
            }
            _ => image::RgbaImage::from_pixel(width / 2, height / 2, image::Rgba([0, 0, 200, 255])),
        })))
    }
}

struct Tone;

impl render::Sounds for Tone {
    fn mix(&mut self, _: &Clip, _: Time, gain: f32, out: &mut [f32]) -> anyhow::Result<()> {
        for sample in out {
            *sample += 0.25 * gain;
        }
        Ok(())
    }
}

#[test]
fn frames_stack_tracks_and_sound_mixes_clips() {
    let mut video = Video::new("Edit", 40, 20, RATE);
    footage(&mut video, 0, 30, 300);
    let still = video.track_named("V2").unwrap().id;
    apply(
        &mut video,
        Command::AddClip {
            source: Source::Image {
                asset: AssetId::new_v4(),
            },
            track: Some(still),
            at: frames(10),
            source_in: Time::ZERO,
            duration: frames(10),
            source_duration: None,
            name: "Still".into(),
            placement: Placement::Overwrite,
            sound: true,
            sound_track: None,
        },
    )
    .unwrap();
    let alone = render::frame(
        &video,
        frames(5),
        (40, 20),
        &mut Solid,
        render::Quality::Final,
    )
    .unwrap();
    assert_eq!(alone.get_pixel(0, 0).0, [200, 0, 0, 255]);
    let stacked = render::frame(
        &video,
        frames(12),
        (40, 20),
        &mut Solid,
        render::Quality::Final,
    )
    .unwrap();
    assert_eq!(stacked.get_pixel(20, 10).0, [0, 0, 200, 255]);
    assert_eq!(stacked.get_pixel(1, 1).0, [200, 0, 0, 255]);
    let empty = render::frame(
        &video,
        frames(40),
        (40, 20),
        &mut Solid,
        render::Quality::Final,
    )
    .unwrap();
    assert_eq!(empty.get_pixel(1, 1).0, [0, 0, 0, 255]);
    // The sound plays where its clip is and stops after.
    let mut out = vec![0f32; 48_000 * 2];
    render::sound(&video, frames(15), &mut out, &mut Tone).unwrap();
    assert_eq!(out[0], 0.25);
    assert_eq!(out[out.len() - 1], 0.);
}

fn velocity(keys: &[(f64, f64, Curve)]) -> Speed {
    Speed::Velocity {
        keys: keys
            .iter()
            .map(|&(at, percent, curve)| VelocityKey {
                at: Time::from_seconds(at),
                percent,
                curve,
            })
            .collect(),
        origin: Time::ZERO,
    }
}

#[test]
fn velocity_integrates_exactly_for_every_curve() {
    for curve in [
        Curve::Linear,
        Curve::Fast,
        Curve::Slow,
        Curve::Smooth,
        Curve::Sharp,
        Curve::Hold,
    ] {
        let speed = velocity(&[(0., 50., curve), (1., 400., curve), (2.5, -100., curve)]);
        // A fine numeric sum of the velocity curve.
        let mut numeric = 0.;
        let step = 1e-4;
        let mut t = 0.;
        while t < 3. {
            numeric += speed.rate(Time::from_seconds(t + step / 2.)) * step;
            t += step;
        }
        let exact = speed.source_seconds(Time::from_seconds(3.));
        assert!(
            (numeric - exact).abs() < 2e-3,
            "{curve:?}: {numeric} vs {exact}"
        );
    }
    // Constant speeds, reverse, and freeze.
    assert!(
        (velocity(&[(0., 50., Curve::Linear)]).source_seconds(Time::from_seconds(2.)) - 1.).abs()
            < 1e-9
    );
    assert!(
        (velocity(&[(0., -100., Curve::Linear)]).source_seconds(Time::from_seconds(2.)) + 2.).abs()
            < 1e-9
    );
    assert_eq!(
        velocity(&[(0., 0., Curve::Linear)]).source_seconds(Time::from_seconds(2.)),
        0.
    );
    assert!(
        Speed::Velocity {
            keys: vec![],
            origin: Time::ZERO
        }
        .checked()
        .is_err()
    );
    assert!(velocity(&[(0., 2000., Curve::Linear)]).checked().is_err());
}

#[test]
fn sped_clips_repeat_and_keep_their_frames_through_edits() {
    let mut video = video();
    let (a, _) = footage(&mut video, 0, 60, 60);
    // Twice as fast: the source runs out halfway and repeats, and the
    // clip can be lengthened past its source's end.
    apply(
        &mut video,
        Command::SetSpeed {
            clip: a,
            speed: velocity(&[(0., 200., Curve::Linear)]),
        },
    )
    .unwrap();
    video.trim(a, Edge::End, frames(60), false).unwrap();
    let clip = video.clip(a).unwrap().1.clone();
    assert_eq!(
        clip.duration,
        frames(120),
        "speed never changes a clip's length"
    );
    assert!((clip.source_seconds(frames(15)) - 1.).abs() < 1e-6);
    assert!(
        (clip.source_seconds(frames(45)) - 1.).abs() < 1e-6,
        "repeats past the end"
    );
    // Backward from the start wraps to the end.
    apply(
        &mut video,
        Command::SetSpeed {
            clip: a,
            speed: velocity(&[(0., -100., Curve::Linear)]),
        },
    )
    .unwrap();
    let clip = video.clip(a).unwrap().1.clone();
    assert!((clip.source_seconds(frames(15)) - 1.5).abs() < 1e-6);
    // A ramp, then cut and trimmed: every frame stays where it was.
    let ramp = velocity(&[
        (0., 100., Curve::Smooth),
        (1., 25., Curve::Smooth),
        (3., 300., Curve::Linear),
    ]);
    apply(
        &mut video,
        Command::SetSpeed {
            clip: a,
            speed: ramp,
        },
    )
    .unwrap();
    let before: Vec<f64> = (40..120)
        .map(|f| video.clip(a).unwrap().1.source_seconds(frames(f)))
        .collect();
    let right = video.split(frames(50), &[a]).unwrap()[0];
    video.trim(right, Edge::Start, frames(10), false).unwrap();
    let after: Vec<f64> = (60..120)
        .map(|f| video.clip(right).unwrap().1.source_seconds(frames(f)))
        .collect();
    for (shown, was) in after.iter().zip(&before[20..]) {
        assert!((shown - was).abs() < 1e-4, "{shown} vs {was}");
    }
    // Time remap: a freeze, then backward.
    let remap = Speed::Remap {
        keys: vec![
            RemapKey {
                at: Time::ZERO,
                source: Time::from_seconds(1.),
                curve: Curve::Hold,
            },
            RemapKey {
                at: Time::from_seconds(1.),
                source: Time::from_seconds(1.),
                curve: Curve::Linear,
            },
            RemapKey {
                at: Time::from_seconds(2.),
                source: Time::ZERO,
                curve: Curve::Linear,
            },
        ],
        origin: Time::ZERO,
    };
    assert!((remap.source_seconds(Time::from_seconds(0.5)) - 1.).abs() < 1e-9);
    assert!((remap.source_seconds(Time::from_seconds(1.5)) - 0.5).abs() < 1e-9);
    assert!(remap.rate(Time::from_seconds(1.5)) < -0.9);
}

/// The source seconds a clip shows on each of its frames, wrapped into
/// the footage.
fn shown(video: &Video, clip: u64) -> Vec<f64> {
    let clip = video.clip(clip).unwrap().1.clone();
    let first = RATE.frame_at(clip.start);
    (first..first + RATE.frame_at(clip.duration))
        .map(|f| clip.source_seconds(frames(f)))
        .collect()
}

fn close(a: f64, b: f64, length: f64) -> bool {
    let apart = (a - b).rem_euclid(length);
    apart.min(length - apart) < 1e-4
}

#[test]
fn reverse_plays_the_same_frames_backward_ramps_and_all() {
    let remap = |keys: &[(f64, f64, Curve)]| Speed::Remap {
        keys: keys
            .iter()
            .map(|&(at, source, curve)| RemapKey {
                at: Time::from_seconds(at),
                source: Time::from_seconds(source),
                curve,
            })
            .collect(),
        origin: Time::ZERO,
    };
    let speeds = [
        Speed::Normal,
        velocity(&[
            (0., 100., Curve::Fast),
            (1., 25., Curve::Hold),
            (2., 300., Curve::Slow),
            (4., -50., Curve::Sharp),
        ]),
        remap(&[
            (0., 1., Curve::Hold),
            (1., 1., Curve::Fast),
            (2., 3., Curve::Smooth),
            (3., 0.5, Curve::Linear),
        ]),
    ];
    for speed in speeds {
        let mut video = video();
        let (a, sound) = footage(&mut video, 0, 120, 300);
        apply(
            &mut video,
            Command::SetSpeed {
                clip: a,
                speed: speed.clone(),
            },
        )
        .unwrap();
        // Trimmed, so keyframes lie before the clip's start.
        video.trim(a, Edge::Start, frames(12), false).unwrap();
        let before = shown(&video, a);
        apply(&mut video, Command::Reverse { clip: a }).unwrap();
        let after = shown(&video, a);
        assert_eq!(shown(&video, sound), after, "the sound reverses with it");
        let length = 10.;
        for (index, value) in after.iter().enumerate() {
            let was = before[before.len() - 1 - index];
            assert!(
                close(*value, was, length),
                "{speed:?} frame {index}: {value} vs {was}"
            );
        }
        // Twice is the clip as it was.
        apply(&mut video, Command::Reverse { clip: a }).unwrap();
        for (now, was) in shown(&video, a).iter().zip(&before) {
            assert!(close(*now, *was, length), "{speed:?}: {now} vs {was}");
        }
    }
}

#[test]
fn freezing_holds_a_frame_then_plays_on() {
    let ramp = velocity(&[(0., 100., Curve::Linear), (2., 50., Curve::Linear)]);
    let remap = Speed::Remap {
        keys: vec![
            RemapKey {
                at: Time::ZERO,
                source: Time::from_seconds(2.),
                curve: Curve::Linear,
            },
            RemapKey {
                at: Time::from_seconds(4.),
                source: Time::ZERO,
                curve: Curve::Linear,
            },
        ],
        origin: Time::ZERO,
    };
    for speed in [Speed::Normal, ramp, remap] {
        let mut video = video();
        let (a, _) = footage(&mut video, 0, 120, 300);
        apply(
            &mut video,
            Command::SetSpeed {
                clip: a,
                speed: speed.clone(),
            },
        )
        .unwrap();
        let before = shown(&video, a);
        apply(
            &mut video,
            Command::Freeze {
                clip: a,
                at: frames(30),
                length: Some(frames(20)),
            },
        )
        .unwrap();
        let held = shown(&video, a);
        assert_eq!(held.len(), 120, "a freeze keeps the clip's length");
        for f in 0..120 {
            let expected = match f {
                ..30 => before[f],
                30..50 => before[30],
                _ => before[f - 20],
            };
            assert!(
                (held[f] - expected).abs() < 1e-4,
                "{speed:?} frame {f}: {} vs {expected}",
                held[f]
            );
        }
        // Held to the end.
        apply(
            &mut video,
            Command::SetSpeed {
                clip: a,
                speed: speed.clone(),
            },
        )
        .unwrap();
        apply(
            &mut video,
            Command::Freeze {
                clip: a,
                at: frames(30),
                length: None,
            },
        )
        .unwrap();
        let held = shown(&video, a);
        assert!(held[30..].iter().all(|v| (v - before[30]).abs() < 1e-4));
        assert!(
            apply(
                &mut video,
                Command::Freeze {
                    clip: a,
                    at: frames(500),
                    length: None,
                },
            )
            .is_err(),
            "only where the clip is"
        );
    }
}

#[test]
fn tracks_move_anywhere_and_sound_moves_between_sound_tracks() {
    let mut video = video();
    let (picture, sound) = footage(&mut video, 0, 30, 90);
    // V1 to the very top: in front of V2, above the sound.
    let v1 = video.track_named("V1").unwrap().id;
    apply(&mut video, Command::MoveTrack { track: v1, to: 99 }).unwrap();
    assert_eq!(video.tracks.last().unwrap().id, v1);
    // Audio between the video tracks.
    let a2 = video.track_named("A2").unwrap().id;
    apply(&mut video, Command::MoveTrack { track: a2, to: 2 }).unwrap();
    let names: Vec<_> = video.tracks.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["A1", "V2", "A2", "V1"]);
    // Sound on its own moves up to the next sound track.
    apply(
        &mut video,
        Command::Unlink {
            clips: vec![picture],
        },
    )
    .unwrap();
    apply(
        &mut video,
        Command::Move {
            clips: vec![sound],
            by: Time::ZERO,
            tracks: 1,
            crossfade: false,
        },
    )
    .unwrap();
    assert_eq!(video.track_of(sound).unwrap(), a2);
    assert!(apply(&mut video, Command::MoveTrack { track: 999, to: 0 }).is_err());
}

#[test]
fn the_background_shows_where_no_clip_covers() {
    let mut video = Video::new("Edit", 40, 20, RATE);
    apply(
        &mut video,
        Command::SetBackground {
            color: [10, 200, 30],
        },
    )
    .unwrap();
    let empty = render::frame(
        &video,
        Time::ZERO,
        (40, 20),
        &mut Solid,
        render::Quality::Final,
    )
    .unwrap();
    assert_eq!(empty.get_pixel(3, 3).0, [10, 200, 30, 255]);
    // A still covers a quarter; the background shows around it.
    let v1 = video.track_named("V1").unwrap().id;
    apply(
        &mut video,
        Command::AddClip {
            source: Source::Image {
                asset: AssetId::new_v4(),
            },
            track: Some(v1),
            at: Time::ZERO,
            source_in: Time::ZERO,
            duration: frames(10),
            source_duration: None,
            name: "Still".into(),
            placement: Placement::Overwrite,
            sound: false,
            sound_track: None,
        },
    )
    .unwrap();
    let framed = render::frame(
        &video,
        Time::ZERO,
        (40, 20),
        &mut Solid,
        render::Quality::Final,
    )
    .unwrap();
    assert_eq!(framed.get_pixel(0, 0).0, [10, 200, 30, 255]);
    assert_eq!(framed.get_pixel(20, 10).0, [0, 0, 200, 255]);
}
