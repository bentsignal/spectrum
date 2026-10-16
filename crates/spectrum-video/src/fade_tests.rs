use super::*;
use spectrum_library::AssetId;
use std::sync::Arc;

const RATE: FrameRate = FrameRate::new(30, 1);

fn frames(n: i64) -> Time {
    RATE.time_of(n)
}

/// A clip of its own asset on V1 at `at` for `length` frames (of a long
/// source), with linked sound.
fn clip(video: &mut Video, at: i64, length: i64) -> (u64, u64) {
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
            source_in: frames(30),
            duration: frames(length),
            source_duration: Some(frames(3000)),
            name: "Clip".into(),
            placement: Placement::Overwrite,
            sound: true,
            sound_track: Some(sound),
        },
    )
    .unwrap();
    (out.clip_ids[0], out.clip_ids[1])
}

/// Each footage asset draws its own flat color, in the order made.
struct Colors(Vec<AssetId>);

impl render::Pictures for Colors {
    fn picture(
        &mut self,
        clip: &Clip,
        _: f64,
        (width, height): (u32, u32),
        _: render::Quality,
    ) -> anyhow::Result<Option<Arc<image::RgbaImage>>> {
        let asset = clip.source.asset();
        let index = match self.0.iter().position(|a| *a == asset) {
            Some(index) => index,
            None => {
                self.0.push(asset);
                self.0.len() - 1
            }
        };
        let color = if index == 0 {
            [200, 0, 0, 255]
        } else {
            [0, 0, 200, 255]
        };
        Ok(Some(Arc::new(image::RgbaImage::from_pixel(
            width,
            height,
            image::Rgba(color),
        ))))
    }
}

/// Every clip sounds as a steady level: the first 0.5, the second 0.25.
struct Levels(Vec<AssetId>);

impl render::Sounds for Levels {
    fn mix(&mut self, clip: &Clip, _: Time, gain: f32, out: &mut [f32]) -> anyhow::Result<()> {
        let asset = clip.source.asset();
        if !self.0.contains(&asset) {
            self.0.push(asset);
        }
        let level = if self.0[0] == asset { 0.5 } else { 0.25 };
        for sample in out {
            *sample += level * gain;
        }
        Ok(())
    }
}

#[test]
fn a_fade_in_rises_from_the_background() {
    let mut video = Video::new("Edit", 8, 8, RATE);
    let (a, _) = clip(&mut video, 0, 60);
    apply(
        &mut video,
        Command::SetFades {
            clip: a,
            fade_in: Some(frames(30)),
            fade_out: None,
        },
    )
    .unwrap();
    let mut colors = Colors(vec![]);
    let at = |video: &Video, colors: &mut Colors, f: i64| {
        render::frame(video, frames(f), (8, 8), colors, render::Quality::Final)
            .unwrap()
            .get_pixel(4, 4)
            .0[0]
    };
    assert_eq!(at(&video, &mut colors, 0), 0, "black at the start");
    let middle = at(&video, &mut colors, 15);
    assert!(middle > 70 && middle < 130, "half way: {middle}");
    assert_eq!(at(&video, &mut colors, 40), 200, "whole after the fade");
    // Sound fades with it.
    let mut out = vec![0f32; 2];
    render::sound(&video, frames(0), &mut out, &mut Levels(vec![])).unwrap();
    assert!(out[0].abs() < 0.01);
}

#[test]
fn crossing_a_neighbor_makes_a_crossfade() {
    let mut video = Video::new("Edit", 8, 8, RATE);
    let (a, _) = clip(&mut video, 0, 60);
    let (b, b_sound) = clip(&mut video, 90, 60);
    // B dragged 40 frames earlier, across A's last 10.
    apply(
        &mut video,
        Command::Move {
            clips: vec![b],
            by: frames(-40),
            tracks: 0,
            crossfade: true,
        },
    )
    .unwrap();
    let (_, moved) = video.clip(b).unwrap();
    assert_eq!(
        (moved.start, moved.fade_in, moved.crossfade),
        (frames(50), frames(10), true)
    );
    assert_eq!(
        video.clip(a).unwrap().1.end(),
        frames(50),
        "A's tail plays under B"
    );
    assert!(
        video.clip(b_sound).unwrap().1.crossfade,
        "the sound crossfades too"
    );
    // Half way through, both show and both sound.
    let mut colors = Colors(vec![]);
    let mixed = render::frame(
        &video,
        frames(55),
        (8, 8),
        &mut colors,
        render::Quality::Final,
    )
    .unwrap()
    .get_pixel(4, 4)
    .0;
    assert!(mixed[0] > 60 && mixed[2] > 60, "both clips show: {mixed:?}");
    let mut out = vec![0f32; 2];
    render::sound(&video, frames(55), &mut out, &mut Levels(vec![])).unwrap();
    assert!(
        out[0] > 0.3 && out[0] < 0.45,
        "both clips sound: {}",
        out[0]
    );
}

#[test]
fn an_edge_dragged_across_a_neighbor_crossfades_instead_of_stopping() {
    let mut video = Video::new("Edit", 8, 8, RATE);
    let (a, _) = clip(&mut video, 0, 60);
    let (b, _) = clip(&mut video, 60, 60);
    let moved = video
        .trim_with(b, edit::Edge::Start, frames(-20), false, true)
        .unwrap();
    assert_eq!(moved, frames(-20));
    let (_, b_clip) = video.clip(b).unwrap();
    assert_eq!(
        (b_clip.start, b_clip.fade_in, b_clip.crossfade),
        (frames(40), frames(20), true)
    );
    assert_eq!(video.clip(a).unwrap().1.end(), frames(40));
    // Without crossfading the edge stops at the neighbor.
    let mut plain = Video::new("Edit", 8, 8, RATE);
    clip(&mut plain, 0, 60);
    let (c, _) = clip(&mut plain, 60, 60);
    assert_eq!(
        plain
            .trim(c, edit::Edge::Start, frames(-20), false)
            .unwrap(),
        Time::ZERO
    );
    // A split keeps the fade in on the left and the fade out on the right.
    apply(
        &mut video,
        Command::SetFades {
            clip: b,
            fade_in: None,
            fade_out: Some(frames(10)),
        },
    )
    .unwrap();
    let right = video.split(frames(80), &[b]).unwrap()[0];
    let (left, right) = (video.clip(b).unwrap().1, video.clip(right).unwrap().1);
    assert_eq!((left.fade_in, left.fade_out), (frames(20), Time::ZERO));
    assert_eq!(
        (right.fade_in, right.fade_out, right.crossfade),
        (Time::ZERO, frames(10), false)
    );
}
