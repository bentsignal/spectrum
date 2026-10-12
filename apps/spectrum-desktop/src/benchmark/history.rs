//! The History mode on a long history: the 24-megapixel photo edited 1,200
//! times across a dozen branches, with an agent working in its own session.
//! Opens the tree, pans and zooms it with two fingers, zooms in until
//! previews show, names the branch and finds it with the branch finder, goes to a
//! revision on another branch and back and forward, and watches the agent's
//! next edit arrive.
use super::{
    Outcome,
    scenarios::{This, ms, outcome, pause, until},
};
use crate::workspace::{Mode, Open, Workspace};
use anyhow::{Context as _, Result};
use gpui::*;
use spectrum_assets::Service;
use spectrum_document::{CollaborationMode, RevisionId, SessionId};
use spectrum_library::{AssetId, ProjectId};
use std::{path::Path, time::Instant};

/// Edits on the first line of work, and how branches split from it.
const MAIN: usize = 600;
const BRANCH_EVERY: usize = 50;
const BRANCH_LENGTH: usize = 45;

#[derive(Clone, Debug)]
pub struct HistoryFixture {
    pub image: AssetId,
    /// A revision on a branch other than the person's.
    pub elsewhere: RevisionId,
    /// The agent's separate session.
    pub agent: SessionId,
}

/// The photo again, as its own asset, with a long branched history.
pub(super) fn make(
    service: &mut Service,
    library: &Path,
    project: ProjectId,
    photo: &Path,
) -> Result<HistoryFixture> {
    let (_, assets) = service.import_into(vec![photo.to_path_buf()], Some(project))?;
    let image = assets
        .first()
        .context("the history photo was not imported")?
        .id;
    let edit = |service: &Service, step: usize| {
        let adjustments = spectrum_image::Adjustments {
            exposure: ((step % 40) as f32 - 20.) / 40.,
            contrast: ((step * 7) % 50) as f32 - 25.,
            ..Default::default()
        };
        service.set_adjustments(image, adjustments)
    };
    let mut main = Vec::with_capacity(MAIN);
    for step in 0..MAIN {
        edit(service, step)?;
        main.push(service.history(image)?.current);
    }
    let mut elsewhere = None;
    for (branch, from) in main.iter().step_by(BRANCH_EVERY).skip(1).enumerate() {
        service.move_to(image, *from)?;
        for step in 0..BRANCH_LENGTH {
            edit(service, 1000 + branch * 100 + step)?;
        }
        let tip = service.history(image)?.current;
        if branch % 3 == 0 {
            service.name_revision(image, tip, Some(&format!("Look {}", branch + 1)))?;
        }
        elsewhere.get_or_insert(tip);
    }
    service.name_revision(image, main[MAIN / 2], Some("Halfway checkpoint"))?;
    service.move_to(image, main[MAIN - 1])?;
    // An agent working separately from the middle of the main line.
    let agent = Service::agent(library)?;
    let started = agent.start_agent(image, CollaborationMode::Separate, Some("Helper"))?;
    let agent = agent.with_session(Some(started.agent_session))?;
    for step in 0..20 {
        let adjustments = spectrum_image::Adjustments {
            saturation: step as f32,
            ..Default::default()
        };
        agent.set_adjustments(image, adjustments)?;
    }
    Ok(HistoryFixture {
        image,
        elsewhere: elsewhere.context("no branch was made")?,
        agent: started.agent_session,
    })
}

/// A two-finger gesture over the tree: sideways pans, up and down zooms.
async fn gesture(this: &This, cx: &mut AsyncWindowContext, dx: f32, dy: f32, steps: usize) {
    for step in 0..steps {
        this.update_in(cx, |ws, window, cx| {
            let center = ws.history.bounds.borrow().center();
            let event = ScrollWheelEvent {
                position: center,
                delta: ScrollDelta::Pixels(point(px(dx), px(dy))),
                modifiers: Modifiers::default(),
                touch_phase: if step == 0 {
                    TouchPhase::Started
                } else {
                    TouchPhase::Moved
                },
            };
            ws.history_scroll(&event, window, cx);
        })
        .ok();
        pause(cx, 16).await;
    }
    pause(cx, 350).await;
}

fn shown_revision(ws: &Workspace, image: AssetId) -> Option<RevisionId> {
    ws.image.edits.revisions.get(&image).copied()
}

