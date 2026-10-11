//! Every canvas tool, the layer controls, and every image slider, used as a
//! person uses them. Each must do what it is for, still be there after the
//! document is read back from disk, and undo and redo; each is timed until
//! it is drawn and saved. A feature that does nothing, is lost on reopening,
//! or will not undo fails the run like a slow one.
use super::{
    Fixtures, Outcome,
    scenarios::{
        This, canvas_drag_along, click, key, on_screen, outcome, pause, read, set_color, until,
    },
};
use crate::workspace::{Open, Workspace};
use gpui::*;
use spectrum_canvas::{Command, Document, LayerKind};
use std::time::Instant;

/// What a tool is given: a drag along canvas points, or clicks at them.
enum Input {
    Drag(Vec<(f32, f32)>),
    Clicks(Vec<(f32, f32)>),
}

/// A tool check: the keys that pick the tool, the input, keys after it, and
/// what must be true of the document after it, given the one before.
struct ToolCheck {
    name: &'static str,
    keys: &'static [&'static str],
    input: Input,
    after: &'static [&'static str],
    expect: fn(&Document, &Document) -> Result<(), String>,
    /// Selections are not undo steps and are not kept with the document.
    history: bool,
}

fn added(kind: fn(&LayerKind) -> bool) -> impl Fn(&Document, &Document) -> Result<(), String> {
    move |before, after| {
        let count = |doc: &Document| doc.layers.iter().filter(|l| kind(&l.kind)).count();
        if count(after) > count(before) {
            Ok(())
        } else {
            Err("added no layer of its kind".into())
        }
    }
}

fn selected(_: &Document, after: &Document) -> Result<(), String> {
    after
        .selection
        .is_some()
        .then_some(())
        .ok_or_else(|| "made no selection".into())
}

fn layers_changed(before: &Document, after: &Document) -> Result<(), String> {
    let json = |doc: &Document| serde_json::to_string(&doc.layers).unwrap_or_default();
    (json(before) != json(after))
        .then_some(())
        .ok_or_else(|| "changed no layer".into())
}

fn cropped(before: &Document, after: &Document) -> Result<(), String> {
    (after.width < before.width && after.height < before.height)
        .then_some(())
        .ok_or_else(|| "did not crop the canvas".into())
}

fn tool_checks() -> Vec<ToolCheck> {
    let square = vec![
        (300., 200.),
        (500., 200.),
        (500., 380.),
        (300., 380.),
        (300., 200.),
    ];
    vec![
        ToolCheck {
            name: "marquee",
            keys: &["m"],
            input: Input::Drag(vec![(200., 150.), (520., 400.)]),
            after: &[],
            expect: selected,
            history: false,
        },
        ToolCheck {
            name: "ellipse_select",
            keys: &["m", "shift-m"],
            input: Input::Drag(vec![(250., 180.), (560., 420.)]),
            after: &[],
            expect: selected,
            history: false,
        },
        ToolCheck {
            name: "lasso",
            keys: &["l"],
            input: Input::Drag(square.clone()),
            after: &[],
            expect: selected,
            history: false,
        },
        ToolCheck {
            name: "magic_wand",
            keys: &["w"],
            input: Input::Clicks(vec![(400., 300.)]),
            after: &[],
            expect: selected,
            history: false,
        },
        ToolCheck {
            name: "move",
            keys: &["v"],
            input: Input::Drag(vec![(1250., 320.), (1330., 400.)]),
            after: &[],
            expect: layers_changed,
            history: true,
        },
        ToolCheck {
            name: "brush",
            keys: &["b"],
            input: Input::Drag(vec![(200., 600.), (500., 650.), (800., 620.)]),
            after: &[],
            expect: layers_changed,
            history: true,
        },
        ToolCheck {
            name: "eraser",
            keys: &["e"],
            // Across the shape, where the Move check left it.
            input: Input::Drag(vec![(1200., 300.), (1400., 380.)]),
            after: &[],
            expect: layers_changed,
            history: true,
        },
        ToolCheck {
            name: "text",
            keys: &["t"],
            input: Input::Clicks(vec![(300., 800.)]),
            after: &["escape"],
            expect: |b, a| added(|k| matches!(k, LayerKind::Text { .. }))(b, a),
            history: true,
        },
        ToolCheck {
            name: "box",
            keys: &["u"],
            input: Input::Drag(vec![(600., 700.), (800., 850.)]),
            after: &[],
            expect: |b, a| added(|k| matches!(k, LayerKind::Rectangle { .. }))(b, a),
            history: true,
        },
        ToolCheck {
            name: "circle",
            keys: &["u", "shift-u"],
            input: Input::Drag(vec![(850., 700.), (1000., 850.)]),
            after: &[],
            expect: |b, a| added(|k| matches!(k, LayerKind::Ellipse { .. }))(b, a),
            history: true,
        },
        ToolCheck {
            name: "gradient",
            keys: &["g"],
            input: Input::Drag(vec![(100., 100.), (700., 500.)]),
            after: &[],
            expect: |b, a| {
                (a.layers.len() > b.layers.len())
                    .then_some(())
                    .ok_or_else(|| "added no gradient layer".into())
            },
            history: true,
        },
        ToolCheck {
            name: "pen",
            keys: &["p"],
            input: Input::Clicks(vec![
                (1100., 650.),
                (1300., 650.),
                (1300., 850.),
                (1100., 850.),
                (1100., 650.),
            ]),
            after: &[],
            expect: |b, a| added(|k| matches!(k, LayerKind::Path { .. }))(b, a),
            history: true,
        },
        ToolCheck {
            name: "crop",
            keys: &["c"],
            input: Input::Drag(vec![(40., 30.), (1560., 960.)]),
            after: &[],
            expect: cropped,
            history: true,
        },
    ]
}

