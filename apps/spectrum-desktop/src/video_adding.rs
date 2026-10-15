//! Adding media from the keyboard: Command+P, "Add media…", then type to
//! search the project's footage, audio, images, and canvases, move with the
//! arrow keys, and press Enter to place the chosen one at the playhead
//! (over what is there; Shift+Enter inserts, pushing later clips along).
use crate::{grid::frame, theme::*, workspace::Workspace};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable,
    input::{Input, InputState},
};
use spectrum_video::Placement;

const TILE: f32 = 128.;
const COLUMNS: usize = 5;

/// The open "Add media" list: its search and the chosen tile.
pub struct Adding {
    pub query: Entity<InputState>,
    pub index: usize,
}

impl Workspace {
    pub fn open_adding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(video) = &mut self.video else {
            return;
        };
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search media"));
        query.update(cx, |state, cx| state.focus(window, cx));
        cx.observe(&query, |_, _, cx| cx.notify()).detach();
        video.adding = Some(Adding { query, index: 0 });
        cx.notify();
    }

    pub fn close_adding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(video) = &mut self.video
            && video.adding.take().is_some()
        {
            self.focus_handle.focus(window);
            cx.notify();
        }
    }

    /// The media matching the search, in order.
    fn adding_matches(&self, cx: &App) -> Vec<spectrum_library::Asset> {
        let query = self
            .video
            .as_ref()
            .and_then(|v| v.adding.as_ref())
            .map(|a| a.query.read(cx).value().to_lowercase())
            .unwrap_or_default();
        self.video_media(cx)
            .into_iter()
            .filter(|asset| query.is_empty() || asset.name.to_lowercase().contains(&query))
            .collect()
    }

    fn adding_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.adding_matches(cx).len();
        let Some(adding) = self.video.as_mut().and_then(|v| v.adding.as_mut()) else {
            return;
        };
        let last = count.saturating_sub(1);
        let index = adding.index.min(last);
        let moved = match event.keystroke.key.as_str() {
            "left" => Some(index.saturating_sub(1)),
            "right" => Some((index + 1).min(last)),
            "up" => Some(index.saturating_sub(COLUMNS)),
            "down" => Some((index + COLUMNS).min(last)),
            _ => None,
        };
        if let Some(moved) = moved {
            adding.index = moved;
            cx.stop_propagation();
            cx.notify();
            return;
        }
        match event.keystroke.key.as_str() {
            "enter" => {
                cx.stop_propagation();
                let placement = if event.keystroke.modifiers.shift {
                    Placement::Insert
                } else {
                    Placement::Overwrite
                };
                self.add_adding(index, placement, window, cx);
            }
            "escape" => {
                cx.stop_propagation();
                self.close_adding(window, cx);
            }
            _ => {}
        }
    }

    fn add_adding(
        &mut self,
        index: usize,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(asset) = self.adding_matches(cx).into_iter().nth(index) else {
            return;
        };
        self.close_adding(window, cx);
        if let Some(video) = &mut self.video {
            video.media = Some(asset.id);
        }
        self.add_to_timeline(asset.id, placement, window, cx);
    }

    /// The "Add media" list, over the editor.
    pub fn adding_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let adding = self.video.as_ref()?.adding.as_ref()?;
        let thumbs = self.store.as_ref().ok().map(|s| &s.thumbs);
        let matches = self.adding_matches(cx);
        let chosen = adding.index.min(matches.len().saturating_sub(1));
        let tiles = matches.iter().enumerate().map(|(index, asset)| {
            let on = index == chosen;
            div()
                .id(("add-media", index))
                .w(px(TILE + 8.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .p(px(2.))
                        .rounded(px(10.))
                        .border_2()
                        .border_color(if on {
                            rgb(0xd6d6d6).into()
                        } else {
                            transparent_black()
                        })
                        .child(frame(
                            thumbs.and_then(|t| t.get(&asset.id)),
                            TILE,
                            TILE * 9. / 16.,
                        )),
                )
                .child(div().px_1().text_xs().truncate().child(asset.name.clone()))
                .child(
                    div()
                        .px_1()
                        .text_xs()
                        .text_color(rgb(FAINT))
                        .child(crate::grid::kind_label(asset.kind)),
                )
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    let placement = if event.modifiers().shift {
                        Placement::Insert
                    } else {
                        Placement::Overwrite
                    };
                    this.add_adding(index, placement, window, cx)
                }))
        });
        let empty = matches.is_empty();
        Some(
            div()
                .id("adding-backdrop")
                .absolute()
                .inset_0()
                .occlude()
                .bg(hsla(0., 0., 0., 0.45))
                .flex()
                .justify_center()
                .items_start()
                .pt(px(72.))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close_adding(window, cx)),
                )
                .child(
                    div()
                        .id("adding")
                        .w(px((TILE + 8. + 12.) * COLUMNS as f32 + 20.))
                        .flex()
                        .flex_col()
                        .rounded_xl()
                        .border_1()
                        .border_color(rgb(0x2e2e2e))
                        .bg(rgb(0x191919))
                        .shadow_lg()
                        .overflow_hidden()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            this.adding_key(event, window, cx)
                        }))
                        .child(
                            div()
                                .px_2()
                                .py_1p5()
                                .border_b_1()
                                .border_color(rgb(BORDER))
                                .child(
                                    Input::new(&adding.query).appearance(false).prefix(
                                        Icon::new(IconName::Search).small().text_color(rgb(MUTED)),
                                    ),
                                ),
                        )
                        .child(
                            div()
                                .id("adding-grid")
                                .max_h(px(420.))
                                .overflow_y_scroll()
                                .p_3()
                                .when(empty, |el| {
                                    el.py_8()
                                        .flex()
                                        .justify_center()
                                        .text_sm()
                                        .text_color(rgb(FAINT))
                                        .child("No media matches.")
                                })
                                .when(!empty, |el| {
                                    el.child(div().flex().flex_wrap().gap_3().children(tiles))
                                }),
                        )
                        .child(
                            div()
                                .px_4()
                                .py_2()
                                .border_t_1()
                                .border_color(rgb(BORDER))
                                .text_xs()
                                .text_color(rgb(FAINT))
                                .child("Arrows choose · Enter adds at the playhead · Shift+Enter inserts · Esc closes"),
                        ),
                )
                .into_any_element(),
        )
    }
}
