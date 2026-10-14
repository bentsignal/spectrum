//! The video editor's commands: what Command+P lists beside the tools and
//! what a right-click on the timeline offers, each with its key. A clip's
//! menu acts on the selection; a keyframe's sets its curve.
use crate::{
    video_speed_panel::CURVES,
    video_state::{MenuTarget, Tool},
    workspace::Workspace,
};
use gpui::*;
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use spectrum_video::{Command, SpeedForm, TrackKind};

type Run = fn(&mut Workspace, &mut Window, &mut Context<Workspace>);

/// A command the palette lists: its name, what it does, and its key.
pub struct Action {
    pub name: &'static str,
    pub detail: &'static str,
    pub key: Option<&'static str>,
    pub run: Run,
}

const fn action(
    name: &'static str,
    detail: &'static str,
    key: Option<&'static str>,
    run: Run,
) -> Action {
    Action {
        name,
        detail,
        key,
        run,
    }
}

const LINK_KEY: &str = if cfg!(target_os = "macos") {
    "⌘L"
} else {
    "Ctrl+L"
};

pub const ACTIONS: [Action; 22] = [
    action(
        "Cut at playhead",
        "Cut the selected clips, or every clip, at the playhead",
        Some("S"),
        |this, window, cx| this.split_at_playhead(window, cx),
    ),
    action(
        "Delete",
        "Remove the selected clips, leaving a gap",
        Some("⌫"),
        |this, window, cx| this.delete_video_clips(false, window, cx),
    ),
    action(
        "Ripple delete",
        "Remove the selected clips and close the gap",
        Some("⇧⌫"),
        |this, window, cx| this.delete_video_clips(true, window, cx),
    ),
    action(
        "Link or unlink",
        "Picture and sound move together, or on their own",
        Some(LINK_KEY),
        |this, window, cx| this.toggle_link(window, cx),
    ),
    action(
        "Enable or disable",
        "A disabled clip stays on the timeline without showing",
        Some("⇧E"),
        |this, window, cx| this.toggle_enabled(window, cx),
    ),
    action(
        "Freeze frame",
        "Hold the frame at the playhead for a second, then play on",
        None,
        |this, window, cx| this.freeze_frame(window, cx),
    ),
    action(
        "Reverse",
        "Play the clip's frames backward, ramps and all",
        None,
        |this, window, cx| this.reverse_clip(window, cx),
    ),
    action(
        "Reset speed",
        "Back to the source's own speed",
        None,
        |this, window, cx| this.reset_speed(window, cx),
    ),
    action(
        "Add speed keyframe",
        "At the playhead, keeping the timing there",
        None,
        |this, window, cx| this.add_speed_key(window, cx),
    ),
    action(
        "Show speed as velocity",
        "How fast the source plays; the same timing",
        None,
        |this, window, cx| this.set_speed_view(SpeedForm::Velocity, window, cx),
    ),
    action(
        "Show speed as time remap",
        "Which source moment shows when; the same timing",
        None,
        |this, window, cx| this.set_speed_view(SpeedForm::Remap, window, cx),
    ),
    action(
        "Add video track",
        "A new track above the others",
        None,
        |this, window, cx| this.add_track(TrackKind::Video, window, cx),
    ),
    action(
        "Add audio track",
        "A new track below the others",
        None,
        |this, window, cx| this.add_track(TrackKind::Audio, window, cx),
    ),
    action(
        "Snapping",
        "Edges snap to the playhead and other clips",
        None,
        |this, _, cx| this.toggle_video_snapping(cx),
    ),
    action(
        "Return to start when stopping",
        "Stopping playback goes back to where it began",
        None,
        |this, _, cx| this.toggle_return_to_start(cx),
    ),
    action(
        "Zoom to fit",
        "The whole edit across the timeline",
        Some(if cfg!(target_os = "macos") {
            "⌘0"
        } else {
            "Ctrl+0"
        }),
        |this, _, cx| this.zoom_timeline(0, cx),
    ),
    action(
        "Render preview",
        "Render In to Out (or the whole edit) ahead, so it plays perfectly",
        Some("Enter"),
        |this, _, cx| this.render_video_preview(cx),
    ),
    action(
        "Stop rendering",
        "Keep what is rendered so far",
        None,
        |this, _, cx| this.stop_preview(cx),
    ),
    action(
        "Set In",
        "Where a preview render starts, at the playhead",
        Some("I"),
        |this, _, cx| this.set_in_point(cx),
    ),
    action(
        "Set Out",
        "Where a preview render ends, at the playhead",
        Some("O"),
        |this, _, cx| this.set_out_point(cx),
    ),
    action(
        "Clear In and Out",
        "Preview renders cover the whole edit again",
        Some(if cfg!(target_os = "macos") {
            "⌥X"
        } else {
            "Alt+X"
        }),
        |this, _, cx| this.clear_in_out(cx),
    ),
    action(
        "Go to the next cut",
        "Down goes to the next, Up to the one before",
        Some("↓"),
        |this, _, cx| this.jump_to_cut(true, cx),
    ),
];

