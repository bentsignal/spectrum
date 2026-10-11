//! The History mode: the open image's or canvas's tree of revisions, which
//! one the person is on, and where each session (theirs, an agent's) is.
//! Clicking a revision selects it; going there moves the person, and their
//! next edit branches from it. Watching a session follows it as it works.
//! The tree pans and zooms like a canvas; zoomed in, revisions show
//! previews.
use crate::{
    history_layout::{Layout, layout},
    workspace::{Mode, Open, Workspace},
};
use gpui::*;
use gpui_component::input::InputState;
use spectrum_assets::Service;
use spectrum_document::{History, RevisionId, SessionId};
use spectrum_library::AssetId;
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

/// A column (one step) and a row (one branch) at zoom 1, in points.
pub const COLUMN: f32 = 44.;
pub const ROW: f32 = 40.;
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 6.;
/// Columns at least this wide show previews instead of dots.
pub const PREVIEWS_FROM: f32 = 96.;
/// Preview size, in pixels on the long edge.
pub const PREVIEW: u32 = 192;
/// Previews kept at once.
const PREVIEWS_KEPT: usize = 400;

/// The shown asset's history and how it lays out.
pub struct Shown {
    pub asset: AssetId,
    pub history: History,
    pub layout: Arc<Layout>,
}

/// Following a session as it works: who, and where the person was put last,
/// so the person moving or editing on their own ends it.
#[derive(Clone)]
pub struct Watching {
    pub asset: AssetId,
    pub session: SessionId,
    pub name: String,
    pub placed: Option<RevisionId>,
}

pub struct HistoryView {
    pub shown: Option<Shown>,
    loading: bool,
    /// Asked to load again while loading.
    again: bool,
    pub zoom: f32,
    /// Where the first revision's column and row start, from the view's
    /// top-left, in points.
    pub pan: (f32, f32),
    /// Whether the view has been placed on the person's revision.
    placed: bool,
    /// A press on the tree: where, the pan then, and whether it moved.
    pressed: Option<(Point<Pixels>, (f32, f32), bool)>,
    pub selected: Option<RevisionId>,
    pub search: Entity<InputState>,
    pub name: Entity<InputState>,
    pub matches: Vec<RevisionId>,
    pub previews: HashMap<RevisionId, Arc<RenderImage>>,
    preview_order: Vec<RevisionId>,
    preview_busy: bool,
    pub bounds: Rc<RefCell<Bounds<Pixels>>>,
    pub watching: Option<Watching>,
}

impl HistoryView {
    pub fn new(search: Entity<InputState>, name: Entity<InputState>) -> Self {
        Self {
            shown: None,
            loading: false,
            again: false,
            zoom: 1.,
            pan: (0., 0.),
            placed: false,
            pressed: None,
            selected: None,
            search,
            name,
            matches: Vec::new(),
            previews: HashMap::new(),
            preview_order: Vec::new(),
            preview_busy: false,
            bounds: Rc::new(RefCell::new(Bounds::default())),
            watching: None,
        }
    }

    /// Forgets the history shown, for another asset.
    pub fn reset(&mut self) {
        for (_, image) in self.previews.drain() {
            crate::retire::drop_later(image);
        }
        self.preview_order.clear();
        self.shown = None;
        self.placed = false;
        self.selected = None;
        self.matches.clear();
        self.zoom = 1.;
        self.pan = (0., 0.);
    }

    /// Where a revision's dot is, from the view's top-left, in points.
    pub fn place_of(&self, depth: u32, lane: u32) -> (f32, f32) {
        (
            self.pan.0 + depth as f32 * COLUMN * self.zoom,
            self.pan.1 + lane as f32 * ROW * self.zoom,
        )
    }
}

impl Workspace {
    /// The open asset with a history: an image or canvas.
    pub fn history_asset(&self) -> Option<AssetId> {
        match self.open {
            Open::Image(id) | Open::Canvas(id) => Some(id),
            Open::Overview => None,
        }
    }

