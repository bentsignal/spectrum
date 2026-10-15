//! The open video: its edit as shown, the edits waiting to save, the
//! selection, the timeline's view, and the program monitor. Every change is
//! a `spectrum_video::Command`, applied on screen at once and saved in
//! batches after a pause, as the canvas editor does.
use crate::{
    audio_out::AudioOut,
    player::{Player, Timing},
    player_decode,
    video_program::{ProgramPictures, ProgramSound, frames},
    workspace::Workspace,
};
use gpui::*;
use spectrum_assets::Service;
use spectrum_document::RevisionId;
use spectrum_library::AssetId;
use spectrum_media::Time;
use spectrum_video::{Command, Video};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

/// The timeline's tools, with Premiere's keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// V: select, move, and trim.
    Select,
    /// C: cut clips where clicked.
    Razor,
    /// B: trims that move what follows.
    Ripple,
    /// N: move the cut between two clips.
    Roll,
    /// Y: show another part of a clip's source in place.
    Slip,
    /// U: move a clip between its neighbors.
    Slide,
}

impl Tool {
    pub const ALL: [Tool; 6] = [
        Tool::Select,
        Tool::Razor,
        Tool::Ripple,
        Tool::Roll,
        Tool::Slip,
        Tool::Slide,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Razor => "Razor",
            Tool::Ripple => "Ripple edit",
            Tool::Roll => "Rolling edit",
            Tool::Slip => "Slip",
            Tool::Slide => "Slide",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tool::Select => "v",
            Tool::Razor => "c",
            Tool::Ripple => "b",
            Tool::Roll => "n",
            Tool::Slip => "y",
            Tool::Slide => "u",
        }
    }
}

/// A drag on the timeline: the edit as it was when the drag began, and
/// what the drag does.
pub struct Drag {
    pub base: Video,
    pub kind: DragKind,
    /// Where the pointer went down, in timeline seconds and track rows.
    pub from: (f64, f32),
    /// The command the drag makes so far.
    pub command: Option<Command>,
    /// Commands applied first, such as keeping a speed as the keyframes of
    /// the view being edited.
    pub prefix: Vec<Command>,
    /// A keyframe drag starts from this speed (with any keyframe it added).
    pub start: Option<spectrum_video::Speed>,
    /// The clip pressed: a click on one clip of a larger selection selects
    /// just it when no drag follows.
    pub clicked: Option<u64>,
    pub started: Instant,
}

impl Drag {
    pub fn new(base: Video, kind: DragKind, from: (f64, f32)) -> Self {
        Self {
            base,
            kind,
            from,
            command: None,
            prefix: Vec::new(),
            start: None,
            clicked: None,
            started: Instant::now(),
        }
    }

    /// Every command the drag makes, in order.
    pub fn commands(&self) -> Vec<Command> {
        let mut commands = self.prefix.clone();
        commands.extend(self.command.clone());
        commands
    }
}

