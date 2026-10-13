//! The video editor's sidebar modes. Edit: the timeline's tools and the
//! selected clips. Media: the project's footage, audio, images, and
//! canvases, added at the playhead. Export: writing the video out.
use crate::{
    controls::{chip, group, slider_row, toggle},
    theme::*,
    video_state::Tool,
    workspace::{Place, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
};
use spectrum_library::{AssetId, AssetKind};
use spectrum_media::{ExportSettings, VideoCodec};
use spectrum_video::{Command, Placement, TrackKind};

/// Export choices, kept for the session.
#[derive(Clone, Copy, PartialEq)]
pub struct ExportChoice {
    pub codec: VideoCodec,
    /// The long edge, or the video's own size when `None`.
    pub long_edge: Option<u32>,
}

impl Default for ExportChoice {
    fn default() -> Self {
        Self {
            codec: VideoCodec::H264,
            long_edge: None,
        }
    }
}

impl Workspace {
    pub fn video_edit_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let Some(video) = &self.video else {
            return div();
        };
        let tools = div()
            .flex()
            .flex_col()
            .gap_0p5()
            .children(Tool::ALL.map(|tool| {
                let on = video.tool == tool;
                div()
                    .id(SharedString::from(format!("tool-{}", tool.key())))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .flex()
                    .justify_between()
                    .text_sm()
                    .when(on, |el| el.bg(rgb(SELECTED)))
                    .hover(|el| el.bg(rgb(HOVER)))
                    .child(tool.label())
                    .child(
                        div()
                            .text_color(rgb(FAINT))
                            .child(tool.key().to_uppercase()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.set_video_tool(tool, cx)))
            }));
        let snapping = video.snapping;
        let selected: Vec<_> = video
            .selected
            .iter()
            .filter_map(|id| video.doc.clip(*id).map(|(t, c)| (t.kind, c)))
            .collect();
        let pictures = selected
            .iter()
            .filter(|(k, _)| *k == TrackKind::Video)
            .count();
        let sounds = selected.len() - pictures;
        let enabled = selected.iter().all(|(_, c)| c.enabled);
        let opacity = video.opacity.read(cx).value().start();
        let volume = video.volume.read(cx).value().start();
        let actions = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(action(
                "split",
                "Cut at playhead (S)",
                cx,
                |this, window, cx| this.split_at_playhead(window, cx),
            ))
            .child(action("delete", "Delete", cx, |this, window, cx| {
                this.delete_video_clips(false, window, cx)
            }))
            .child(action("ripple", "Ripple delete", cx, |this, window, cx| {
                this.delete_video_clips(true, window, cx)
            }))
            .child(action("link", "Link", cx, |this, window, cx| {
                this.link_video_clips(true, window, cx)
            }))
            .child(action("unlink", "Unlink", cx, |this, window, cx| {
                this.link_video_clips(false, window, cx)
            }));
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(group("Tools", None).gap_2().child(tools).child(
                toggle("snapping", "Snap to edges and playhead", snapping).on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(video) = &mut this.video {
                            video.snapping = !video.snapping;
                        }
                        cx.notify();
                    },
                )),
            ))
            .child(
                group("Clips", None)
                    .gap_3()
                    .child(actions)
                    .when(!selected.is_empty(), |el| {
                        el.child(toggle("enabled", "Enabled", enabled).on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.set_video_clips(
                                    |id| Command::SetClip {
                                        clip: id,
                                        name: None,
                                        enabled: Some(!enabled),
                                        opacity: None,
                                        gain_db: None,
                                        interpolation: None,
                                        sound_follows_speed: None,
                                    },
                                    window,
                                    cx,
                                )
                            },
                        )))
                        .when(pictures > 0, |el| {
                            el.child(slider_row(
                                "Opacity",
                                format!("{opacity:.0}%"),
                                &video.opacity,
                            ))
                        })
                        .when(sounds > 0, |el| {
                            el.child(slider_row(
                                "Volume",
                                format!("{volume:+.1} dB"),
                                &video.volume,
                            ))
                        })
                    }),
            )
            .children(self.speed_section(cx))
            .child(
                group("Tracks", None).child(
                    div()
                        .flex()
                        .gap_2()
                        .child(action(
                            "add-video",
                            "Add video track",
                            cx,
                            |this, window, cx| {
                                this.video_commands(
                                    vec![Command::AddTrack {
                                        kind: TrackKind::Video,
                                    }],
                                    window,
                                    cx,
                                )
                            },
                        ))
                        .child(action(
                            "add-audio",
                            "Add audio track",
                            cx,
                            |this, window, cx| {
                                this.video_commands(
                                    vec![Command::AddTrack {
                                        kind: TrackKind::Audio,
                                    }],
                                    window,
                                    cx,
                                )
                            },
                        )),
                ),
            )
    }

    pub fn video_media_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let Ok(store) = &self.store else {
            return div();
        };
        let Place::Project(project) = self.place else {
            return div();
        };
        let assets = store
            .service
            .library
            .project_assets(project)
            .unwrap_or_default()
            .into_iter()
            .filter(|asset| {
                matches!(
                    asset.kind,
                    AssetKind::Footage | AssetKind::Audio | AssetKind::Image | AssetKind::Canvas
                )
            });
        let rows = assets.map(|asset| {
            let id = asset.id;
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1p5()
                .rounded_md()
                .hover(|el| el.bg(rgb(HOVER)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().text_sm().truncate().child(asset.name.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(FAINT))
                                .child(crate::grid::kind_label(asset.kind)),
                        ),
                )
                .child(
                    Button::new(SharedString::from(format!("overwrite-{id}")))
                        .ghost()
                        .xsmall()
                        .label("Add")
                        .tooltip("Add at the playhead, over what is there (.)")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.add_to_timeline(id, Placement::Overwrite, window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("insert-{id}")))
                        .ghost()
                        .xsmall()
                        .label("Insert")
                        .tooltip("Insert at the playhead, pushing later clips along (,)")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.add_to_timeline(id, Placement::Insert, window, cx)
                        })),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(FAINT))
                    .child("This project's footage, audio, images, and canvases."),
            )
            .children(rows)
    }

    pub fn video_export_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let Some(video) = &self.video else {
            return div();
        };
        let choice = self.video_export;
        let codecs = div().flex().flex_wrap().gap_2().children(
            [
                (VideoCodec::H264, "h264", "H.264 (MP4)"),
                (VideoCodec::Hevc, "hevc", "HEVC (MP4)"),
                (VideoCodec::ProRes, "prores", "ProRes (MOV)"),
            ]
            .map(|(codec, id, label)| {
                chip(id, label, choice.codec == codec).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.video_export.codec = codec;
                        cx.notify();
                    },
                ))
            }),
        );
        let long = video.doc.width.max(video.doc.height);
        let sizes = div().flex().flex_wrap().gap_2().children(
            [
                (None, "full", "Full size"),
                (Some(1920), "hd", "1080p"),
                (Some(1280), "720", "720p"),
            ]
            .into_iter()
            .filter(|(edge, ..)| edge.is_none_or(|edge| edge < long))
            .map(|(edge, id, label)| {
                chip(id, label, choice.long_edge == edge).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.video_export.long_edge = edge;
                        cx.notify();
                    },
                ))
            }),
        );
        let exporting = video.exporting;
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(group("Format", None).gap_2().child(codecs))
            .child(group("Size", None).gap_2().child(sizes))
            .child(
                Button::new("export-video")
                    .primary()
                    .w_full()
                    .label(match exporting {
                        Some(share) => format!("Exporting {:.0}%…", share * 100.),
                        None => "Export…".into(),
                    })
                    .loading(exporting.is_some())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.choose_video_export(window, cx)),
                    ),
            )
    }

    pub fn set_video_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        if let Some(video) = &mut self.video {
            video.tool = tool;
        }
        cx.notify();
    }

    /// Applies a command to each selected clip.
    pub fn set_video_clips(
        &mut self,
        command: impl Fn(u64) -> Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(video) = &self.video else {
            return;
        };
        let commands: Vec<_> = video.selected.iter().map(|id| command(*id)).collect();
        if !commands.is_empty() {
            self.video_commands(commands, window, cx);
        }
    }

    /// Cuts the selected clips at the playhead, or every clip there when
    /// none is selected.
    pub fn split_at_playhead(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(video) = &self.video else {
            return;
        };
        let at = video.doc.snap(video.monitor.now());
        let clips = video.selected.clone();
        let cuts = video
            .doc
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .any(|c| c.start < at && at < c.end() && (clips.is_empty() || clips.contains(&c.id)));
        if cuts {
            self.video_commands(vec![Command::Split { at, clips }], window, cx);
        }
    }

    pub fn delete_video_clips(
        &mut self,
        ripple: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(video) = &mut self.video else {
            return;
        };
        let clips = std::mem::take(&mut video.selected);
        if !clips.is_empty() {
            self.video_commands(vec![Command::Delete { clips, ripple }], window, cx);
        }
    }

    fn link_video_clips(&mut self, linked: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(video) = &self.video else {
            return;
        };
        let clips = video.selected.clone();
        if clips.is_empty() {
            return;
        }
        let command = if linked {
            Command::Link { clips }
        } else {
            Command::Unlink { clips }
        };
        self.video_commands(vec![command], window, cx);
    }

    /// Adds an asset at the playhead.
    pub fn add_to_timeline(
        &mut self,
        asset: AssetId,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(video), Ok(store)) = (&self.video, &self.store) else {
            return;
        };
        let at = video.monitor.now();
        match store.service.clip_command(asset, at, None, placement) {
            Ok(command) => self.video_commands(vec![command], window, cx),
            Err(error) => self.notify_error(error, window, cx),
        }
    }

    fn choose_video_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(video) = &self.video else {
            return;
        };
        if video.exporting.is_some() {
            return;
        }
        let extension = self.video_export.codec.extension();
        let directory = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let name = format!("{}.{extension}", video.doc.name);
        let path = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = path.await else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                this.export_video_to(path, window, cx)
            })
            .ok();
        })
        .detach();
    }

    /// Exports the open video to `path` in the background.
    pub fn export_video_to(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(video), Ok(store)) = (&mut self.video, &self.store) else {
            return;
        };
        let choice = self.video_export;
        let (width, height) =
            crate::video_program::fitted(&video.doc, choice.long_edge.unwrap_or(u32::MAX));
        let settings = ExportSettings {
            codec: choice.codec,
            width: width & !1,
            height: height & !1,
            rate: video.doc.rate,
            bitrate: None,
            sound: true,
        };
        video.exporting = Some(0.);
        let (root, id) = (store.root.clone(), video.id);
        let share = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (progress, done) = (share.clone(), finished.clone());
        let task = cx.background_executor().spawn(async move {
            let result = spectrum_assets::Service::open(&root).and_then(|service| {
                service.export_video(id, &path, settings, &mut |part| {
                    progress.store(part.to_bits(), std::sync::atomic::Ordering::Relaxed);
                    true
                })
            });
            done.store(true, std::sync::atomic::Ordering::SeqCst);
            result.map(|_| path)
        });
        cx.spawn_in(window, async move |this, cx| {
            // Show how far along it is until it finishes.
            while !finished.load(std::sync::atomic::Ordering::SeqCst) {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(250))
                    .await;
                let part = f32::from_bits(share.load(std::sync::atomic::Ordering::Relaxed));
                let alive = this.update(cx, |this, cx| {
                    if let Some(video) = &mut this.video {
                        video.exporting = Some(part);
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    return;
                }
            }
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                if let Some(video) = &mut this.video {
                    video.exporting = None;
                }
                match result {
                    Ok(path) => {
                        use gpui_component::WindowExt;
                        window.push_notification(format!("Exported {}", path.display()), cx);
                    }
                    Err(error) => this.notify_error(error, window, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn action(
    id: &'static str,
    label: &'static str,
    cx: &mut Context<Workspace>,
    act: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
) -> Button {
    Button::new(id)
        .small()
        .label(label)
        .on_click(cx.listener(move |this, _, window, cx| act(this, window, cx)))
}
