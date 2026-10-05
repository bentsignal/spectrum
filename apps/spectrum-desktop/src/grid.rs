//! Asset grids with multi-selection, shared by Home and project overviews.
use crate::{
    library::{described, empty},
    store::{Entry, Thumb},
    theme::*,
    workspace::{LibraryView, Open, Place, Workspace},
};
use gpui::{prelude::*, *};
use gpui_component::{
    IconName,
    button::Button,
    menu::{ContextMenuExt, PopupMenu, PopupMenuItem},
};
use spectrum_assets::Service;
use spectrum_library::AssetId;

/// Long edge of library thumbnails, enough for the largest card on Retina.
const THUMBNAIL: u32 = 720;
/// Width of an asset card.
pub const CARD_WIDTH: f32 = 196.;
const GAP: f32 = 16.;
const FADE: std::time::Duration = std::time::Duration::from_millis(180);
const PADDING: f32 = 24.;

/// Width of a centered block of `item`-wide columns that fits in `available`.
pub fn block_width(available: f32, item: f32) -> f32 {
    let columns = ((available + GAP) / (item + GAP)).floor().max(1.);
    columns * item + (columns - 1.) * GAP
}

impl Workspace {
    /// Entries after search, kind filters, and sorting, in display order.
    pub fn visible(&self, cx: &App) -> Vec<&Entry> {
        let Ok(store) = &self.store else {
            return Vec::new();
        };
        let query = self.search.read(cx).value().to_lowercase();
        let mut items: Vec<&Entry> = store
            .entries
            .iter()
            .filter(|e| {
                let kind = if e.asset.kind == "canvas" {
                    self.show_canvases
                } else {
                    self.show_images
                };
                kind && e.asset.name.to_lowercase().contains(&query)
            })
            .collect();
        match self.sort {
            1 => items.sort_by_key(|e| e.asset.name.to_lowercase()),
            2 => items.sort_by_key(|e| (e.asset.kind.clone(), e.asset.name.to_lowercase())),
            _ => items.sort_by_key(|e| (std::cmp::Reverse(e.added), e.asset.name.to_lowercase())),
        }
        items
    }