/// What a right-click was on, for the menu it opens.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum MenuTarget {
    #[default]
    Timeline,
    Clip(u64),
    Key {
        clip: u64,
        index: usize,
    },
    Track(u64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DragKind {
    Playhead,
    Move,
    Trim {
        clip: u64,
        edge: spectrum_video::Edge,
        ripple: bool,
    },
    Roll {
        clip: u64,
    },
    Slip {
        clip: u64,
    },
    Slide {
        clip: u64,
    },
    /// A speed keyframe of a clip.
    Key {
        clip: u64,
        index: usize,
    },
    /// A track's row by its top edge, from the height it had and the
    /// scroll there was.
    Resize {
        track: u64,
        height: f32,
        scroll: f32,
    },
    /// A track by its name, to another place in the list.
    TrackOrder {
        track: u64,
    },
    /// The grip above the ruler, from the timeline's height.
    Panel {
        height: f32,
    },
}

pub struct VideoEditor {
    pub id: AssetId,
    pub doc: Video,
    pub revision: Option<RevisionId>,
    queue: Vec<Command>,
    saving: bool,
    save_waiting: bool,
    changed_at: Option<Instant>,
    /// An undo or redo is saving; the edit reloads once it has.
    reload: bool,
    pub selected: Vec<u64>,
    /// The program monitor, whose playhead is the timeline's.
    pub monitor: Player,
    /// Timeline pixels per second.
    pub zoom: f32,
    /// Timeline seconds at the left edge of the tracks.
    pub scroll: f64,
    pub tool: Tool,
    pub snapping: bool,
    pub drag: Option<Drag>,
    pub timeline: Rc<RefCell<Bounds<Pixels>>>,
    /// An export's share done while one runs.
    pub exporting: Option<f32>,
    /// When the last edit reached the monitor, to time edits to screen.
    pub edited_at: Option<Instant>,
    /// The selected clips' opacity and volume.
    /// The selected speed keyframe: its clip and place.
    pub key: Option<(u64, usize)>,
    /// The media chosen in the Media sidebar, for , and .
    pub media: Option<AssetId>,
    /// Rows dragged taller or shorter, by track, for this session.
    pub heights: std::collections::HashMap<u64, f32>,
    /// How far the tracks are scrolled up under the ruler.
    pub scroll_y: f32,
    /// Where a track being moved would go, as a timeline height.
    pub track_drop: Option<f32>,
    /// How tall the timeline is, kept between launches.
    pub panel: f32,
    /// The pointer over the timeline, for the razor's mark.
    pub hover: Option<Point<Pixels>>,
    /// Whether speed shows as velocity or as a time remap; either view
    /// shows the same timing.
    pub speed_view: spectrum_video::SpeedForm,
    /// What the last right-click was on.
    pub menu: MenuTarget,
    /// Stopping returns the playhead to where play began, and from where.
    pub return_to_start: bool,
    pub played_from: Option<Time>,
    /// In and Out, which bound a preview render (and later, exports).
    pub in_point: Option<Time>,
    pub out_point: Option<Time>,
    /// Frames rendered ahead, played from memory.
    pub preview: std::sync::Arc<crate::video_preview::Preview>,
    pub strip: crate::video_preview::Strip,
    pub keys: crate::video_speed_panel::KeyFields,
    /// The Media tab's search.
    pub media_query: Entity<gpui_component::input::InputState>,
    pub opacity: Entity<gpui_component::slider::SliderState>,
    pub volume: Entity<gpui_component::slider::SliderState>,
    _subscriptions: Vec<Subscription>,
}

impl VideoEditor {
    /// Saves are done and nothing waits.
    pub fn idle(&self) -> bool {
        !self.saving && self.queue.is_empty() && !self.save_waiting
    }
}

pub fn timing(video: &Video) -> Timing {
    Timing {
        rate: video.rate,
        // One more than the clips fill: the playhead can rest at the end,
        // where the next clip would go.
        frames: if video.duration() == Time::ZERO {
            1
        } else {
            frames(video) + 1
        },
        duration: video.duration(),
        picture: true,
    }
}

impl Workspace {
    /// Opens a video in the editor.
    pub fn load_video(&mut self, id: AssetId, window: &mut Window, cx: &mut Context<Self>) {
        self.video = None;
        let Ok(store) = &self.store else {
            return;
        };
        let (doc, revision) = match store.service.video_view(id) {
            Ok(found) => found,
            Err(error) => return self.notify_error(error, window, cx),
        };
        let root = store.root.clone();
        let size = self.player_size(window);
        let preview = std::sync::Arc::new(crate::video_preview::Preview::default());
        let pictures = {
            let (root, doc, preview) = (root.clone(), doc.clone(), preview.clone());
            player_decode::start_with(move || {
                Ok(Box::new(ProgramPictures::open(root, doc, preview)?)
                    as Box<dyn player_decode::Pictures>)
            })
        };
        let (requests, frames) = match pictures {
            Ok(started) => started,
            Err(error) => return self.notify_error(error, window, cx),
        };
        let audio = {
            let (root, doc) = (root.clone(), doc.clone());
            AudioOut::open_with(move || {
                ProgramSound::open(root, doc)
                    .ok()
                    .map(|sound| Box::new(sound) as Box<dyn crate::audio_out::Sounds>)
            })
        };
        let mut monitor = Player::new(
            id,
            None,
            timing(&doc),
            size,
            Some(requests),
            None,
            Some(audio),
        );
        // Scrubs show quick frames; a rested one is then made carefully.
        monitor.refines = true;
        monitor.seek(Time::ZERO);
        let seconds = doc.duration().seconds().max(10.);
        let opacity = crate::controls::in_sidebar(crate::workspace::slider(cx, 0., 100., 1., 100.));
        let volume = crate::controls::in_sidebar(crate::workspace::slider(cx, -60., 12., 0.5, 0.));
        use gpui_component::slider::SliderEvent;
        let media_query = cx.new(|cx| {
            gpui_component::input::InputState::new(window, cx).placeholder("Search media")
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &media_query,
                window,
                |_, _, _: &gpui_component::input::InputEvent, _, cx| cx.notify(),
            ),
            cx.subscribe_in(
                &opacity,
                window,
                |this, state, _: &SliderEvent, window, cx| {
                    let opacity = state.read(cx).value().start() / 100.;
                    this.set_video_clips_of(
                        spectrum_video::TrackKind::Video,
                        |clip| Command::SetClip {
                            clip,
                            name: None,
                            enabled: None,
                            opacity: Some(opacity),
                            gain_db: None,
                            interpolation: None,
                            sound_follows_speed: None,
                        },
                        window,
                        cx,
                    );
                },
            ),
            cx.subscribe_in(
                &volume,
                window,
                |this, state, _: &SliderEvent, window, cx| {
                    let gain = state.read(cx).value().start();
                    this.set_video_clips_of(
                        spectrum_video::TrackKind::Audio,
                        |clip| Command::SetClip {
                            clip,
                            name: None,
                            enabled: None,
                            opacity: None,
                            gain_db: Some(gain),
                            interpolation: None,
                            sound_follows_speed: None,
                        },
                        window,
                        cx,
                    );
                },
            ),
        ];
        self.video = Some(VideoEditor {
            id,
            doc,
            revision: Some(revision),
            queue: Vec::new(),
            saving: false,
            save_waiting: false,
            changed_at: None,
            reload: false,
            selected: Vec::new(),
            monitor,
            // The whole edit fits at first.
            zoom: (900. / seconds) as f32,
            scroll: 0.,
            tool: Tool::Select,
            snapping: true,
            drag: None,
            timeline: Default::default(),
            exporting: None,
            edited_at: None,
            key: None,
            media: None,
            heights: Default::default(),
            scroll_y: 0.,
            track_drop: None,
            panel: crate::prefs::get("timeline_height")
                .and_then(|v| v.as_f64())
                .map_or(crate::video_layout::PANEL, |h| h as f32),
            hover: None,
            speed_view: spectrum_video::SpeedForm::Velocity,
            menu: MenuTarget::Timeline,
            return_to_start: crate::prefs::get("return_to_start")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            played_from: None,
            in_point: None,
            out_point: None,
            preview,
            strip: Default::default(),
            keys: crate::video_speed_panel::KeyFields::new(window, cx),
            media_query,
            opacity,
            volume,
            _subscriptions: subscriptions,
        });
        self.receive_program(id, frames, window, cx);
    }

    /// Hands the program's frames to the monitor as they arrive.
    fn receive_program(
        &mut self,
        id: AssetId,
        frames: flume::Receiver<player_decode::Delivery>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                while this
                    .read_with(cx, |ws, _| {
                        ws.video.as_ref().is_some_and(|v| v.monitor.full())
                    })
                    .unwrap_or(false)
                {
                    cx.background_executor()
                        .timer(Duration::from_millis(4))
                        .await;
                }
                let Ok(delivery) = frames.recv_async().await else {
                    break;
                };
                let delivered = this.update_in(cx, |this, window, cx| {
                    let frame = match delivery {
                        Ok(frame) => frame,
                        Err(error) => return this.notify_error(error, window, cx),
                    };
                    match this.video.as_mut().filter(|v| v.id == id) {
                        Some(video) => {
                            if let Some(edited) = video.edited_at.take() {
                                crate::perf::record("video_edit_to_screen", edited.elapsed());
                            }
                            video.monitor.receive(frame)
                        }
                        None => crate::retire::drop_later(frame.image),
                    }
                    cx.notify();
                });
                if delivered.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Applies a command to each selected clip on tracks of `kind`.
    pub fn set_video_clips_of(
        &mut self,
        kind: spectrum_video::TrackKind,
        command: impl Fn(u64) -> Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(video) = &self.video else {
            return;
        };
        let commands: Vec<_> = video
            .selected
            .iter()
            .filter(|id| video.doc.clip(**id).is_some_and(|(t, _)| t.kind == kind))
            .map(|id| command(*id))
            .collect();
        if !commands.is_empty() {
            self.video_commands(commands, window, cx);
        }
    }

    /// Shows the first selected clips' opacity and volume on the sliders.
    pub fn sync_video_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(video) = &self.video else {
            return;
        };
        let first = |kind| {
            video
                .selected
                .iter()
                .filter_map(|id| video.doc.clip(*id))
                .find(|(t, _)| t.kind == kind)
                .map(|(_, c)| c.clone())
        };
        if let Some(clip) = first(spectrum_video::TrackKind::Video) {
            video
                .opacity
                .update(cx, |s, cx| s.set_value(clip.opacity * 100., window, cx));
        }
        if let Some(clip) = first(spectrum_video::TrackKind::Audio) {
            video
                .volume
                .update(cx, |s, cx| s.set_value(clip.gain_db, window, cx));
        }
    }

    /// Shows `doc` as the edit: on the timeline and in the monitor.
    pub fn show_video_doc(&mut self, doc: Video) {
        let Some(video) = &mut self.video else {
            return;
        };
        if video.doc == doc {
            return;
        }
        video.monitor.retime(timing(&doc));
        video
            .monitor
            .update(Box::new(doc.clone()), Box::new(doc.clone()));
        video.selected.retain(|id| doc.clip(*id).is_some());
        video.doc = doc;
        video.edited_at = Some(Instant::now());
    }

    /// Applies edits: on screen at once, saved after a pause.
    pub fn video_commands(
        &mut self,
        commands: Vec<Command>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(video) = &mut self.video else {
            return;
        };
        let history = commands
            .iter()
            .any(|c| matches!(c, Command::Undo | Command::Redo));
        if !history {
            let mut doc = video.doc.clone();
            for command in &commands {
                if let Err(error) = spectrum_video::apply(&mut doc, command.clone()) {
                    return self.notify_error(error, window, cx);
                }
            }
            self.show_video_doc(doc);
        }
        let Some(video) = &mut self.video else {
            return;
        };
        video.reload |= history;
        video.changed_at = Some(Instant::now());
        video.queue.extend(commands);
        self.save_video(window, cx);
        cx.notify();
    }

    /// Saves queued edits one batch at a time, after a pause in editing.
    fn save_video(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(video), Ok(store)) = (&mut self.video, &self.store) else {
            return;
        };
        if video.saving || video.queue.is_empty() {
            return;
        }
        const PAUSE: Duration = Duration::from_millis(300);
        let quiet = video.changed_at.map_or(PAUSE, |at| at.elapsed());
        let history = matches!(video.queue.last(), Some(Command::Undo | Command::Redo));
        if quiet < PAUSE && !history {
            if !video.save_waiting {
                video.save_waiting = true;
                let wait = PAUSE - quiet;
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    this.update_in(cx, |this, window, cx| {
                        if let Some(video) = &mut this.video {
                            video.save_waiting = false;
                        }
                        this.save_video(window, cx);
                    })
                    .ok();
                })
                .detach();
            }
            return;
        }
        video.saving = true;
        let batch = std::mem::take(&mut video.queue);
        let (root, id, base) = (store.root.clone(), video.id, video.revision);
        let reload = std::mem::take(&mut video.reload);
        let task = cx.background_executor().spawn(async move {
            let started = Instant::now();
            let mut service = Service::open(&root)?;
            crate::perf::record("video_save_open", started.elapsed());
            let editing = Instant::now();
            let (_, revision) = service.edit_video_from(id, base, batch)?;
            crate::perf::record("video_save_edit", editing.elapsed());
            let doc = if reload {
                Some(service.video_view(id)?.0)
            } else {
                None
            };
            crate::perf::record("video_save", started.elapsed());
            Ok::<_, anyhow::Error>((revision, doc))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(video) = this.video.as_mut().filter(|v| v.id == id) else {
                    return;
                };
                video.saving = false;
                match result {
                    Ok((revision, doc)) => {
                        video.revision = Some(revision);
                        if let Some(doc) = doc {
                            this.show_video_doc(doc);
                        }
                    }
                    Err(error) => {
                        this.notify_error(error, window, cx);
                        // Show what the library really holds.
                        if let Ok(store) = &this.store
                            && let Ok((doc, revision)) = store.service.video_view(id)
                        {
                            this.show_video_doc(doc);
                            if let Some(video) = &mut this.video {
                                video.revision = Some(revision);
                                video.queue.clear();
                            }
                        }
                    }
                }
                if let Ok(store) = &mut this.store {
                    store.thumbs.remove(&id);
                }
                this.save_video(window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Drop for VideoEditor {
    fn drop(&mut self) {
        // A render for a closed video is wasted work.
        self.preview.cancel();
    }
}