pub(super) async fn history(
    this: &This,
    cx: &mut AsyncWindowContext,
    project: ProjectId,
    fixture: &HistoryFixture,
) -> Outcome {
    let image = fixture.image;
    let root = this
        .read_with(cx, |ws, _| ws.store.as_ref().ok().map(|s| s.root.clone()))
        .ok()
        .flatten()
        .unwrap_or_default();
    this.update_in(cx, |ws, window, cx| {
        ws.enter_project(project, window, cx);
        ws.open_item(Open::Image(image), window, cx);
    })
    .ok();
    until(this, cx, 30_000, Workspace::image_idle).await;
    crate::perf::drain();
    let mut samples = Vec::new();
    let mut problems: Vec<String> = Vec::new();

    let started = Instant::now();
    this.update_in(cx, |ws, window, cx| ws.set_mode(Mode::History, window, cx))
        .ok();
    match until(this, cx, 20_000, |ws| ws.history.shown.is_some()).await {
        Some(_) => samples.push(("settle", ms(started.elapsed()))),
        None => problems.push("the history never showed".into()),
    }
    let revisions = this
        .read_with(cx, |ws, _| {
            ws.history
                .shown
                .as_ref()
                .map_or(0, |s| s.layout.nodes.len())
        })
        .unwrap_or(0);
    if revisions < MAIN {
        problems.push(format!("the tree shows {revisions} revisions"));
    }

    // DEBUG (debug/mac-input only): focus the branch field, type slowly,
    // and hold, for screenshots on macOS.
    if std::env::var("SPECTRUM_INPUT_PROBE").is_ok() {
        // The runner's screen is 1024 by 768: fit the window on it.
        this.update_in(cx, |_, window, _| window.resize(size(px(960.), px(640.))))
            .ok();
        pause(cx, 2000).await;
        eprintln!("PROBE focus");
        this.update_in(cx, |ws, window, cx| {
            ws.history.name.update(cx, |input, cx| input.focus(window, cx));
        })
        .ok();
        pause(cx, 2500).await;
        for spec in ["end", "backspace", "backspace", "x", "y", "z"] {
            eprintln!("PROBE key {spec}");
            super::scenarios::key(cx, spec);
            pause(cx, 700).await;
        }
        eprintln!("PROBE hold");
        pause(cx, 2500).await;
        eprintln!("PROBE done");
    }

    // Pan and zoom like a canvas.
    let before = this.read_with(cx, |ws, _| ws.history.pan).ok();
    gesture(this, cx, -40., 0., 20).await;
    if this.read_with(cx, |ws, _| ws.history.pan).ok() == before {
        problems.push("a two-finger swipe did not pan the tree".into());
    }
    let zoom = this.read_with(cx, |ws, _| ws.history.zoom).unwrap_or(1.);
    gesture(this, cx, 0., -40., 12).await;
    gesture(this, cx, 0., 40., 24).await;
    if this.read_with(cx, |ws, _| ws.history.zoom).ok() == Some(zoom) {
        problems.push("a two-finger zoom did not zoom the tree".into());
    }

    // Zoomed in, revisions show previews.
    let started = Instant::now();
    this.update_in(cx, |ws, window, cx| {
        let current = ws.history.shown.as_ref().map(|s| s.history.current);
        ws.history.zoom = 3.;
        if let Some(current) = current {
            ws.center_on_revision(current);
        }
        ws.request_visible_previews(window, cx);
        cx.notify();
    })
    .ok();
    match until(this, cx, 20_000, |ws| !ws.history.previews.is_empty()).await {
        Some(_) => samples.push(("preview", ms(started.elapsed()))),
        None => problems.push("zoomed in, no preview showed".into()),
    }

    // Typing a name while the history reloads underneath (anyone saving
    // reloads it) keeps what is typed.
    this.update_in(cx, |ws, window, cx| {
        ws.history.name.update(cx, |input, cx| {
            input.focus(window, cx);
            input.set_value("Typed so f", window, cx);
        });
        ws.load_history(window, cx);
    })
    .ok();
    pause(cx, 300).await;
    let typed = this
        .read_with(cx, |ws, cx| ws.history.name.read(cx).value().to_string())
        .unwrap_or_default();
    if typed != "Typed so f" {
        problems.push(format!(
            "a reload replaced the name being typed with {typed:?}"
        ));
    }
    this.update_in(cx, |ws, window, cx| {
        ws.focus_handle.focus(window);
        ws.sync_history_name(window, cx);
    })
    .ok();

    // Name the person's branch, then find it by searching.
    let named = this
        .update_in(cx, |ws, window, cx| {
            ws.history.name.update(cx, |input, cx| {
                input.set_value("Benchmark pick", window, cx)
            });
            ws.name_current_branch(window, cx);
            ws.history.shown.as_ref().map(|s| s.history.current)
        })
        .ok()
        .flatten();
    let branch_named = |ws: &Workspace| {
        ws.history.shown.as_ref().is_some_and(|s| {
            let Some(lane) = s.layout.node(s.history.current).map(|n| n.lane) else {
                return false;
            };
            s.layout.branches.iter().any(|b| {
                s.layout.nodes[b.tip].lane == lane && b.name.as_deref() == Some("Benchmark pick")
            })
        })
    };
    if until(this, cx, 10_000, branch_named).await.is_none() {
        problems.push("naming the branch did not show its name".into());
    }
    // The branch finder (the palette, ⌘P) finds it by name.
    let found = this
        .update_in(cx, |ws, window, cx| {
            ws.open_branches(window, cx);
            let found = ws.branch_choices("benchmark pick");
            ws.close_palette(cx);
            found
        })
        .unwrap_or_default();
    if named.is_none() || found.len() != 1 {
        problems.push(format!(
            "finding the branch by name found {} branches",
            found.len()
        ));
    }

    // Go to a revision on another branch: the image shows it.
    let target = fixture.elsewhere;
    let started = Instant::now();
    this.update_in(cx, |ws, window, cx| ws.go_to_revision(target, window, cx))
        .ok();
    match until(this, cx, 20_000, |ws| {
        shown_revision(ws, image) == Some(target) && ws.image_idle()
    })
    .await
    {
        Some(_) => samples.push(("jump", ms(started.elapsed()))),
        None => problems.push("going to a revision did not show it".into()),
    }

    // Back returns to where the person was, Forward to the branch again.
    let before = named;
    this.update_in(cx, |ws, window, cx| ws.history_back(window, cx))
        .ok();
    if until(this, cx, 20_000, |ws| {
        shown_revision(ws, image) == before && ws.image_idle()
    })
    .await
    .is_none()
    {
        problems.push("Back did not return to the revision before".into());
    }
    this.update_in(cx, |ws, window, cx| ws.history_forward(window, cx))
        .ok();
    if until(this, cx, 20_000, |ws| {
        shown_revision(ws, image) == Some(target) && ws.image_idle()
    })
    .await
    .is_none()
    {
        problems.push("Forward did not return to the branch".into());
    }

    // Watch the agent: its next edit arrives without the person asking.
    let agent = fixture.agent;
    this.update_in(cx, |ws, window, cx| {
        ws.watch_session(agent, "Helper".into(), window, cx)
    })
    .ok();
    let joined = until(this, cx, 10_000, |ws| {
        ws.history
            .watching
            .as_ref()
            .is_some_and(|w| w.placed.is_some())
            && ws.image_idle()
    })
    .await;
    if joined.is_none() {
        problems.push("watching did not go to the agent's place".into());
    }
    let library = root.clone();
    let agent_edit = std::thread::spawn(move || -> Result<RevisionId> {
        let service = Service::agent(&library)?.with_session(Some(agent))?;
        service.set_adjustments(
            image,
            spectrum_image::Adjustments {
                vibrance: 33.,
                ..Default::default()
            },
        )?;
        Ok(service.history(image)?.current)
    });
    let started = Instant::now();
    let latest = loop {
        if agent_edit.is_finished() {
            break agent_edit.join().ok().and_then(Result::ok);
        }
        pause(cx, 5).await;
    };
    match latest {
        Some(latest) => {
            match until(this, cx, 10_000, |ws| {
                shown_revision(ws, image) == Some(latest) && ws.image_idle()
            })
            .await
            {
                Some(_) => samples.push(("watch", ms(started.elapsed()))),
                None => problems.push("the watched agent's edit never showed".into()),
            }
        }
        None => problems.push("the agent could not edit".into()),
    }
    this.update_in(cx, |ws, window, cx| {
        ws.stop_watching(cx);
        ws.set_mode(Mode::Color, window, cx);
    })
    .ok();
    pause(cx, 50).await;
    outcome(
        "history",
        samples,
        (!problems.is_empty()).then(|| problems.join("; ")),
    )
}
