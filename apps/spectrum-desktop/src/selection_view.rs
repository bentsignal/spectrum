//! Selections on the canvas: the Marquee, Ellipse select, Lasso, and Magic
//! wand tools, marching ants around the selection, and what a selection can
//! do (fill, hide, mask, crop, invert). Each change is an engine command,
//! as `spectrum canvas selection`.
use crate::{tools::Tool, workspace::Workspace};
use gpui::{prelude::*, *};
use spectrum_assets::Service;
use spectrum_canvas::{
    Command, Selection, SelectionCombineMode, SelectionMaskOutline, SelectionOutlinePath,
    SelectionOutlinePoint, SelectionOutlineRect, SelectionOutlineTransform, SelectionOutlineView,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Shift adds, Option subtracts, both intersect; otherwise replace.
pub fn combine_mode(modifiers: Modifiers) -> SelectionCombineMode {
    match (modifiers.shift, modifiers.alt) {
        (true, true) => SelectionCombineMode::Intersect,
        (true, false) => SelectionCombineMode::Add,
        (false, true) => SelectionCombineMode::Subtract,
        (false, false) => SelectionCombineMode::Replace,
    }
}

/// A key that changes whenever the selection does.
fn selection_key(selection: &Selection) -> u64 {
    let (x, y, w, h) = selection.bounds();
    let mask = selection
        .alpha()
        .map_or(0, |a| a.as_ptr() as u64 ^ a.len() as u64);
    (u64::from(x) << 48) ^ (u64::from(y) << 32) ^ (u64::from(w) << 16) ^ u64::from(h) ^ mask
}

/// The selection's edges as paths in canvas pixels.
fn outline(selection: &Selection) -> Arc<[SelectionOutlinePath]> {
    let (x, y, w, h) = selection.bounds();
    let Some(alpha) = selection.alpha() else {
        let p = |x: u32, y: u32| SelectionOutlinePoint::new(x as f32, y as f32);
        return Arc::from([vec![
            p(x, y),
            p(x + w, y),
            p(x + w, y + h),
            p(x, y + h),
            p(x, y),
        ]]);
    };
    match spectrum_canvas::selection_mask_outline((x, y, w, h), alpha) {
        SelectionMaskOutline::Exact(paths) => paths,
        SelectionMaskOutline::Complex => spectrum_canvas::complex_selection_mask_outline(
            (x, y, w, h),
            alpha,
            SelectionOutlineView {
                x: 0,
                y: 0,
                width: w,
                height: h,
            },
        ),
    }
}

impl Workspace {
    /// Finishes a Marquee, Ellipse, or Lasso drag as a selection; a click
    /// without a drag clears the selection.
    pub fn finish_selection(
        &mut self,
        tool: Tool,
        start: (f32, f32),
        end: (f32, f32),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        let points = std::mem::take(&mut canvas.points);
        let mode = canvas.select_mode;
        let (w, h) = (canvas.doc.width as f32, canvas.doc.height as f32);
        let clamp = |(x, y): (f32, f32)| (x.clamp(0., w), y.clamp(0., h));
        let tiny = (end.0 - start.0).abs() < 1. && (end.1 - start.1).abs() < 1.;
        if tiny && tool != Tool::Lasso || tool == Tool::Lasso && points.len() < 3 {
            if mode == SelectionCombineMode::Replace && canvas.doc.selection.is_some() {
                self.deselect(window, cx);
            }
            return;
        }
        let path = match tool {
            Tool::Marquee => spectrum_canvas::rectangle_lasso(clamp(start), clamp(end)),
            Tool::EllipseSelect => spectrum_canvas::ellipse_lasso(start, end),
            _ => spectrum_canvas::LassoPath::new(
                points
                    .iter()
                    .map(|&(x, y)| spectrum_canvas::LassoPoint::from_canvas(x, y))
                    .collect::<anyhow::Result<Vec<_>>>()
                    .unwrap_or_default(),
            ),
        };
        match path {
            Ok(points) => self.canvas_commands(
                vec![Command::LassoSelection {
                    points,
                    mode,
                    antialias: true,
                }],
                window,
                cx,
            ),
            Err(error) => self.notify_error(error, window, cx),
        }
        self.animate_ants(window, cx);
    }

    /// Selects colors like the one at `at`, from the canvas as rendered,
    /// computed in the background; the result is saved as the selection.
    pub fn magic_wand(&mut self, at: (f32, f32), window: &mut Window, cx: &mut Context<Self>) {
        let (Some(canvas), Ok(store)) = (&mut self.canvas, &self.store) else {
            return;
        };
        if canvas.wand_busy {
            return;
        }
        let (x, y) = (at.0.floor(), at.1.floor());
        if x < 0. || y < 0. || x >= canvas.doc.width as f32 || y >= canvas.doc.height as f32 {
            // A click off the canvas clears the selection, as in Photoshop.
            if canvas.select_mode == SelectionCombineMode::Replace {
                self.deselect(window, cx);
            }
            return;
        }
        canvas.wand_busy = true;
        let (root, doc, id, mode) = (
            store.root.clone(),
            canvas.doc.clone(),
            canvas.id,
            canvas.select_mode,
        );
        let tolerance = self.canvas_ui.tool_options.tolerance(cx);
        let contiguous = self.canvas_ui.tool_options.contiguous;
        let task = cx.background_executor().spawn(async move {
            let mut resolved = doc.clone();
            Service::open(&root)?.resolve(&mut resolved)?;
            let picked = spectrum_canvas::magic_wand_selection(
                &resolved, x as u32, y as u32, tolerance, contiguous, true,
            )?;
            spectrum_canvas::combine_selections(doc.selection.as_ref(), &picked, mode)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(canvas) = this.canvas.as_mut().filter(|c| c.id == id) else {
                    return;
                };
                canvas.wand_busy = false;
                match result {
                    Ok(selection) => {
                        this.canvas_commands(vec![Command::SetSelection { selection }], window, cx);
                        this.animate_ants(window, cx);
                    }
                    Err(error) => this.notify_error(error, window, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn has_canvas_selection(&self) -> bool {
        self.canvas
            .as_ref()
            .is_some_and(|c| c.doc.selection.is_some())
    }

    /// Command+A on a canvas: select the whole canvas.
    pub fn select_canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(canvas) = &self.canvas else {
            return;
        };
        let selection = Selection::rectangle(0, 0, canvas.doc.width, canvas.doc.height);
        self.canvas_commands(
            vec![Command::SetSelection {
                selection: Some(selection),
            }],
            window,
            cx,
        );
        self.animate_ants(window, cx);
    }

    /// Command+D: drop the selection.
    pub fn deselect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_canvas_selection() {
            self.canvas_commands(vec![Command::SetSelection { selection: None }], window, cx);
        }
    }

    /// Command+Shift+I: select what is not selected.
    pub fn invert_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(canvas) = &self.canvas else {
            return;
        };
        match spectrum_canvas::inverted_selection(&canvas.doc) {
            Ok(selection) => self.canvas_commands(
                vec![Command::SetSelection {
                    selection: Some(selection),
                }],
                window,
                cx,
            ),
            Err(error) => self.notify_error(error, window, cx),
        }
        self.animate_ants(window, cx);
    }

    /// A new layer of the foreground color over the selection.
    pub fn fill_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_canvas_selection() {
            let color = self.colors.fore;
            self.canvas_commands(
                vec![Command::FillSelection {
                    color,
                    name: Some("Fill".into()),
                }],
                window,
                cx,
            );
        }
    }

    /// Masks the selected layer to the selection with a vector mask.
    pub fn mask_to_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(canvas) = &self.canvas else {
            return;
        };
        let Some(id) = canvas.selected else {
            return;
        };
        match spectrum_canvas::vector_mask_from_selection(&canvas.doc, id) {
            Ok(mask) => self.canvas_commands(
                vec![Command::SetVectorMask {
                    id,
                    mask: Some(mask),
                }],
                window,
                cx,
            ),
            Err(error) => self.notify_error(error, window, cx),
        }
    }

    /// Delete or Backspace with a selection: hide what it covers on the
    /// selected layer, or else the topmost layer under it, of any kind.
    /// Images hide those pixels; other layers keep it in their mask and
    /// stay editable.
    pub fn delete_in_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(canvas) = &self.canvas else {
            return;
        };
        let Some(selection) = &canvas.doc.selection else {
            return;
        };
        // The selected layer, or else the top one the selection covers.
        let (sx, sy, sw, sh) = selection.bounds();
        let covers = |id: u64| {
            canvas.bounds.get(&id).is_some_and(|(min, max)| {
                min[0] < (sx + sw) as f32
                    && max[0] > sx as f32
                    && min[1] < (sy + sh) as f32
                    && max[1] > sy as f32
            })
        };
        // The first, the selected layer before the rest from the top, with
        // something showing under the selection.
        let candidates = canvas.selected.filter(|id| covers(*id)).into_iter().chain(
            canvas
                .doc
                .layers
                .iter()
                .rev()
                .filter(|l| l.visible && !l.locked && covers(l.id))
                .map(|l| l.id),
        );
        let target = candidates.into_iter().find(|&id| {
            let mut trial = spectrum_canvas::Workspace::new(canvas.doc.clone());
            trial.execute(Command::HideSelection { id }).is_ok()
        });
        let Some(layer) = target.and_then(|id| canvas.doc.layer(id).ok()) else {
            return self.notify_error(
                anyhow::anyhow!("Nothing under the selection to hide."),
                window,
                cx,
            );
        };
        let id = layer.id;
        self.canvas_commands(vec![Command::HideSelection { id }], window, cx);
    }

    /// Crops the canvas to the selection's bounds.
    pub fn crop_to_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_canvas_selection() {
            self.canvas_commands(vec![Command::CropToSelection], window, cx);
        }
    }

    /// Keeps the marching ants moving while there is a selection.
    pub fn animate_ants(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        if canvas.ants_running {
            return;
        }
        canvas.ants_running = true;
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let going = this
                    .update(cx, |this, cx| {
                        let selected = this.has_canvas_selection();
                        if let Some(canvas) = &mut this.canvas {
                            canvas.ants_running = selected;
                        }
                        cx.notify();
                        selected
                    })
                    .unwrap_or(false);
                if !going {
                    break;
                }
            }
        })
        .detach();
    }

    /// The selection's marching ants and any selection being drawn.
    pub fn selection_overlay(&self, offset: Point<Pixels>, scale: f32) -> Option<AnyElement> {
        let canvas = self.canvas.as_ref()?;
        let current = canvas.doc.selection.as_ref().map(|selection| {
            let key = selection_key(selection);
            let mut cache = canvas.ants.borrow_mut();
            match &*cache {
                Some((cached, paths)) if *cached == key => paths.clone(),
                _ => {
                    let paths = outline(selection);
                    *cache = Some((key, paths.clone()));
                    paths
                }
            }
        });
        // What is being drawn: a box, an ellipse, or the lasso so far.
        let drawing: Option<Vec<(f32, f32)>> =
            canvas.creating.and_then(|(a, b)| match canvas.tool {
                Tool::Marquee => Some(vec![a, (b.0, a.1), b, (a.0, b.1), a]),
                Tool::EllipseSelect => {
                    let center = ((a.0 + b.0) / 2., (a.1 + b.1) / 2.);
                    let radius = ((b.0 - a.0).abs() / 2., (b.1 - a.1).abs() / 2.);
                    Some(
                        (0..=64)
                            .map(|i| {
                                let t = i as f32 / 64. * std::f32::consts::TAU;
                                (center.0 + radius.0 * t.cos(), center.1 + radius.1 * t.sin())
                            })
                            .collect(),
                    )
                }
                Tool::Lasso => Some(canvas.points.clone()),
                _ => None,
            });
        if current.is_none() && drawing.is_none() {
            return None;
        }
        let phase = (Instant::now()
            .duration_since(*START.get_or_init(Instant::now))
            .as_millis()
            / 120) as f32
            * 2.;
        Some(
            gpui::canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let origin = bounds.origin + offset;
                    let transform = SelectionOutlineTransform {
                        scale,
                        offset: SelectionOutlinePoint::new(origin.x.into(), origin.y.into()),
                    };
                    let clip = SelectionOutlineRect::new(
                        SelectionOutlinePoint::new(bounds.origin.x.into(), bounds.origin.y.into()),
                        SelectionOutlinePoint::new(
                            f32::from(bounds.origin.x + bounds.size.width),
                            f32::from(bounds.origin.y + bounds.size.height),
                        ),
                    );
                    let mut paths: Vec<SelectionOutlinePath> =
                        current.iter().flat_map(|p| p.iter().cloned()).collect();
                    if let Some(points) = &drawing {
                        paths.push(
                            points
                                .iter()
                                .map(|&(x, y)| SelectionOutlinePoint::new(x, y))
                                .collect(),
                        );
                    }
                    let frame =
                        spectrum_canvas::marching_ants_frame(&paths, transform, clip, phase, 4.);
                    for (segments, color) in [
                        (&frame.contrast, hsla(0., 0., 0., 0.85)),
                        (&frame.light, hsla(0., 0., 1., 0.95)),
                    ] {
                        let mut path = PathBuilder::stroke(px(1.));
                        for segment in segments.iter() {
                            path.move_to(point(px(segment.start.x), px(segment.start.y)));
                            path.line_to(point(px(segment.end.x), px(segment.end.y)));
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color);
                        }
                    }
                },
            )
            .absolute()
            .size_full()
            .into_any_element(),
        )
    }
}

static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