    /// Reads the open asset's history off the main thread and lays it out.
    pub fn load_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(id), Ok(store)) = (self.history_asset(), &self.store) else {
            return;
        };
        if self.history.shown.as_ref().is_some_and(|s| s.asset != id) {
            self.history.reset();
        }
        if self.history.loading {
            self.history.again = true;
            return;
        }
        self.history.loading = true;
        let root = store.root.clone();
        let task = cx.background_executor().spawn(async move {
            let started = std::time::Instant::now();
            let history = Service::open(&root)?.history(id)?;
            let layout = Arc::new(layout(&history));
            crate::perf::record("history_load", started.elapsed());
            anyhow::Ok((history, layout))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.history.loading = false;
                match result {
                    Ok((history, layout)) if this.history_asset() == Some(id) => {
                        let current = history.current;
                        this.history.shown = Some(Shown {
                            asset: id,
                            history,
                            layout,
                        });
                        if !this.history.placed {
                            this.history.placed = true;
                            this.history.selected = Some(current);
                            this.center_on_revision(current);
                            this.sync_history_name(window, cx);
                            this.request_history_preview(current, window, cx);
                        }
                        this.update_history_matches(cx);
                    }
                    Ok(_) => {}
                    Err(error) => this.notify_error(error, window, cx),
                }
                if std::mem::take(&mut this.history.again) {
                    this.load_history(window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Keeps part of the tree on screen, however far it is panned.
    fn clamp_history_pan(&mut self) {
        let area = *self.history.bounds.borrow();
        let view = &mut self.history;
        let Some(shown) = &view.shown else {
            return;
        };
        let (width, height) = (f32::from(area.size.width), f32::from(area.size.height));
        let wide = shown.layout.depth as f32 * COLUMN * view.zoom;
        let tall = shown.layout.lanes.saturating_sub(1) as f32 * ROW * view.zoom;
        let keep = 60.;
        view.pan.0 = view
            .pan
            .0
            .clamp(keep - wide, (width - keep).max(keep - wide));
        view.pan.1 = view
            .pan
            .1
            .clamp(keep - tall, (height - keep).max(keep - tall));
    }

    /// Puts a revision in the middle of the view.
    pub fn center_on_revision(&mut self, id: RevisionId) {
        let area = *self.history.bounds.borrow();
        let view = &mut self.history;
        let Some(node) = view.shown.as_ref().and_then(|s| s.layout.node(id)) else {
            return;
        };
        let (width, height) = (f32::from(area.size.width), f32::from(area.size.height));
        let (width, height) = if width < 1. {
            (800., 600.)
        } else {
            (width, height)
        };
        view.pan = (
            width / 2. - node.depth as f32 * COLUMN * view.zoom,
            height / 2. - node.lane as f32 * ROW * view.zoom,
        );
    }

    /// Selects a revision, showing its details in the sidebar.
    pub fn select_revision(&mut self, id: RevisionId, window: &mut Window, cx: &mut Context<Self>) {
        self.history.selected = Some(id);
        self.sync_history_name(window, cx);
        self.request_history_preview(id, window, cx);
        cx.notify();
    }

    /// Fills the name field with the selected revision's name.
    pub fn sync_history_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .history
            .selected
            .zip(self.history.shown.as_ref())
            .and_then(|(id, shown)| shown.layout.node(id).and_then(|n| n.name.clone()))
            .unwrap_or_default();
        self.history
            .name
            .update(cx, |input, cx| input.set_value(name, window, cx));
    }

    /// Moves the person to a revision; the editor shows it, and their next
    /// edit branches from it. Ends any watching.
    pub fn go_to_revision(&mut self, id: RevisionId, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(asset), Ok(store)) = (self.history_asset(), &self.store) else {
            return;
        };
        self.history.watching = None;
        let root = store.root.clone();
        let task = cx.background_executor().spawn(async move {
            let started = std::time::Instant::now();
            Service::open(&root)?.move_to(asset, id)?;
            crate::perf::record("history_jump", started.elapsed());
            anyhow::Ok(())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    return this.notify_error(error, window, cx);
                }
                this.show_moved_asset(asset, window, cx);
                this.load_history(window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Shows the open asset again after the person moved in its history.
    pub fn show_moved_asset(
        &mut self,
        asset: AssetId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.open {
            Open::Canvas(id) if id == asset => self.show_saved_canvas(window, cx),
            Open::Image(id) if id == asset => self.refresh_open_image(id, window, cx),
            _ => {}
        }
    }

    /// Names the selected revision from the name field (blank clears it).
    pub fn name_selected_revision(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(asset), Some(revision), Ok(store)) =
            (self.history_asset(), self.history.selected, &self.store)
        else {
            return;
        };
        let name = self.history.name.read(cx).value().trim().to_string();
        let had = self
            .history
            .shown
            .as_ref()
            .and_then(|s| s.layout.node(revision))
            .and_then(|node| node.name.clone())
            .unwrap_or_default();
        if name == had {
            return;
        }
        let root = store.root.clone();
        let task = cx.background_executor().spawn(async move {
            let name = (!name.is_empty()).then_some(name);
            Service::open(&root)?.name_revision(asset, revision, name.as_deref())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) => this.load_history(window, cx),
                Err(error) => this.notify_error(error, window, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Revisions matching the search: by name, description, or who.
    pub fn update_history_matches(&mut self, cx: &mut Context<Self>) {
        let query = self.history.search.read(cx).value().trim().to_lowercase();
        let view = &mut self.history;
        view.matches.clear();
        let (Some(shown), false) = (&view.shown, query.is_empty()) else {
            return;
        };
        let found = |text: &str| text.to_lowercase().contains(&query);
        view.matches = shown
            .layout
            .nodes
            .iter()
            .filter(|node| {
                node.name.as_deref().is_some_and(found)
                    || node.label.as_deref().is_some_and(found)
                    || found(&node.actor)
            })
            .map(|node| node.id)
            .collect();
    }

    /// Selects and centers the next match after the selected revision.
    pub fn next_history_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let matches = &self.history.matches;
        if matches.is_empty() {
            return;
        }
        let at = self
            .history
            .selected
            .and_then(|selected| matches.iter().position(|m| *m == selected))
            .map_or(0, |at| (at + 1) % matches.len());
        let id = matches[at];
        self.center_on_revision(id);
        self.select_revision(id, window, cx);
    }

    /// Starts following a session: the person goes where it is now, and
    /// along with it as it works, until they edit or move on their own.
    pub fn watch_session(
        &mut self,
        session: SessionId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(asset) = self.history_asset() else {
            return;
        };
        self.history.watching = Some(Watching {
            asset,
            session,
            name,
            placed: None,
        });
        self.followed_stamp = None;
        self.check_watching(window, cx);
    }

    pub fn stop_watching(&mut self, cx: &mut Context<Self>) {
        self.history.watching = None;
        cx.notify();
    }

    /// While watching: moves the person to the watched session's newest
    /// revision, or ends the watching once the person went elsewhere.
    pub fn check_watching(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(watching), Ok(store)) = (self.history.watching.clone(), &self.store) else {
            return;
        };
        if self.history_asset() != Some(watching.asset) {
            self.history.watching = None;
            return;
        }
        let root = store.root.clone();
        let task = cx.background_executor().spawn(async move {
            let mut service = Service::open(&root)?;
            let mine = service.session_place(watching.asset, service.person_session())?;
            if watching.placed.is_some_and(|placed| placed != mine) {
                // The person edited or moved since being put here.
                return anyhow::Ok(None);
            }
            let theirs = service.session_place(watching.asset, watching.session)?;
            if theirs != mine {
                service.move_to(watching.asset, theirs)?;
            }
            anyhow::Ok(Some((theirs, theirs != mine)))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(current) = this.history.watching.as_mut() else {
                    return;
                };
                match result {
                    Ok(Some((place, moved))) => {
                        current.placed = Some(place);
                        if moved {
                            this.show_moved_asset(watching.asset, window, cx);
                            if this.mode == Mode::History {
                                this.load_history(window, cx);
                            }
                        }
                    }
                    Ok(None) => this.history.watching = None,
                    Err(error) => {
                        this.history.watching = None;
                        this.notify_error(error, window, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Renders previews of revisions on screen, one at a time, when the
    /// tree is zoomed in far enough to show them.
    pub fn request_history_preview(
        &mut self,
        id: RevisionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(asset), Ok(store)) = (self.history_asset(), &self.store) else {
            return;
        };
        let view = &mut self.history;
        if view.preview_busy || view.previews.contains_key(&id) {
            return;
        }
        view.preview_busy = true;
        let root = store.root.clone();
        let task = cx.background_executor().spawn(async move {
            let started = std::time::Instant::now();
            let path = Service::open(&root)?.revision_preview(asset, id, PREVIEW)?;
            let image = image::open(path)?;
            crate::perf::record("history_preview", started.elapsed());
            anyhow::Ok(crate::preview::to_render_image(image).0)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let view = &mut this.history;
                view.preview_busy = false;
                if let Ok(image) = result {
                    view.previews.insert(id, image);
                    view.preview_order.push(id);
                    if view.preview_order.len() > PREVIEWS_KEPT {
                        let old = view.preview_order.remove(0);
                        if let Some(image) = view.previews.remove(&old) {
                            crate::retire::drop_later(image);
                        }
                    }
                }
                this.request_visible_previews(window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Asks for the preview of the next revision on screen without one.
    pub fn request_visible_previews(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode != Mode::History {
            return;
        }
        let area = *self.history.bounds.borrow();
        // The selected revision's preview first: the sidebar shows it.
        if let Some(selected) = self.history.selected
            && !self.history.previews.contains_key(&selected)
        {
            return self.request_history_preview(selected, window, cx);
        }
        if COLUMN * self.history.zoom < PREVIEWS_FROM {
            return;
        }
        let view = &self.history;
        let Some(shown) = &view.shown else {
            return;
        };
        let (width, height) = (f32::from(area.size.width), f32::from(area.size.height));
        let wanted = shown.layout.nodes.iter().find(|node| {
            let (x, y) = view.place_of(node.depth, node.lane);
            (-COLUMN..width + COLUMN).contains(&x)
                && (-ROW..height + ROW).contains(&y)
                && !view.previews.contains_key(&node.id)
        });
        if let Some(id) = wanted.map(|node| node.id) {
            self.request_history_preview(id, window, cx);
        }
    }

    /// Two-finger and wheel scrolls pan and zoom the tree, as on a canvas.
    pub fn history_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let Some(scroll) = self.gestures.read(event) else {
            return;
        };
        let origin = self.history.bounds.borrow().origin;
        let view = &mut self.history;
        match scroll.gesture {
            crate::gesture::Gesture::Pan => {
                view.pan = (view.pan.0 + scroll.dx, view.pan.1 + scroll.dy);
            }
            crate::gesture::Gesture::Zoom => {
                let zoom = (view.zoom * scroll.factor).clamp(MIN_ZOOM, MAX_ZOOM);
                // About the pointer: the point under it stays under it.
                let at = (
                    f32::from(event.position.x - origin.x),
                    f32::from(event.position.y - origin.y),
                );
                let ratio = zoom / view.zoom;
                view.pan = (
                    at.0 - (at.0 - view.pan.0) * ratio,
                    at.1 - (at.1 - view.pan.1) * ratio,
                );
                view.zoom = zoom;
            }
        }
        self.clamp_history_pan();
        self.request_visible_previews(window, cx);
        cx.notify();
    }

    pub fn history_press(&mut self, position: Point<Pixels>) {
        self.history.pressed = Some((position, self.history.pan, false));
    }

    pub fn history_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let view = &mut self.history;
        let Some((start, pan, moved)) = &mut view.pressed else {
            return;
        };
        let delta = position - *start;
        if !*moved && f32::from(delta.x).hypot(f32::from(delta.y)) < 3. {
            return;
        }
        *moved = true;
        view.pan = (pan.0 + f32::from(delta.x), pan.1 + f32::from(delta.y));
        self.clamp_history_pan();
        cx.notify();
    }

    /// A click (a press that did not move) selects the revision under it;
    /// a double click goes there.
    pub fn history_release(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, _, moved)) = self.history.pressed.take() else {
            return;
        };
        if moved {
            self.request_visible_previews(window, cx);
            return;
        }
        let Some(id) = self.revision_at(event.position) else {
            return;
        };
        if event.click_count >= 2 {
            self.go_to_revision(id, window, cx);
        } else {
            self.select_revision(id, window, cx);
        }
    }

    /// The revision drawn under a point on screen, if any.
    pub fn revision_at(&self, position: Point<Pixels>) -> Option<RevisionId> {
        let area = *self.history.bounds.borrow();
        let view = &self.history;
        let shown = view.shown.as_ref()?;
        let at = (
            f32::from(position.x - area.origin.x),
            f32::from(position.y - area.origin.y),
        );
        let reach = crate::history_paint::node_radius(view.zoom).max(8.);
        let reach = if COLUMN * view.zoom >= PREVIEWS_FROM {
            crate::history_paint::card_size(view.zoom).0 / 2.
        } else {
            reach
        };
        shown
            .layout
            .nodes
            .iter()
            .map(|node| {
                let (x, y) = view.place_of(node.depth, node.lane);
                (node.id, (x - at.0).hypot(y - at.1))
            })
            .filter(|(_, distance)| *distance <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }
}
