//! The edits every video tool makes: placing clips by overwriting or
//! inserting, moving them, trimming, rolling, slipping, and sliding their
//! edges, cutting them, and deleting them with or without closing the gap.
//! Linked clips (a picture and its sound) change together.
use anyhow::{Context, Result, bail};
use spectrum_time::Time;

use crate::{Clip, Track, TrackKind, Video};

/// Which end of a clip a trim moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    Start,
    End,
}

impl Clip {
    /// Shortens the clip from the front by `by` (negative lengthens it): it
    /// starts later and further into its source, showing the same frames
    /// at the same times.
    pub(crate) fn trim_front(&mut self, by: Time) {
        self.start = self.start + by;
        self.duration = self.duration - by;
        // A clip with a speed keeps its keyframes where they were on the
        // timeline.
        self.source_in = self.source_in + self.speed.shift(by);
    }

    /// Cuts the clip at `time`, keeping the left part and returning the
    /// right one under `id`.
    pub(crate) fn cut(&mut self, time: Time, id: u64) -> Clip {
        let mut right = self.clone();
        right.id = id;
        right.trim_front(time - self.start);
        self.duration = time - self.start;
        right
    }
}

impl Track {
    pub(crate) fn place(&mut self, clip: Clip) {
        let at = self.clips.partition_point(|c| c.start < clip.start);
        self.clips.insert(at, clip);
    }

    /// Makes room for `[from, to)`: clips there are removed, shortened, or
    /// cut in two, except those in `keep`.
    pub(crate) fn clear(
        &mut self,
        from: Time,
        to: Time,
        keep: &[u64],
        ids: &mut dyn FnMut() -> u64,
    ) {
        let mut kept = Vec::with_capacity(self.clips.len());
        for mut clip in std::mem::take(&mut self.clips) {
            if keep.contains(&clip.id) || clip.end() <= from || clip.start >= to {
                kept.push(clip);
            } else if clip.start < from && clip.end() > to {
                let mut right = clip.cut(to, ids());
                right.link = None;
                clip.duration = from - clip.start;
                kept.push(clip);
                kept.push(right);
            } else if clip.start < from {
                clip.duration = from - clip.start;
                kept.push(clip);
            } else if clip.end() > to {
                clip.trim_front(to - clip.start);
                kept.push(clip);
            }
        }
        kept.sort_by_key(|c| c.start);
        self.clips = kept;
    }

    /// Moves every clip starting at or after `from` by `by`.
    pub(crate) fn shift(&mut self, from: Time, by: Time) {
        for clip in &mut self.clips {
            if clip.start >= from {
                clip.start = clip.start + by;
            }
        }
    }

    /// The clip ending exactly where `clip` starts, and the one starting
    /// exactly where it ends.
    fn neighbors(&self, clip: &Clip) -> (Option<&Clip>, Option<&Clip>) {
        let before = self
            .clips
            .iter()
            .find(|c| c.end() == clip.start && c.id != clip.id);
        let after = self
            .clips
            .iter()
            .find(|c| c.start == clip.end() && c.id != clip.id);
        (before, after)
    }

    fn gap_before(&self, clip: &Clip) -> Time {
        let previous = self
            .clips
            .iter()
            .filter(|c| c.end() <= clip.start && c.id != clip.id)
            .map(Clip::end)
            .max()
            .unwrap_or(Time::ZERO);
        clip.start - previous
    }

    fn gap_after(&self, clip: &Clip) -> Option<Time> {
        self.clips
            .iter()
            .filter(|c| c.start >= clip.end() && c.id != clip.id)
            .map(|c| c.start - clip.end())
            .min()
    }
}

impl Video {
    fn frame(&self) -> Time {
        self.rate.frame_duration()
    }

    fn unlocked(&self, track: u64) -> Result<()> {
        let track = self.track(track).context("no such track")?;
        if track.locked {
            bail!("{} is locked", track.name);
        }
        Ok(())
    }

    /// The track a clip is on.
    pub(crate) fn track_of(&self, clip: u64) -> Result<u64> {
        Ok(self.clip(clip).context("no such clip")?.0.id)
    }

    /// The clips with all their linked clips, each once.
    pub(crate) fn with_links(&self, clips: &[u64]) -> Vec<u64> {
        let mut all = Vec::new();
        for clip in clips {
            for id in self.linked(*clip) {
                if !all.contains(&id) {
                    all.push(id);
                }
            }
        }
        all
    }

    /// Places `clip` on `track` at its start, overwriting what is there or
    /// (inserting) pushing every unlocked track's later clips along.
    pub(crate) fn put(&mut self, track: u64, clip: Clip, insert: bool) -> Result<()> {
        self.unlocked(track)?;
        let (from, to) = (clip.start, clip.end());
        let mut next = self.next_id;
        let mut ids = || {
            next += 1;
            next - 1
        };
        if insert {
            for track in self.tracks.iter_mut().filter(|t| !t.locked) {
                // A clip across the insert point is cut there first.
                if let Some(index) = track
                    .clips
                    .iter()
                    .position(|c| c.start < from && c.end() > from)
                {
                    let mut right = track.clips[index].cut(from, ids());
                    right.link = None;
                    track.place(right);
                }
                track.shift(from, to - from);
            }
        } else {
            self.track_mut(track)
                .context("no such track")?
                .clear(from, to, &[], &mut ids);
        }
        self.next_id = next;
        self.track_mut(track).context("no such track")?.place(clip);
        Ok(())
    }