impl Workspace {
    pub fn toggle_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(video) = &self.video else {
            return;
        };
        let clips = video.selected.clone();
        if clips.is_empty() {
            return;
        }
        // Linked clips unlink; anything else links.
        let linked = clips
            .iter()
            .any(|id| video.doc.clip(*id).is_some_and(|(_, c)| c.link.is_some()));
        let command = if linked {
            Command::Unlink { clips }
        } else {
            Command::Link { clips }
        };
        self.video_commands(vec![command], window, cx);
    }

    pub fn toggle_enabled(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(video) = &self.video else {
            return;
        };
        let enabled = video
            .selected
            .iter()
            .all(|id| video.doc.clip(*id).is_some_and(|(_, c)| c.enabled));
        self.set_video_clips(
            |clip| Command::SetClip {
                clip,
                name: None,
                enabled: Some(!enabled),
                opacity: None,
                gain_db: None,
                interpolation: None,
                sound_follows_speed: None,
            },
            window,
            cx,
        );
    }

    pub fn add_track(&mut self, kind: TrackKind, window: &mut Window, cx: &mut Context<Self>) {
        self.video_commands(vec![Command::AddTrack { kind }], window, cx);
    }

    fn remove_track(&mut self, track: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.video_commands(vec![Command::RemoveTrack { track }], window, cx);
    }

    pub fn toggle_video_snapping(&mut self, cx: &mut Context<Self>) {
        if let Some(video) = &mut self.video {
            video.snapping = !video.snapping;
        }
        cx.notify();
    }

    pub fn toggle_return_to_start(&mut self, cx: &mut Context<Self>) {
        if let Some(video) = &mut self.video {
            video.return_to_start = !video.return_to_start;
            if !crate::benchmark::running() {
                crate::prefs::set("return_to_start", video.return_to_start.into());
            }
        }
        cx.notify();
    }

    /// The right-click menu, for what the click was on.
    pub fn timeline_menu(
        &self,
        cx: &mut Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let view = cx.entity();
        move |menu, _, cx| {
            let (target, curve, tool) = {
                let this = view.read(cx);
                let Some(video) = &this.video else {
                    return menu;
                };
                let curve = this
                    .selected_key()
                    .map(|(clip, index)| crate::video_speed::keys(&clip.speed)[index].2);
                (video.menu, curve, video.tool)
            };
            let item = |label: &'static str, run: Run| {
                let view = view.clone();
                PopupMenuItem::new(label).on_click(move |_, window, cx| {
                    view.update(cx, |this, cx| run(this, window, cx))
                })
            };
            let menu = menu.min_w(px(220.));
            match target {
                MenuTarget::Key { .. } => {
                    let menu = CURVES.iter().fold(menu, |menu, (choice, label, _)| {
                        let (view, choice) = (view.clone(), *choice);
                        menu.item(
                            PopupMenuItem::new(*label)
                                .checked(curve == Some(choice))
                                .on_click(move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.set_key_curve(choice, window, cx)
                                    })
                                }),
                        )
                    });
                    menu.separator()
                        .item(item("Delete keyframe", |this, window, cx| {
                            this.delete_speed_key(window, cx)
                        }))
                }
                MenuTarget::Clip(_) => menu
                    .item(item("Cut at playhead", |this, window, cx| {
                        this.split_at_playhead(window, cx)
                    }))
                    .item(item("Delete", |this, window, cx| {
                        this.delete_video_clips(false, window, cx)
                    }))
                    .item(item("Ripple delete", |this, window, cx| {
                        this.delete_video_clips(true, window, cx)
                    }))
                    .item(item("Link or unlink", |this, window, cx| {
                        this.toggle_link(window, cx)
                    }))
                    .item(item("Enable or disable", |this, window, cx| {
                        this.toggle_enabled(window, cx)
                    }))
                    .separator()
                    .item(item("Freeze frame", |this, window, cx| {
                        this.freeze_frame(window, cx)
                    }))
                    .item(item("Reverse", |this, window, cx| {
                        this.reverse_clip(window, cx)
                    }))
                    .item(item("Reset speed", |this, window, cx| {
                        this.reset_speed(window, cx)
                    }))
                    .item(item("Speed…", |this, window, cx| {
                        this.set_mode(crate::workspace::Mode::Speed, window, cx)
                    })),
                MenuTarget::Track(track) => {
                    let view = view.clone();
                    menu.item(item("Add video track", |this, window, cx| {
                        this.add_track(TrackKind::Video, window, cx)
                    }))
                    .item(item("Add audio track", |this, window, cx| {
                        this.add_track(TrackKind::Audio, window, cx)
                    }))
                    .separator()
                    .item(
                        PopupMenuItem::new("Remove track").on_click(move |_, window, cx| {
                            view.update(cx, |this, cx| this.remove_track(track, window, cx))
                        }),
                    )
                }
                MenuTarget::Timeline => {
                    let razor = tool == Tool::Razor;
                    menu.item(item("Add video track", |this, window, cx| {
                        this.add_track(TrackKind::Video, window, cx)
                    }))
                    .item(item("Add audio track", |this, window, cx| {
                        this.add_track(TrackKind::Audio, window, cx)
                    }))
                    .separator()
                    .item(
                        PopupMenuItem::new(if razor { "Select tool" } else { "Razor tool" })
                            .on_click({
                                let view = view.clone();
                                move |_, _, cx| {
                                    view.update(cx, |this, cx| {
                                        this.set_video_tool(
                                            if razor { Tool::Select } else { Tool::Razor },
                                            cx,
                                        )
                                    })
                                }
                            }),
                    )
                }
            }
        }
    }
}
