//! Where everything on the timeline is, from the top: a grip that sets how
//! tall the timeline is, the ruler, the video tracks (anchored at the
//! divider, the top one in front, scrolling up), the divider, and the
//! audio tracks (anchored below it, scrolling down). As in Premiere, a
//! video track grows from its top edge and an audio track from its bottom
//! edge, away from the divider, so the edge follows the pointer.
use crate::video_state::VideoEditor;
use gpui::*;
use spectrum_media::Time;
use spectrum_video::{Edge, TrackKind, Video};
use std::collections::HashMap;

/// The track names and switches, left of the clips.
pub const HEADER: f32 = 76.;
/// The strip above the ruler that drags the timeline taller or shorter.
pub const GRIP: f32 = 6.;
pub const RULER: f32 = 26.;
/// Where the tracks start.
pub const TOP: f32 = GRIP + RULER;
/// The bar between video and audio tracks.
pub const DIVIDER: f32 = 6.;
pub const VIDEO_ROW: f32 = 54.;
pub const AUDIO_ROW: f32 = 42.;
/// How short and tall a dragged row can be.
pub const ROW_RANGE: (f32, f32) = (30., 240.);
/// How near an edge, in points, counts as grabbing it.
pub const EDGE: f32 = 6.;
/// How tall the timeline starts, and its limits.
pub const PANEL: f32 = 320.;
pub const PANEL_MIN: f32 = TOP + DIVIDER + 2. * AUDIO_ROW;

#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub track: u64,
    pub kind: TrackKind,
    pub top: f32,
    pub height: f32,
}

/// The timeline's parts, in points from its top-left corner.
pub struct Layout {
    pub height: f32,
    /// Each track's row, top to bottom; rows may reach outside their pane.
    pub rows: Vec<Row>,
    /// The video and audio panes, top and bottom.
    pub video: (f32, f32),
    pub audio: (f32, f32),
    /// How far each pane can scroll.
    pub video_room: f32,
    pub audio_room: f32,
}

impl Layout {
    pub fn divider(&self) -> f32 {
        self.video.1
    }

    /// The pane a row is drawn in.
    pub fn pane(&self, kind: TrackKind) -> (f32, f32) {
        match kind {
            TrackKind::Video => self.video,
            TrackKind::Audio => self.audio,
        }
    }

    pub fn row(&self, track: u64) -> Option<Row> {
        self.rows.iter().copied().find(|r| r.track == track)
    }
}

fn row_height(heights: &HashMap<u64, f32>, track: u64, kind: TrackKind) -> f32 {
    heights.get(&track).copied().unwrap_or(match kind {
        TrackKind::Video => VIDEO_ROW,
        TrackKind::Audio => AUDIO_ROW,
    })
}

/// Lays out `doc`'s tracks in a timeline `height` points tall.
pub fn layout(doc: &Video, view: &VideoEditor, height: f32) -> Layout {
    let video_tracks: Vec<_> = doc
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .collect();
    let audio_tracks: Vec<_> = doc
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .collect();
    let content = |tracks: &[&spectrum_video::Track]| -> f32 {
        tracks
            .iter()
            .map(|t| row_height(&view.heights, t.id, t.kind))
            .sum()
    };
    let (video_content, audio_content) = (content(&video_tracks), content(&audio_tracks));
    let room = (height - TOP - DIVIDER).max(0.);
    // Each pane takes its share; at first, in proportion to its tracks.
    let share = view.split.unwrap_or_else(|| {
        (video_content / (video_content + audio_content).max(1.)).clamp(0.3, 0.7)
    });
    let video_height = (room * share).round().clamp(AUDIO_ROW.min(room), room);
    let video = (TOP, TOP + video_height);
    let audio = (video.1 + DIVIDER, height.max(video.1 + DIVIDER));
    let video_room = (video_content - (video.1 - video.0)).max(0.);
    let audio_room = (audio_content - (audio.1 - audio.0)).max(0.);
    let mut rows = Vec::new();
    // Video from the divider up: V1 at the bottom.
    let mut bottom = video.1 + view.video_scroll.clamp(0., video_room);
    for track in &video_tracks {
        let h = row_height(&view.heights, track.id, track.kind);
        rows.push(Row {
            track: track.id,
            kind: track.kind,
            top: bottom - h,
            height: h,
        });
        bottom -= h;
    }
    rows.reverse();
    let mut top = audio.0 - view.audio_scroll.clamp(0., audio_room);
    for track in &audio_tracks {
        let h = row_height(&view.heights, track.id, track.kind);
        rows.push(Row {
            track: track.id,
            kind: track.kind,
            top,
            height: h,
        });
        top += h;
    }
    Layout {
        height,
        rows,
        video,
        audio,
        video_room,
        audio_room,
    }
}