    fn visible_ids(&self, cx: &App) -> Vec<AssetId> {
        self.visible(cx).into_iter().map(|e| e.asset.id).collect()
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        if self.place == Place::Home && self.mode == crate::workspace::Mode::Projects {
            self.project_selection = self
                .store
                .as_ref()
                .map_or(Vec::new(), |s| s.projects.iter().map(|p| p.id).collect());
        } else {
            self.selection = self.visible_ids(cx);
        }
        cx.notify();
    }

    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selection.clear();
        self.anchor = None;
        self.project_selection.clear();
        self.project_anchor = None;
        cx.notify();
    }

    /// Removes assets from the grid at once, before the engine finishes.
    pub fn hide_assets(&mut self, ids: &[AssetId], cx: &mut Context<Self>) {
        if let Ok(store) = &mut self.store {
            store.entries.retain(|e| !ids.contains(&e.asset.id));
        }
        self.selection.retain(|id| !ids.contains(id));
        cx.notify();
    }

    /// Click selects one, Shift extends from the anchor, Command toggles.
    fn click_asset(
        &mut self,
        id: AssetId,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.modifiers();
        if event.click_count() == 2 && !modifiers.shift && !modifiers.secondary() {
            return self.open_asset(id, window, cx);
        }
        if modifiers.shift
            && let Some(anchor) = self.anchor
        {
            let ids = self.visible_ids(cx);
            let (Some(a), Some(b)) = (
                ids.iter().position(|x| *x == anchor),
                ids.iter().position(|x| *x == id),
            ) else {
                return;
            };
            self.selection = ids[a.min(b)..=a.max(b)].to_vec();
        } else if modifiers.secondary() {
            if let Some(index) = self.selection.iter().position(|x| *x == id) {
                self.selection.remove(index);
            } else {
                self.selection.push(id);
            }
            self.anchor = Some(id);
        } else {
            self.selection = vec![id];
            self.anchor = Some(id);
        }
        cx.notify();
    }

    fn open_asset(&mut self, id: AssetId, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(store) = &self.store else {
            return;
        };
        let Some(entry) = store.entries.iter().find(|e| e.asset.id == id) else {
            return;
        };
        match (self.place, entry.asset.kind.as_str()) {
            (_, _) if entry.purge_after.is_some() => {}
            (Place::Project(_), "image") => self.open_item(Open::Image(id), window, cx),
            (Place::Project(_), "canvas") => self.open_item(Open::Canvas(id), window, cx),
            _ => {}
        }
    }

    pub fn asset_grid(&self, available: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let store = match &self.store {
            Ok(store) => store,
            Err(error) => {
                return empty("Spectrum could not open your library", error.clone())
                    .into_any_element();
            }
        };
        let pending = self.pending_cards(self.card_width());
        if store.entries.is_empty() && pending.is_empty() {
            let detail = match self.view {
                LibraryView::Unassigned => "Every asset belongs to a project.",
                LibraryView::Trash => "Deleted assets stay here for 30 days.",
                LibraryView::All => "Import images, or drop them here.",
                LibraryView::Project(_) => "Import images into this project, or drop them here.",
            };
            return empty("No assets", detail.into())
                .when(
                    matches!(self.view, LibraryView::All | LibraryView::Project(_)),
                    |el| {
                        el.child(
                            Button::new("empty-import")
                                .icon(IconName::Plus)
                                .label("Import assets")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.choose_import(window, cx)
                                })),
                        )
                    },
                )
                .into_any_element();
        }
        let items = self.visible(cx);
        if items.is_empty() && pending.is_empty() {
            return empty("No matches", "Try another search or filter.".into()).into_any_element();
        }
        let width = self.card_width();
        self.card_bounds.borrow_mut().clear();
        let cards: Vec<AnyElement> = pending
            .into_iter()
            .chain(items.into_iter().map(|entry| {
                let id = entry.asset.id;
                let menu = self.asset_menu(id, cx);
                let selected = self.selection.contains(&id);
                div()
                    .id(SharedString::from(format!("card-{id}")))
                    .child(
                        self.card(entry, store.thumbs.get(&id), width, selected, cx)
                            .context_menu(menu),
                    )
                    .into_any_element()
            }))
            .collect();
        let block = block_width(available - PADDING * 2., width + 8.);
        let grid_bounds = self.grid_bounds.clone();
        div()
            .id("grid-wrap")
            .relative()
            .size_full()
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, _, _| *grid_bounds.borrow_mut() = bounds,
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .id("library-grid")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.grid_scroll)
                    .px(px(PADDING))
                    .pt_2()
                    .pb_8()
                    .child(
                        div()
                            .w(px(block))
                            .mx_auto()
                            .flex()
                            .flex_wrap()
                            .gap_x(px(GAP))
                            .gap_y_6()
                            .children(cards),
                    ),
            )
            .children(self.marquee_area().map(|area| {
                let origin = self.grid_bounds.borrow().origin;
                div()
                    .absolute()
                    .left(area.origin.x - origin.x)
                    .top(area.origin.y - origin.y)
                    .w(area.size.width)
                    .h(area.size.height)
                    .rounded_sm()
                    .bg(hsla(0., 0., 1., 0.06))
                    .border_1()
                    .border_color(hsla(0., 0., 1., 0.35))
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    let additive = event.modifiers.shift || event.modifiers.secondary();
                    this.start_marquee(event.position, additive, cx)
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    this.move_marquee(event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_marquee(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_marquee(cx)),
            )
            .into_any_element()
    }

    /// Starts background renders for thumbnails and covers not requested yet.
    pub fn request_thumbnails(&mut self, cx: &mut Context<Self>) {
        let Ok(store) = &mut self.store else {
            return;
        };
        let missing: Vec<AssetId> = store
            .entries
            .iter()
            .map(|e| e.asset.id)
            .chain(store.covers.values().copied())
            .chain(self.picker_assets.iter().map(|a| a.id))
            .filter(|id| !store.thumbs.contains_key(id))
            .collect();
        for id in missing {
            store.thumbs.insert(id, Thumb::Loading);
            let root = store.root.clone();
            let render = cx
                .background_executor()
                .spawn(async move { card_crop(&Service::open(&root)?.thumbnail(id, THUMBNAIL)?) });
            cx.spawn(async move |this, cx| {
                let thumb = match render.await {
                    Ok(path) => Thumb::ready(path),
                    Err(_) => Thumb::Failed,
                };
                this.update(cx, |this, cx| {
                    if let Ok(store) = &mut this.store {
                        store.thumbs.insert(id, thumb);
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// Right-click menu. It acts on the whole selection when the asset is part
    /// of it, otherwise on that asset alone.
    fn asset_menu(
        &self,
        id: AssetId,
        cx: &mut Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let view = cx.entity();
        move |menu, _, cx| {
            let menu = menu.min_w(px(280.));
            let (ids, current, place, name, images, copied) = {
                let this = view.read(cx);
                let Ok(store) = &this.store else {
                    return menu;
                };
                let ids = if this.selection.contains(&id) {
                    this.selection.clone()
                } else {
                    vec![id]
                };
                let place = this.place;
                let name = store
                    .entries
                    .iter()
                    .find(|e| e.asset.id == id)
                    .map(|e| e.asset.name.clone())
                    .unwrap_or_default();
                let images: Vec<AssetId> = ids
                    .iter()
                    .copied()
                    .filter(|id| store.is_image(*id))
                    .collect();
                let copied = this.copied_edits.is_some();
                (ids, this.view, place, name, images, copied)
            };
            let count = ids.len();
            let noun = |n: usize| {
                if n == 1 {
                    "asset".to_string()
                } else {
                    format!("{n} assets")
                }
            };
            if current == LibraryView::Trash {
                let view = view.clone();
                return menu.item(
                    PopupMenuItem::element(move |_, _| {
                        described(
                            format!("Restore {}", noun(count)),
                            "Returns it to its projects and canvases.",
                            TEXT,
                        )
                    })
                    .on_click(move |_, window, cx| {
                        let ids = ids.clone();
                        view.update(cx, |this, cx| {
                            this.change(window, cx, |store| {
                                ids.iter()
                                    .try_for_each(|id| store.service.restore(*id).map(|_| ()))
                            })
                        })
                    }),
                );
            }
            let menu = if count == 1 {
                let view = view.clone();
                let export = view.clone();
                menu.item(
                    PopupMenuItem::new("Export…").on_click(move |_, window, cx| {
                        export.update(cx, |this, cx| this.export_asset(id, window, cx))
                    }),
                )
                .item(
                    PopupMenuItem::new("Rename…").on_click(move |_, window, cx| {
                        let name = name.clone();
                        view.update(cx, |this, cx| {
                            this.open_rename(
                                "Rename asset",
                                name,
                                move |store, name| store.service.rename(id, name).map(|_| ()),
                                window,
                                cx,
                            )
                        })
                    }),
                )
            } else {
                menu
            };
            let menu = {
                let view = view.clone();
                let ids = ids.clone();
                menu.item(
                    PopupMenuItem::new("Add to project…").on_click(move |_, window, cx| {
                        let ids = ids.clone();
                        view.update(cx, |this, cx| this.open_add_to_project(ids, window, cx))
                    }),
                )
            };
            let menu = if images.is_empty() {
                menu
            } else {
                let copy = view.clone();
                let paste = view.clone();
                let single = (count == 1).then_some(id);
                menu.separator()
                    .when_some(single, |menu, id| {
                        menu.item(PopupMenuItem::new("Copy edits").on_click(
                            move |_, window, cx| {
                                copy.update(cx, |this, cx| this.copy_edits(Some(id), window, cx))
                            },
                        ))
                    })
                    .when(copied, |menu| {
                        let label = match images.len() {
                            1 => "Paste edits".to_string(),
                            n => format!("Paste edits onto {n} images"),
                        };
                        menu.item(
                            PopupMenuItem::element(move |_, _| {
                                described(
                                    label.clone(),
                                    "Replaces their edits, crop included.",
                                    TEXT,
                                )
                            })
                            .on_click({
                                let images = images.clone();
                                move |_, window, cx| {
                                    let images = images.clone();
                                    paste.update(cx, |this, cx| {
                                        this.paste_edits(Some(images), window, cx)
                                    })
                                }
                            }),
                        )
                    })
            };
            let menu = match place {
                Place::Project(project) => {
                    let view = view.clone();
                    let ids = ids.clone();
                    menu.item(
                        PopupMenuItem::element(move |_, _| {
                            described(
                                format!("Remove {} from project", noun(count)),
                                "They stay in your library.",
                                TEXT,
                            )
                        })
                        .on_click(move |_, window, cx| {
                            let ids = ids.clone();
                            view.update(cx, |this, cx| {
                                this.change(window, cx, |store| {
                                    store.service.library.remove_from_project(project, &ids)
                                })
                            })
                        }),
                    )
                }
                Place::Home => menu,
            };
            let view = view.clone();
            menu.separator().item(
                PopupMenuItem::element(move |_, _| {
                    described(
                        format!("Delete {}", noun(count)),
                        "Moves to the trash. Spectrum deletes permanently after 30 days.",
                        DANGER,
                    )
                })
                .on_click(move |_, window, cx| {
                    let ids = ids.clone();
                    view.update(cx, |this, cx| this.confirm_delete(ids, window, cx))
                }),
            )
        }
    }

    fn card(
        &self,
        entry: &Entry,
        thumb: Option<&Thumb>,
        width: f32,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = entry.asset.id;
        let detail: SharedString = match entry.purge_after {
            Some(purge_after) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64);
                let days = ((purge_after - now) as f32 / 86_400.).ceil().max(0.) as i64;
                format!("{days} day{} left", if days == 1 { "" } else { "s" }).into()
            }
            None if entry.asset.kind == "canvas" => "Canvas".into(),
            None => "Image".into(),
        };
        let bounds = self.card_bounds.clone();
        div()
            .id(SharedString::from(format!("asset-{id}")))
            .relative()
            .w(px(width + 8.))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                canvas(
                    move |b, _, _| {
                        bounds.borrow_mut().insert(id, b);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .p(px(2.))
                    .rounded(px(11.))
                    .border_2()
                    .border_color(if selected {
                        rgb(0xd6d6d6).into()
                    } else {
                        transparent_black()
                    })
                    .when(!selected, |el| {
                        el.hover(|el| el.border_color(rgb(0x2e2e2e)))
                    })
                    .child(frame(thumb, width, width * 0.75)),
            )
            .child(
                div()
                    .px_1()
                    .flex()
                    .flex_col()
                    .child(div().text_sm().truncate().child(entry.asset.name.clone()))
                    .child(div().text_xs().text_color(rgb(FAINT)).child(detail)),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.click_asset(id, event, window, cx)
            }))
    }
}

/// Crops a rendered thumbnail to the 4:3 card shape. GPUI rounds the image
/// it paints, so a cover-fitted image larger than its card keeps square
/// corners; a pre-cropped image fills the card exactly and rounds with it.
fn card_crop(path: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let out = path.with_file_name(format!("{stem}-card.png"));
    if out.exists() {
        return Ok(out);
    }
    let image = image::open(path)?;
    let (w, h) = (image.width(), image.height());
    let (cw, ch) = if w * 3 > h * 4 {
        (h * 4 / 3, h)
    } else {
        (w, w * 3 / 4)
    };
    let cropped = image.crop_imm((w - cw) / 2, (h - ch) / 2, cw.max(1), ch.max(1));
    let temporary = out.with_file_name(format!("{}.png", AssetId::new_v4()));
    cropped.save(&temporary)?;
    std::fs::rename(&temporary, &out)?;
    Ok(out)
}

impl Workspace {
    pub fn card_width(&self) -> f32 {
        CARD_WIDTH
    }

    /// Placeholder cards for files still importing into the current view.
    fn pending_cards(&self, width: f32) -> Vec<AnyElement> {
        let shown = |project: Option<spectrum_library::ProjectId>| match (self.view, project) {
            (LibraryView::All, _) => true,
            (LibraryView::Unassigned, None) => true,
            (LibraryView::Project(view), Some(project)) => view == project,
            _ => false,
        };
        self.pending_imports
            .iter()
            .filter(|p| shown(p.project))
            .flat_map(|p| {
                p.names
                    .iter()
                    .enumerate()
                    .map(move |(i, name)| (p.token, i, name))
            })
            .map(|(token, index, name)| {
                div()
                    .id(SharedString::from(format!("pending-{token}-{index}")))
                    .w(px(width + 8.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().p(px(4.)).child(frame(None, width, width * 0.75)))
                    .child(
                        div()
                            .px_1()
                            .flex()
                            .flex_col()
                            .child(div().text_sm().truncate().child(name.clone()))
                            .child(div().text_xs().text_color(rgb(FAINT)).child("Importing…")),
                    )
                    .into_any_element()
            })
            .collect()
    }
}

/// A thumbnail frame of the given size, cropped to fill.
pub fn frame(thumb: Option<&Thumb>, width: f32, height: f32) -> Div {
    let frame = div()
        .w(px(width))
        .h(px(height))
        .rounded(px(7.))
        .overflow_hidden()
        .bg(rgb(SURFACE));
    match thumb {
        Some(Thumb::Ready(path, ready)) => {
            // GPUI clips to rectangles, so the image carries the rounding itself.
            let image = img(path.clone())
                .size_full()
                .rounded(px(7.))
                .object_fit(ObjectFit::Fill);
            // Fade in only renders that just finished, so revisiting a view
            // shows thumbnails immediately instead of flickering.
            if ready.elapsed() < FADE {
                frame.child(image.with_animation(
                    SharedString::from(path.to_string_lossy().to_string()),
                    Animation::new(FADE).with_easing(ease_in_out),
                    |image, delta| image.opacity(delta),
                ))
            } else {
                frame.child(image)
            }
        }
        Some(Thumb::Failed) => frame
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(rgb(FAINT))
            .child("No preview"),
        _ => frame,
    }
}