    /// Moves clips (and their linked clips) by `by` in time and `tracks`
    /// tracks up (positive) or down, overwriting what they land on.
    pub fn move_clips(&mut self, clips: &[u64], by: Time, tracks: i32) -> Result<()> {
        let ids = self.with_links(clips);
        let earliest = ids
            .iter()
            .filter_map(|id| self.clip(*id).map(|(_, c)| c.start))
            .min()
            .context("no such clip")?;
        let by = self.snap(by).max(-earliest);
        let mut moving = Vec::new();
        for id in &ids {
            let from = self.track_of(*id)?;
            self.unlocked(from)?;
            let kind = self.track(from).context("no such track")?.kind;
            let same: Vec<u64> = self
                .tracks
                .iter()
                .filter(|t| t.kind == kind)
                .map(|t| t.id)
                .collect();
            let index = same.iter().position(|t| *t == from).unwrap_or(0) as i32;
            // Only pictures move between tracks; sound follows its picture.
            let shift = if kind == TrackKind::Video { tracks } else { 0 };
            let to = same[(index + shift).clamp(0, same.len() as i32 - 1) as usize];
            self.unlocked(to)?;
            let track = self.track_mut(from).context("no such track")?;
            let at = track
                .clips
                .iter()
                .position(|c| c.id == *id)
                .context("no such clip")?;
            let mut clip = track.clips.remove(at);
            clip.start = clip.start + by;
            moving.push((to, clip));
        }
        let mut next = self.next_id;
        let mut fresh = || {
            next += 1;
            next - 1
        };
        for (to, clip) in &moving {
            let track = self.track_mut(*to).context("no such track")?;
            track.clear(clip.start, clip.end(), &ids, &mut fresh);
        }
        self.next_id = next;
        for (to, clip) in moving {
            self.track_mut(to).context("no such track")?.place(clip);
        }
        Ok(())
    }

    /// Moves one edge of a clip (and its linked clips) by `by`. With
    /// `ripple`, later clips on the same tracks move to follow, so no gap
    /// opens or closes; otherwise the edge stops at its neighbors.
    pub fn trim(&mut self, clip: u64, edge: Edge, by: Time, ripple: bool) -> Result<Time> {
        let ids = self.with_links(&[clip]);
        let frame = self.frame();
        let mut by = self.snap(by);
        for id in &ids {
            let (track, c) = self.clip(*id).context("no such clip")?;
            if track.locked {
                bail!("{} is locked", track.name);
            }
            let left = c.source_left();
            match edge {
                Edge::End => {
                    by = by.max(frame - c.duration);
                    if let Some(left) = left {
                        by = by.min(left);
                    }
                    if !ripple && let Some(gap) = track.gap_after(c) {
                        by = by.min(gap);
                    }
                }
                Edge::Start => {
                    by = by.min(c.duration - frame);
                    if !c.source.is_still() && c.speed.is_normal() {
                        by = by.max(-c.source_in);
                    }
                    if !ripple {
                        by = by.max(-track.gap_before(c));
                    }
                }
            }
        }
        for id in &ids {
            let track = self.track_of(*id)?;
            let c = self.clip_mut(*id).context("no such clip")?;
            let end = c.end();
            match (edge, ripple) {
                (Edge::End, _) => c.duration = c.duration + by,
                (Edge::Start, false) => c.trim_front(by),
                (Edge::Start, true) => {
                    // The clip keeps its place; what follows moves instead.
                    c.source_in = c.source_in + c.speed.shift(by);
                    c.duration = c.duration - by;
                }
            }
            if ripple {
                let shift = if edge == Edge::End { by } else { -by };
                let track = self.track_mut(track).context("no such track")?;
                for other in &mut track.clips {
                    if other.id != *id && other.start >= end {
                        other.start = other.start + shift;
                    }
                }
            }
        }
        Ok(by)
    }

    /// Moves the cut between a clip and the one right after it by `by`:
    /// one gets longer as the other gets shorter.
    pub fn roll(&mut self, clip: u64, by: Time) -> Result<Time> {
        let frame = self.frame();
        let mut by = self.snap(by);
        let mut pairs = Vec::new();
        for id in self.with_links(&[clip]) {
            let (track, left) = self.clip(id).context("no such clip")?;
            let Some(right) = track.neighbors(left).1 else {
                continue;
            };
            if track.locked {
                bail!("{} is locked", track.name);
            }
            by = by.max(frame - left.duration).min(right.duration - frame);
            if let Some(more) = left.source_left() {
                by = by.min(more);
            }
            if !right.source.is_still() && right.speed.is_normal() {
                by = by.max(-right.source_in);
            }
            pairs.push((id, right.id));
        }
        if pairs.is_empty() {
            bail!("no clip comes right after this one");
        }
        for (left, right) in pairs {
            let clip = self.clip_mut(left).context("no such clip")?;
            clip.duration = clip.duration + by;
            self.clip_mut(right).context("no such clip")?.trim_front(by);
        }
        Ok(by)
    }

