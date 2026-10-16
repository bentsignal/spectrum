//! Where everything on the timeline is, from the top: a grip that sets how
//! tall the timeline is, the ruler, and the tracks, top to bottom in one
//! list that scrolls as one, as in Vegas. Any track can go anywhere (drag
//! its name); video tracks higher in the list are in front. A track grows
//! from its top edge, which follows the pointer.
use crate::video_state::VideoEditor;
use gpui::*;
use spectrum_media::Time;
use spectrum_video::{Edge, TrackKind, Video};
use std::collections::HashMap;

/// The track names and switches, left of the clips.
pub const HEADER: f32 = 76.;
/// The strip above the ruler that drags the timeline taller or shorter.
pub const GRIP: f32 = 10.;
pub const RULER: f32 = 26.;
/// Where the tracks start.
pub const TOP: f32 = GRIP + RULER;
pub const VIDEO_ROW: f32 = 54.;
pub const AUDIO_ROW: f32 = 42.;
/// How short and tall a dragged row can be.
pub const ROW_RANGE: (f32, f32) = (30., 240.);
/// How near an edge, in points, counts as grabbing it.
pub const EDGE: f32 = 6.;
/// How tall the timeline starts, and how short it can get.
pub const PANEL: f32 = 320.;
pub const PANEL_MIN: f32 = TOP + AUDIO_ROW;
/// What the monitor keeps however tall the timeline grows: the header, the
/// transport, and a sliver of picture.
pub const MONITOR_MIN: f32 = crate::workspace::HEADER_HEIGHT + 56. + 48.;

/// The tallest the timeline can be in a window this tall.
pub fn panel_most(window: &Window) -> f32 {
    (f32::from(window.viewport_size().height) - MONITOR_MIN).max(PANEL_MIN)
}

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
    /// Each track's row, top to bottom; rows may reach past the tracks'
    /// area, which clips them.
    pub rows: Vec<Row>,
    /// How far the tracks can scroll.
    pub room: f32,
}

impl Layout {
    pub fn row(&self, track: u64) -> Option<Row> {
        self.rows.iter().copied().find(|r| r.track == track)
    }

    /// Where the tracks' area is, top and bottom.
    pub fn area(&self) -> (f32, f32) {
        (TOP, self.height.max(TOP))
    }

    /// The place a track dropped at timeline height `y` goes, counted from
    /// the bottom track as the document keeps them.
    pub fn place_at(&self, y: f32) -> usize {
        let above = self
            .rows
            .iter()
            .filter(|r| r.top + r.height / 2. < y)
            .count();
        self.rows.len().saturating_sub(above + 1)
    }
}

pub fn row_height(heights: &HashMap<u64, f32>, track: u64, kind: TrackKind) -> f32 {
    heights.get(&track).copied().unwrap_or(match kind {
        TrackKind::Video => VIDEO_ROW,
        TrackKind::Audio => AUDIO_ROW,
    })
}

/// Lays out `doc`'s tracks in a timeline `height` points tall. While a
/// track is dragged by its name, the tracks show in the order they would
/// take if it were let go there.
pub fn layout(doc: &Video, view: &VideoEditor, height: f32) -> Layout {
    let laid = laid_out(doc, view, height);
    let moving = match view.drag.as_ref().map(|d| d.kind) {
        Some(crate::video_state::DragKind::TrackOrder { track }) => track,
        _ => return laid,
    };
    let (Some(y), Some(from)) = (
        view.track_drop,
        doc.tracks.iter().position(|t| t.id == moving),
    ) else {
        return laid;
    };
    let to = laid.place_at(y);
    if to == from {
        return laid;
    }
    let mut moved = doc.clone();
    let track = moved.tracks.remove(from);
    moved.tracks.insert(to.min(moved.tracks.len()), track);
    laid_out(&moved, view, height)
}

fn laid_out(doc: &Video, view: &VideoEditor, height: f32) -> Layout {
    let content: f32 = doc
        .tracks
        .iter()
        .map(|t| row_height(&view.heights, t.id, t.kind))
        .sum();
    // A little room below the last track, to drop onto.
    let room = (content + 24. - (height - TOP)).max(0.);
    let mut top = TOP - view.scroll_y.clamp(0., room);
    let mut rows = Vec::with_capacity(doc.tracks.len());
    // The document keeps tracks from the bottom up.
    for track in doc.tracks.iter().rev() {
        let h = row_height(&view.heights, track.id, track.kind);
        rows.push(Row {
            track: track.id,
            kind: track.kind,
            top,
            height: h,
        });
        top += h;
    }
    Layout { height, rows, room }
}

/// What is under a point on the timeline.
pub enum Hit {
    /// The strip above the ruler.
    Grip,
    Ruler,
    /// In or Out on the ruler, which drags.
    Mark {
        out: bool,
    },
    /// A track's name: dragging it moves the track.
    Header {
        track: u64,
    },
    /// The top edge that drags a row taller or shorter.
    RowEdge {
        track: u64,
        height: f32,
    },
    Clip {
        clip: u64,
        edge: Option<Edge>,
    },
    /// A clip's fade handle at its top corner (or where its fade ends).
    Fade {
        clip: u64,
        out: bool,
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
        // The grip reaches a little into the ruler, so it is easy to catch.
        if y < GRIP + 3. {
            return Hit::Grip;
        }
        if y < TOP {
            let near = |at: Option<Time>| at.is_some_and(|t| (self.x_of(t) - x).abs() <= 5.);
            if near(self.out_point) {
                return Hit::Mark { out: true };
            }
            if near(self.in_point) {
                return Hit::Mark { out: false };
            }
            return Hit::Ruler;
        }
        let row = layout
            .rows
            .iter()
            .find(|r| y >= r.top && y < r.top + r.height);
        if x < HEADER {
            let edge = layout
                .rows
                .iter()
                .find(|r| r.top > TOP - 4. && (y - r.top).abs() <= 4.);
            if let Some(r) = edge {
                return Hit::RowEdge {
                    track: r.track,
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
            // Fade handles sit along the clip's top.
            if x >= left && x <= right && y >= row.top + 2. && y < row.top + 14. {
                let in_x = self.x_of(clip.start + clip.fade_in);
                let out_x = self.x_of(clip.end() - clip.fade_out);
                let near_in = (x - in_x).abs() <= 6. || (clip.fade_in.0 == 0 && x <= left + 12.);
                let near_out =
                    (x - out_x).abs() <= 6. || (clip.fade_out.0 == 0 && x >= right - 12.);
                if near_in && (!near_out || (x - in_x).abs() <= (x - out_x).abs()) {
                    return Hit::Fade {
                        clip: clip.id,
                        out: false,
                    };
                }
                if near_out {
                    return Hit::Fade {
                        clip: clip.id,
                        out: true,
                    };
                }
            }
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
}
