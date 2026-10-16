//! Adding media from the keyboard: Command+P, "Add media…", then type to
//! search the project's footage, audio, images, and canvases. Tab (or Down)
//! moves into the grid, where the arrow keys choose; Enter places the chosen
//! one at the playhead (over what is there; Shift+Enter inserts, pushing
//! later clips along). Shift+Tab, Up from the top row, or typing goes back
//! to the search.
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
    /// The grid, which takes the arrow keys once Tab moves to it.
    pub grid: FocusHandle,
}

impl Workspace {
    pub fn open_adding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(video) = &mut self.video else {
            return;
        };
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search media"));
        query.update(cx, |state, cx| state.focus(window, cx));
        cx.observe(&query, |_, _, cx| cx.notify()).detach();
        // A tab stop, so Tab and Shift+Tab move between search and grid.
        let grid = cx.focus_handle().tab_stop(true);
        video.adding = Some(Adding {
            query,
            index: 0,
            grid,
        });
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

    /// Keys while the list is open: Tab and Down go from the search to the
    /// grid; in the grid the arrows choose; Enter adds; Escape closes.
    fn adding_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.adding_matches(cx).len();
        let Some(adding) = self.video.as_mut().and_then(|v| v.adding.as_mut()) else {
            return;
        };
        let in_grid = adding.grid.is_focused(window);
        let last = count.saturating_sub(1);
        let index = adding.index.min(last);
        let key = event.keystroke.key.as_str();
        let shift = event.keystroke.modifiers.shift;
        let to_search = |adding: &Adding, window: &mut Window, cx: &mut Context<Self>| {
            adding.query.update(cx, |state, cx| state.focus(window, cx));
        };
        match (in_grid, key) {
            (false, "tab") if !shift => {
                window.focus(&adding.grid);
                cx.stop_propagation();
            }
            (false, "down") => {
                window.focus(&adding.grid);
                cx.stop_propagation();
            }
            (true, "tab") if shift => {
                to_search(adding, window, cx);
                cx.stop_propagation();
            }
            (true, "up") if index < COLUMNS => {
                to_search(adding, window, cx);
                cx.stop_propagation();
            }
            (true, "left" | "right" | "up" | "down") => {
                adding.index = match key {
                    "left" => index.saturating_sub(1),
                    "right" => (index + 1).min(last),
                    "up" => index.saturating_sub(COLUMNS),
                    _ => (index + COLUMNS).min(last),
                };
                cx.stop_propagation();
            }
            (_, "enter") => {
                cx.stop_propagation();
                let placement = if shift {
                    Placement::Insert
                } else {
                    Placement::Overwrite
                };
                self.add_adding(index, placement, window, cx);
                return;
            }
            (_, "escape") => {
                cx.stop_propagation();
                self.close_adding(window, cx);
                return;
            }
            // Typing in the grid goes back to the search, with the key.
            (true, _)
                if event.keystroke.key_char.is_some() && !event.keystroke.modifiers.secondary() =>
            {
                let typed = event.keystroke.key_char.clone().unwrap_or_default();
                adding.query.update(cx, |state, cx| {
                    let text = format!("{}{typed}", state.value());
                    state.set_value(text, window, cx);
                    state.focus(window, cx);
                });
                adding.index = 0;
                cx.stop_propagation();
            }
            _ => {}
        }
        cx.notify();
    }

    /// Moves the choice one tile (or one row) in the grid; up from the top
    /// row goes back to the search.
    fn adding_step(&mut self, by: i32, row: bool, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.adding_matches(cx).len();
        let Some(adding) = self.video.as_mut().and_then(|v| v.adding.as_mut()) else {
            return;
        };
        let last = count.saturating_sub(1) as i32;
        let index = adding.index.min(last.max(0) as usize) as i32;
        if row && by < 0 && index < COLUMNS as i32 {
            adding.query.update(cx, |state, cx| state.focus(window, cx));
        } else {
            let step = if row { by * COLUMNS as i32 } else { by };
            adding.index = (index + step).clamp(0, last.max(0)) as usize;
        }
        cx.notify();
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
    pub fn adding_overlay(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let adding = self.video.as_ref()?.adding.as_ref()?;
        let thumbs = self.store.as_ref().ok().map(|s| &s.thumbs);
        let matches = self.adding_matches(cx);
        let chosen = adding.index.min(matches.len().saturating_sub(1));
        let grid_focused = adding.grid.is_focused(window);
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
                                .track_focus(&adding.grid)
                                // The arrows are the workspace's nudges;
                                // here they choose.
                                .on_action(cx.listener(|this, _: &crate::NudgeLeft, window, cx| {
                                    this.adding_step(-1, false, window, cx)
                                }))
                                .on_action(cx.listener(|this, _: &crate::NudgeRight, window, cx| {
                                    this.adding_step(1, false, window, cx)
                                }))
                                .on_action(cx.listener(|this, _: &crate::NudgeUp, window, cx| {
                                    this.adding_step(-1, true, window, cx)
                                }))
                                .on_action(cx.listener(|this, _: &crate::NudgeDown, window, cx| {
                                    this.adding_step(1, true, window, cx)
                                }))
                                .when(grid_focused, |el| el.bg(rgb(0x1e1e1e)))
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
                                .child(if grid_focused {
                                    "Arrows choose · Enter adds at the playhead · Shift+Enter inserts · Shift+Tab searches"
                                } else {
                                    "Type to search · Tab to choose with the arrows · Enter adds the first · Esc closes"
                                }),
                        ),
                )
                .into_any_element(),
        )
    }
}