/// What is under a point on the timeline.
pub enum Hit {
    /// The strip above the ruler.
    Grip,
    Ruler,
    /// The bar between video and audio.
    Divider,
    /// A track's name and switches.
    Header {
        track: u64,
    },
    /// The edge that drags a row taller or shorter.
    RowEdge {
        track: u64,
        kind: TrackKind,
        height: f32,
    },
    Clip {
        clip: u64,
        edge: Option<Edge>,
    },
    /// Track space with no clip, or no track.
    Empty {
        track: Option<u64>,
    },
}

impl VideoEditor {
    /// The timeline's layout as it is drawn now.
    pub fn layout(&self) -> Layout {
        self.layout_of(&self.doc)
    }

    /// `doc`'s layout at the height the timeline is drawn.
    pub fn layout_of(&self, doc: &Video) -> Layout {
        let drawn = f32::from(self.timeline.borrow().size.height);
        layout(doc, self, if drawn > 0. { drawn } else { self.panel })
    }

    /// Where `time` is across the timeline.
    pub fn x_of(&self, time: Time) -> f32 {
        HEADER + ((time.seconds() - self.scroll) as f32) * self.zoom
    }

    /// The time at `x` across the timeline.
    pub fn time_at(&self, x: f32) -> Time {
        Time::from_seconds((f64::from((x - HEADER) / self.zoom) + self.scroll).max(0.))
    }

    /// A window point in timeline points.
    pub fn local(&self, position: Point<Pixels>) -> (f32, f32) {
        let bounds = *self.timeline.borrow();
        (
            f32::from(position.x - bounds.origin.x),
            f32::from(position.y - bounds.origin.y),
        )
    }

    pub fn hit(&self, position: Point<Pixels>) -> Hit {
        let (x, y) = self.local(position);
        let layout = self.layout();
        if y < GRIP {
            return Hit::Grip;
        }
        if y < TOP {
            return Hit::Ruler;
        }
        if y >= layout.divider() - 2. && y < layout.divider() + DIVIDER + 2. {
            return Hit::Divider;
        }
        let in_pane = |row: &Row| {
            let (top, bottom) = layout.pane(row.kind);
            y >= top && y < bottom
        };
        let row = layout
            .rows
            .iter()
            .find(|r| in_pane(r) && y >= r.top && y < r.top + r.height);
        if x < HEADER {
            // Video rows grow from their top edge, audio rows from their
            // bottom edge.
            let edge = layout.rows.iter().find(|r| {
                in_pane(r)
                    && match r.kind {
                        TrackKind::Video => (y - r.top).abs() <= 4.,
                        TrackKind::Audio => (y - (r.top + r.height)).abs() <= 4.,
                    }
            });
            if let Some(r) = edge {
                return Hit::RowEdge {
                    track: r.track,
                    kind: r.kind,
                    height: r.height,
                };
            }
            return match row {
                Some(r) => Hit::Header { track: r.track },
                None => Hit::Empty { track: None },
            };
        }
        let Some(row) = row else {
            return Hit::Empty { track: None };
        };
        let Some(track) = self.doc.track(row.track) else {
            return Hit::Empty { track: None };
        };
        for clip in &track.clips {
            let (left, right) = (self.x_of(clip.start), self.x_of(clip.end()));
            if x >= left - EDGE && x <= right + EDGE {
                let edge = if (x - left).abs() <= EDGE {
                    Some(Edge::Start)
                } else if (x - right).abs() <= EDGE {
                    Some(Edge::End)
                } else if x < left || x > right {
                    continue;
                } else {
                    None
                };
                return Hit::Clip {
                    clip: clip.id,
                    edge,
                };
            }
        }
        Hit::Empty {
            track: Some(row.track),
        }
    }

    /// How many rows up from window height `from` to `to`, counting the
    /// rows between them as they are drawn.
    pub fn rows_between(&self, from: f32, to: f32) -> i32 {
        let top = f32::from(self.timeline.borrow().origin.y);
        let rows = self.layout().rows;
        let index = |y: f32| -> f32 {
            let y = y - top;
            let mut index = 0.;
            for row in &rows {
                if y >= row.top + row.height {
                    index += 1.;
                } else {
                    return index + ((y - row.top) / row.height).max(0.);
                }
            }
            index
        };
        (index(from) - index(to)).round() as i32
    }
}