/// What the canvas saved, read back from disk.
fn saved(
    this: &This,
    cx: &mut AsyncWindowContext,
    id: spectrum_library::AssetId,
) -> Option<Document> {
    let root = read(this, cx, |ws| {
        ws.store.as_ref().ok().map(|s| s.root.clone())
    })??;
    spectrum_assets::Service::open(&root)
        .ok()?
        .canvas_view(id)
        .ok()
        .map(|(doc, _)| doc)
}

fn doc(this: &This, cx: &mut AsyncWindowContext) -> Option<Document> {
    read(this, cx, |ws| ws.canvas.as_ref().map(|c| c.doc.clone()))?
}

/// What must match between the document on screen and the one on disk.
fn same(a: &Document, b: &Document) -> bool {
    let json = |doc: &Document| serde_json::to_string(&doc.layers).unwrap_or_default();
    json(a) == json(b) && a.width == b.width && a.height == b.height
}

/// Runs one change and checks it does what it should, is saved, and undoes
/// and redoes. Returns the problem, if any, and records how long it took to
/// be drawn and saved.
async fn check_change(
    this: &This,
    cx: &mut AsyncWindowContext,
    canvas: spectrum_library::AssetId,
    history: bool,
    expect: &dyn Fn(&Document, &Document) -> Result<(), String>,
    before: &Document,
    started: Instant,
) -> Result<(), String> {
    // Drawn, including a wand's selection worked out off the main thread.
    let shown =
        |ws: &Workspace| ws.canvas_drawn() && ws.canvas.as_ref().is_some_and(|c| !c.wand_busy);
    until(this, cx, 20_000, shown)
        .await
        .ok_or("was not drawn within 20 s")?;
    crate::perf::record("feature_shown", started.elapsed());
    until(this, cx, 20_000, Workspace::canvas_idle)
        .await
        .ok_or("was not saved within 20 s")?;
    crate::perf::record("feature_saved", started.elapsed());
    let after = doc(this, cx).ok_or("the canvas closed")?;
    expect(before, &after)?;
    if !history {
        return Ok(());
    }
    let disk = saved(this, cx, canvas).ok_or("the saved canvas could not be read")?;
    if !same(&after, &disk) {
        return Err("what was saved differs from what is shown".into());
    }
    key(cx, "secondary-z");
    until(this, cx, 20_000, Workspace::canvas_idle).await;
    let undone = doc(this, cx).ok_or("the canvas closed")?;
    if !same(&undone, before) {
        return Err("undo did not restore the canvas".into());
    }
    key(cx, "secondary-shift-z");
    until(this, cx, 20_000, Workspace::canvas_idle).await;
    let redone = doc(this, cx).ok_or("the canvas closed")?;
    if !same(&redone, &after) {
        return Err("redo did not bring the change back".into());
    }
    Ok(())
}

