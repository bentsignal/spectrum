use super::*;
use spectrum_library::AssetId;

const RATE: FrameRate = FrameRate::new(30, 1);

/// A two-second footage clip of a ten-second file, trimmed so its speed's
/// origin is past zero, with `speed`.
fn clip(speed: Speed) -> (Video, u64) {
    let mut video = Video::new("Edit", 1920, 1080, RATE);
    let track = video.track_named("V1").unwrap().id;
    let out = apply(
        &mut video,
        Command::AddClip {
            source: Source::Footage {
                asset: AssetId::new_v4(),
            },
            track: Some(track),
            at: Time::ZERO,
            source_in: Time::from_seconds(1.),
            duration: RATE.time_of(120),
            source_duration: Some(Time::from_seconds(10.)),
            name: "Clip".into(),
            placement: Placement::Overwrite,
            sound: true,
            sound_track: None,
        },
    )
    .unwrap();
    let id = out.clip_ids[0];
    apply(&mut video, Command::SetSpeed { clip: id, speed }).unwrap();
    apply(
        &mut video,
        Command::Trim {
            clip: id,
            edge: Edge::Start,
            by: RATE.time_of(9),
            ripple: false,
            crossfade: false,
        },
    )
    .unwrap();
    (video, id)
}

/// What the clip shows on each frame, wrapped into the footage.
fn shown(video: &Video, id: u64) -> Vec<f64> {
    let clip = video.clip(id).unwrap().1.clone();
    let first = RATE.frame_at(clip.start);
    (first..first + RATE.frame_at(clip.duration))
        .map(|f| clip.source_seconds(RATE.time_of(f)))
        .collect()
}

fn assert_same(before: &[f64], after: &[f64], what: &str) {
    let quarter = RATE.frame_duration().seconds() / 2. + 1e-6;
    assert_eq!(before.len(), after.len());
    for (frame, (was, now)) in before.iter().zip(after).enumerate() {
        let apart = (was - now).rem_euclid(10.);
        let apart = apart.min(10. - apart);
        assert!(apart <= quarter, "{what} frame {frame}: {was} became {now}");
    }
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

fn remap(keys: &[(f64, f64, Curve)]) -> Speed {
    Speed::Remap {
        keys: keys
            .iter()
            .map(|&(at, source, curve)| RemapKey {
                at: Time::from_seconds(at),
                source: Time::from_seconds(source),
                curve,
            })
            .collect(),
        origin: Time::ZERO,
    }
}

#[test]
fn every_speed_keeps_its_frames_in_the_other_view_and_back() {
    let mut speeds = vec![("normal", Speed::Normal)];
    for curve in [
        Curve::Linear,
        Curve::Fast,
        Curve::Slow,
        Curve::Smooth,
        Curve::Sharp,
        Curve::Hold,
    ] {
        speeds.push((
            "velocity",
            velocity(&[
                (0., 100., curve),
                (1., 300., curve),
                (1.6, -80., curve),
                (3., 0., curve),
            ]),
        ));
    }
    for curve in [
        Curve::Linear,
        Curve::Fast,
        Curve::Slow,
        Curve::Smooth,
        Curve::Sharp,
    ] {
        speeds.push((
            "remap",
            remap(&[
                (0., 1., curve),
                (1., 4., curve),
                (2., 2.5, curve),
                (3., 2.5, curve),
            ]),
        ));
    }
    for (name, speed) in speeds {
        let (mut video, id) = clip(speed.clone());
        let before = shown(&video, id);
        for form in [SpeedForm::Remap, SpeedForm::Velocity, SpeedForm::Remap] {
            let out = apply(&mut video, Command::ConvertSpeed { clip: id, to: form }).unwrap();
            assert!(out.ok);
            let now = &video.clip(id).unwrap().1.speed;
            assert_eq!(
                matches!(now, Speed::Remap { .. }),
                form == SpeedForm::Remap,
                "{name} became {form:?}"
            );
            assert_same(
                &before,
                &shown(&video, id),
                &format!("{name} {speed:?} as {form:?}"),
            );
            let count = match now {
                Speed::Velocity { keys, .. } => keys.len(),
                Speed::Remap { keys, .. } => keys.len(),
                Speed::Normal => 0,
            };
            assert!(count <= 64, "{name} as {form:?} took {count} keyframes");
        }
        // The linked sound converted with it.
        let sound = video.linked(id).into_iter().find(|c| *c != id).unwrap();
        assert_eq!(
            video.clip(sound).unwrap().1.speed,
            video.clip(id).unwrap().1.speed
        );
    }
}

#[test]
fn conversion_keeps_the_keyframes_where_they_were() {
    let (mut video, id) = clip(velocity(&[
        (0., 100., Curve::Linear),
        (1., 300., Curve::Linear),
        (2., 300., Curve::Linear),
    ]));
    apply(
        &mut video,
        Command::ConvertSpeed {
            clip: id,
            to: SpeedForm::Remap,
        },
    )
    .unwrap();
    let Speed::Remap { keys, .. } = &video.clip(id).unwrap().1.speed else {
        panic!("remap");
    };
    let times: Vec<f64> = keys.iter().map(|k| k.at.seconds()).collect();
    // The keyframe at 2 s lies on the straight line after 1 s, so it goes.
    for at in [0., 1.] {
        assert!(
            times.iter().any(|t| (t - at).abs() < 1e-6),
            "{at} in {times:?}"
        );
    }
    assert!(
        keys.len() <= 16,
        "only as many keyframes as needed: {}",
        keys.len()
    );
}

#[test]
fn a_remap_jump_is_reported_inexact_in_velocity() {
    let (mut video, id) = clip(remap(&[(0., 1., Curve::Hold), (1., 5., Curve::Linear)]));
    let out = apply(
        &mut video,
        Command::ConvertSpeed {
            clip: id,
            to: SpeedForm::Velocity,
        },
    )
    .unwrap();
    assert!(out.message.contains("nearest"), "{}", out.message);
}
