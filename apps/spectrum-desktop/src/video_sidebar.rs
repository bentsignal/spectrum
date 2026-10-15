//! The video editor's sidebar tabs, each about one thing, under the tool
//! button. Clip: the selected clips' settings. Media: the project's
//! footage, audio, images, and canvases, to search and add. Export: writing
//! the video out. (Speed has its own file; actions are in Command+P and the
//! timeline's right-click menu.)
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
    input::Input,
};
use spectrum_library::{AssetId, AssetKind};
use spectrum_media::{ExportSettings, VideoCodec};
use spectrum_video::{Command, Placement, TrackKind};

/// Media dragged from the sidebar onto the timeline.
#[derive(Clone)]
pub struct DraggedMedia(pub AssetId, pub String);

impl Render for DraggedMedia {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_1p5()
            .rounded_md()
            .bg(rgb(SELECTED))
            .shadow_lg()
            .text_sm()
            .child(self.1.clone())
    }
}

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
    /// The current tool, at the top of every video tab; it and Command+P
    /// list the tools and the editor's commands.
    pub fn video_tool_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tool = self.video.as_ref().map_or(Tool::Select, |v| v.tool);
        div()
            .id("video-tool")
            .h(px(34.))
            .px_3()
            .flex()
            .items_center()
            .gap_2p5()
            .rounded_md()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .hover(|el| el.bg(rgb(HOVER)))
            .child(key_cap(tool.key()))
            .child(div().flex_1().text_sm().child(tool.label()))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(FAINT))
                    .child(crate::tools::SHORTCUT),
            )
            .on_click(cx.listener(|this, _, window, cx| this.open_tools(window, cx)))
    }

    /// The Clip tab: the selected clips' settings.
    pub fn video_clip_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let Some(video) = &self.video else {
            return div();
        };
        let snapping = video.snapping;
        let tool = group("Timeline", None).child(
            toggle("snapping", "Snap to edges and the playhead", snapping)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_video_snapping(cx))),
        );
        let selected: Vec<_> = video
            .selected
            .iter()
            .filter_map(|id| video.doc.clip(*id).map(|(t, c)| (t.kind, c)))
            .collect();
        if selected.is_empty() {
            return div().flex().flex_col().gap_7().child(tool).child(
                div()
                    .text_xs()
                    .text_color(rgb(FAINT))
                    .child("Select a clip to see its settings. Right-click a clip, or press ⌘P, for what it can do."),
            );
        }
        let pictures = selected
            .iter()
            .filter(|(k, _)| *k == TrackKind::Video)
            .count();
        let sounds = selected.len() - pictures;
        let enabled = selected.iter().all(|(_, c)| c.enabled);
        let opacity = video.opacity.read(cx).value().start();
        let volume = video.volume.read(cx).value().start();
        let rate = video.doc.rate;
        let title = match selected.as_slice() {
            [(_, clip)] => clip.name.clone(),
            [(_, first), (_, second)] if first.link.is_some() && first.link == second.link => {
                first.name.clone()
            }
            _ => format!("{} clips", selected.len()),
        };
        let first = selected[0].1;
        let facts = format!(
            "Starts {} · lasts {}",
            first.start.timecode(rate),
            first.duration.timecode(rate)
        );
        div().flex().flex_col().gap_7().child(tool).child(
            group("Clip", None)
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().text_sm().truncate().child(title))
                        .child(div().text_xs().text_color(rgb(FAINT)).child(facts)),
                )
                .child(
                    toggle("enabled", "Enabled", enabled).on_click(
                        cx.listener(|this, _, window, cx| this.toggle_enabled(window, cx)),
                    ),
                )
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
                }),
        )
    }

    /// The Media tab: the project's footage, audio, images, and canvases
    /// as a grid to search, drag onto the timeline, or choose for , and .
    pub fn video_media_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let (Ok(store), Some(video)) = (&self.store, &self.video) else {
            return div();
        };
        let query = video.media_query.read(cx).value().to_lowercase();
        let chosen = video.media;
        let width = (crate::workspace::SIDEBAR_WIDTH - 32. - 10.) / 2.;
        let tiles = self
            .video_media(cx)
            .into_iter()
            .filter(|asset| query.is_empty() || asset.name.to_lowercase().contains(&query));
        let tiles = tiles.map(|asset| {
            let id = asset.id;
            let name = asset.name.clone();
            let detail = crate::grid::kind_label(asset.kind);
            div()
                .id(SharedString::from(format!("media-{id}")))
                .w(px(width))
                .p_1()
                .flex()
                .flex_col()
                .gap_1()
                .rounded_md()
                .when(chosen == Some(id), |el| el.bg(rgb(SELECTED)))
                .hover(|el| el.bg(rgb(HOVER)))
                .child(crate::grid::frame(
                    store.thumbs.get(&id),
                    width - 8.,
                    (width - 8.) * 9. / 16.,
                ))
                .child(
                    div()
                        .px_0p5()
                        .text_xs()
                        .truncate()
                        .child(asset.name.clone()),
                )
                .child(
                    div()
                        .px_0p5()
                        .text_xs()
                        .text_color(rgb(FAINT))
                        .child(detail),
                )
                // Drag onto the timeline; a click chooses it for , (insert)
                // and . (overwrite); a double-click adds it at the playhead.
                .on_drag(DraggedMedia(id, name), |media, _, _, cx| {
                    cx.new(|_| media.clone())
                })
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if let Some(video) = &mut this.video {
                        video.media = Some(id);
                    }
                    if event.click_count() == 2 {
                        this.add_to_timeline(id, Placement::Overwrite, window, cx);
                    }
                    cx.notify();
                }))
        });
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(Input::new(&video.media_query).small())
            .child(div().flex().flex_wrap().gap(px(10.)).children(tiles))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(FAINT))
                    .child("Drag onto a track, or choose one and press , to insert or . to overwrite at the playhead."),
            )
    }

    /// The project's assets a video can use.
    pub fn video_media(&self, _: &App) -> Vec<spectrum_library::Asset> {
        let (Ok(store), Place::Project(project)) = (&self.store, self.place) else {
            return Vec::new();
        };
        store
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
            })
            .collect()
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

    /// Adds media dropped on the timeline where it was dropped: at that
    /// time, on that track when it is the right kind.
    pub fn drop_media(
        &mut self,
        asset: AssetId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(video), Ok(store)) = (&self.video, &self.store) else {
            return;
        };
        let (x, y) = video.local(position);
        let at = video.doc.snap(video.time_at(x));
        let layout = video.layout();
        let row = layout.rows.iter().find(|r| {
            let (top, bottom) = layout.area();
            y >= top.max(r.top) && y < bottom.min(r.top + r.height)
        });
        let kind = match store.service.library.get(asset).map(|a| a.kind) {
            Ok(AssetKind::Audio) => TrackKind::Audio,
            _ => TrackKind::Video,
        };
        let track = row.filter(|r| r.kind == kind).map(|r| r.track);
        match store
            .service
            .clip_command(asset, at, track, Placement::Overwrite)
        {
            Ok(command) => self.video_commands(vec![command], window, cx),
            Err(error) => self.notify_error(error, window, cx),
        }
    }

    /// , and .: the chosen media at the playhead, inserted or overwriting.
    pub fn add_chosen_media(
        &mut self,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(asset) = self.video.as_ref().and_then(|v| v.media) else {
            return false;
        };
        self.add_to_timeline(asset, placement, window, cx);
        true
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

/// A tool's key, as a key cap.
fn key_cap(key: &'static str) -> impl IntoElement {
    div()
        .min_w(px(20.))
        .h(px(20.))
        .px_1()
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .border_1()
        .border_color(rgb(BORDER))
        .text_xs()
        .text_color(rgb(MUTED))
        .child(key.to_uppercase())
}