    /// Shows a different part of the source in the same place: the clip's
    /// in point moves by `by`.
    pub fn slip(&mut self, clip: u64, by: Time) -> Result<Time> {
        let mut by = self.snap(by);
        let ids = self.with_links(&[clip]);
        for id in &ids {
            let (track, c) = self.clip(*id).context("no such clip")?;
            if track.locked {
                bail!("{} is locked", track.name);
            }
            if let (Some(left), true) = (c.source_left(), !c.source.is_still()) {
                by = by.max(-c.source_in).min(left);
            }
        }
        for id in ids {
            let c = self.clip_mut(id).context("no such clip")?;
            c.source_in = c.source_in + by;
        }
        Ok(by)
    }

    /// Moves a clip along between its neighbors, which give and take the
    /// time: the one before gets longer as the one after gets shorter.
    pub fn slide(&mut self, clip: u64, by: Time) -> Result<Time> {
        let frame = self.frame();
        let mut by = self.snap(by);
        let mut moves = Vec::new();
        for id in self.with_links(&[clip]) {
            let (track, c) = self.clip(id).context("no such clip")?;
            if track.locked {
                bail!("{} is locked", track.name);
            }
            let (before, after) = track.neighbors(c);
            match before {
                Some(before) => {
                    by = by.max(frame - before.duration);
                    if let Some(more) = before.source_left() {
                        by = by.min(more);
                    }
                }
                None => by = by.max(-track.gap_before(c)),
            }
            match after {
                Some(after) => {
                    by = by.min(after.duration - frame);
                    if !after.source.is_still() && after.speed.is_normal() {
                        by = by.max(-after.source_in);
                    }
                }
                None => {
                    if let Some(gap) = track.gap_after(c) {
                        by = by.min(gap);
                    }
                }
            }
            moves.push((id, before.map(|c| c.id), after.map(|c| c.id)));
        }
        for (id, before, after) in moves {
            if let Some(before) = before {
                let c = self.clip_mut(before).context("no such clip")?;
                c.duration = c.duration + by;
            }
            if let Some(after) = after {
                self.clip_mut(after).context("no such clip")?.trim_front(by);
            }
            let c = self.clip_mut(id).context("no such clip")?;
            c.start = c.start + by;
        }
        Ok(by)
    }

    /// Cuts clips at `at`: the given ones and their linked clips, or every
    /// clip there on unlocked tracks. Returns the new right-hand clips.
    pub fn split(&mut self, at: Time, clips: &[u64]) -> Result<Vec<u64>> {
        let at = self.snap(at);
        let targets: Vec<u64> = if clips.is_empty() {
            self.tracks
                .iter()
                .filter(|t| !t.locked)
                .flat_map(|t| t.clips.iter())
                .filter(|c| c.start < at && at < c.end())
                .map(|c| c.id)
                .collect()
        } else {
            self.with_links(clips)
                .into_iter()
                .filter(|id| {
                    self.clip(*id)
                        .is_some_and(|(t, c)| !t.locked && c.start < at && at < c.end())
                })
                .collect()
        };
        let mut links = std::collections::HashMap::new();
        let mut made = Vec::new();
        for id in targets {
            let track = self.track_of(id)?;
            let new_id = self.take_id();
            let link = self.clip(id).and_then(|(_, c)| c.link);
            let new_link = link.map(|link| *links.entry(link).or_insert_with(|| new_id));
            let clip = self.clip_mut(id).context("no such clip")?;
            let mut right = clip.cut(at, new_id);
            right.link = new_link;
            self.track_mut(track).context("no such track")?.place(right);
            made.push(new_id);
        }
        Ok(made)
    }

    /// Removes clips and their linked clips; with `ripple`, what follows on
    /// their tracks moves back to close the gap.
    pub fn delete(&mut self, clips: &[u64], ripple: bool) -> Result<()> {
        let ids = self.with_links(clips);
        let mut removed = Vec::new();
        for id in &ids {
            let track = self.track_of(*id)?;
            self.unlocked(track)?;
            let track = self.track_mut(track).context("no such track")?;
            let at = track
                .clips
                .iter()
                .position(|c| c.id == *id)
                .context("no such clip")?;
            let clip = track.clips.remove(at);
            removed.push((track.id, clip.start, clip.duration));
        }
        if ripple {
            // Latest first, so earlier gaps close after later ones.
            removed.sort_by_key(|(_, start, _)| std::cmp::Reverse(*start));
            for (track, start, duration) in removed {
                if let Some(track) = self.track_mut(track) {
                    track.shift(start + duration, -duration);
                }
            }
        }
        Ok(())
    }

    /// Links clips so they change together, or unlinks them.
    pub fn link(&mut self, clips: &[u64], linked: bool) -> Result<()> {
        let link = linked.then(|| self.take_id());
        for id in self.with_links(clips) {
            self.clip_mut(id).context("no such clip")?.link = link;
        }
        Ok(())
    }
}