/// Every canvas tool on the feature canvas.
pub(super) async fn canvas_tools(
    this: &This,
    cx: &mut AsyncWindowContext,
    fixtures: &Fixtures,
) -> Outcome {
    let (project, canvas) = (fixtures.project, fixtures.feature_canvas);
    this.update_in(cx, |ws, window, cx| {
        ws.enter_project(project, window, cx);
        ws.open_item(Open::Canvas(canvas), window, cx);
    })
    .ok();
    until(this, cx, 30_000, Workspace::canvas_idle).await;
    crate::perf::drain();
    let mut problems = Vec::new();
    // The Eyedropper takes the color under a click: here, the shape's.
    this.update_in(cx, |ws, _, _| ws.colors.fore = [0, 0, 0, 255])
        .ok();
    key(cx, "i");
    pause(cx, 250).await;
    until(this, cx, 10_000, |ws| {
        ws.canvas.as_ref().is_some_and(|c| !c.pixels_busy)
    })
    .await;
    if let Some(at) = read(this, cx, |ws| on_screen(ws, 1250., 450.)) {
        click(this, cx, at).await;
    }
    pause(cx, 50).await;
    let picked = read(this, cx, |ws| ws.colors.fore);
    if picked != Some([90, 160, 220, 255]) {
        problems.push(format!(
            "eyedropper: took {picked:?}, not the shape's color"
        ));
    }
    for check in tool_checks() {
        // From a clean start: no selection, the Move tool.
        this.update_in(cx, |ws, window, cx| {
            ws.canvas_commands(vec![Command::SetSelection { selection: None }], window, cx);
        })
        .ok();
        key(cx, "escape");
        key(cx, "v");
        until(this, cx, 10_000, Workspace::canvas_idle).await;
        for spec in check.keys {
            key(cx, spec);
        }
        // As long as a hand takes to reach the canvas after picking a tool.
        pause(cx, 250).await;
        let before = doc(this, cx).unwrap_or_else(|| Document::new("", 1, 1));
        match &check.input {
            Input::Drag(points) => {
                let points = points.clone();
                let screen = read(this, cx, |ws| {
                    points
                        .iter()
                        .map(|(x, y)| on_screen(ws, *x, *y))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
                let along = move |t: f32| {
                    let last = screen.len().saturating_sub(1).max(1) as f32;
                    let at = t * last;
                    let i = (at.floor() as usize).min(screen.len().saturating_sub(2));
                    let f = at - i as f32;
                    screen[i] + (screen[i + 1] - screen[i]) * f
                };
                canvas_drag_along(this, cx, 400, along).await;
            }
            Input::Clicks(points) => {
                for (x, y) in points {
                    if let Some(at) = read(this, cx, |ws| on_screen(ws, *x, *y)) {
                        click(this, cx, at).await;
                        pause(cx, 30).await;
                    }
                }
            }
        }
        for spec in check.after {
            key(cx, spec);
        }
        // Timed from the release or last click.
        let started = Instant::now();
        if let Err(problem) = check_change(
            this,
            cx,
            canvas,
            check.history,
            &check.expect,
            &before,
            started,
        )
        .await
        {
            problems.push(format!("{}: {problem}", check.name));
        }
    }
    // The layer controls, as the layer panel and style controls apply them.
    let layer = fixtures.feature_layer;
    let copy = doc(this, cx).map_or(0, |doc| doc.next_id);
    let controls: Vec<(&str, Command)> = vec![
        ("duplicate", Command::DuplicateLayer { id: layer }),
        ("reorder", Command::MoveLayer { id: copy, index: 0 }),
        (
            "clipping",
            Command::SetClipping {
                id: layer,
                enabled: true,
            },
        ),
        (
            "opacity",
            Command::SetOpacity {
                id: layer,
                opacity: 0.5,
            },
        ),
        (
            "visibility",
            Command::SetVisibility {
                id: layer,
                visible: false,
            },
        ),
        (
            "blend_mode",
            Command::SetBlendMode {
                id: layer,
                blend_mode: spectrum_canvas::BlendMode::Screen,
            },
        ),
        (
            "layer_style",
            Command::SetLayerStyle {
                id: layer,
                style: spectrum_canvas::LayerStyle {
                    drop_shadow: Some(spectrum_canvas::DropShadow::default()),
                    ..Default::default()
                },
            },
        ),
        (
            "rename",
            Command::RenameLayer {
                id: layer,
                name: "Renamed".into(),
            },
        ),
        (
            "lock",
            Command::SetLocked {
                id: layer,
                locked: true,
            },
        ),
        ("delete", Command::RemoveLayer { id: copy }),
    ];
    for (name, command) in controls {
        let before = doc(this, cx).unwrap_or_else(|| Document::new("", 1, 1));
        let started = Instant::now();
        this.update_in(cx, |ws, window, cx| {
            ws.canvas_commands(vec![command], window, cx)
        })
        .ok();
        if let Err(problem) =
            check_change(this, cx, canvas, true, &layers_changed, &before, started).await
        {
            problems.push(format!("{name}: {problem}"));
        }
    }
    outcome(
        "canvas_tools",
        vec![],
        (!problems.is_empty()).then(|| problems.join("; ")),
    )
}

/// Every image slider: each changes the image, is saved, and undoes.
pub(super) async fn image_sliders(
    this: &This,
    cx: &mut AsyncWindowContext,
    fixtures: &Fixtures,
) -> Outcome {
    let (project, photo) = (fixtures.project, fixtures.photo);
    this.update_in(cx, |ws, window, cx| {
        ws.enter_project(project, window, cx);
        ws.open_item(Open::Image(photo), window, cx);
    })
    .ok();
    until(this, cx, 20_000, Workspace::image_idle).await;
    crate::perf::drain();
    let mut problems = Vec::new();
    let root = read(this, cx, |ws| {
        ws.store.as_ref().ok().map(|s| s.root.clone())
    })
    .flatten();
    for (index, field) in crate::color_fields::FIELDS.iter().enumerate() {
        let before = read(this, cx, |ws| ws.image.adjust.clone()).unwrap_or_default();
        let current = crate::color_fields::get(&before, field.key, 0, 0);
        let value = if current < (field.min + field.max) / 2. {
            field.max * 0.6 + field.min * 0.4
        } else {
            field.min * 0.6 + field.max * 0.4
        };
        let started = Instant::now();
        set_color(this, cx, index, value);
        if until(this, cx, 20_000, Workspace::image_idle_view)
            .await
            .is_none()
        {
            problems.push(format!("{}: never drawn", field.label));
            continue;
        }
        crate::perf::record("feature_shown", started.elapsed());
        if until(this, cx, 20_000, Workspace::image_idle)
            .await
            .is_none()
        {
            problems.push(format!("{}: never saved", field.label));
            continue;
        }
        crate::perf::record("feature_saved", started.elapsed());
        let after = read(this, cx, |ws| ws.image.adjust.clone()).unwrap_or_default();
        if after == before {
            problems.push(format!("{}: did not change the image", field.label));
            continue;
        }
        let disk = root.as_ref().and_then(|root| {
            spectrum_assets::Service::open(root)
                .ok()?
                .image(photo)
                .ok()
                .map(|image| image.adjustments)
        });
        if disk.as_ref() != Some(&after) {
            problems.push(format!("{}: what was saved differs", field.label));
        }
        key(cx, "secondary-z");
        until(this, cx, 20_000, Workspace::image_idle).await;
        let undone = read(this, cx, |ws| ws.image.adjust.clone()).unwrap_or_default();
        if undone != before {
            problems.push(format!("{}: undo did not restore it", field.label));
        }
        key(cx, "secondary-shift-z");
        until(this, cx, 20_000, Workspace::image_idle).await;
        let redone = read(this, cx, |ws| ws.image.adjust.clone()).unwrap_or_default();
        if redone != after {
            problems.push(format!("{}: redo did not bring it back", field.label));
        }
    }
    outcome(
        "image_sliders",
        vec![],
        (!problems.is_empty()).then(|| problems.join("; ")),
    )
}
