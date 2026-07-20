//! Shared revision-history presentation for Spectrum creative applications.
//!
//! Applications provide their own previews and navigation behavior. This crate owns the stable
//! graph layout, node presentation, legends, revision metadata, and the macOS shortcut reservation
//! so Prism, Lumen, Bloom, and future tools present one coherent history surface.

use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use spectrum_revisions::{ActorKind, Revision, RevisionId, Session};

const NODE_SIZE: Vec2 = Vec2::new(224.0, 66.0);
const LANE_GAP: f32 = 264.0;
const DEPTH_GAP: f32 = 104.0;
const TREE_MARGIN: f32 = 48.0;

#[derive(Clone, Copy, Debug)]
pub struct HistoryTheme {
    pub ink: Color32,
    pub panel: Color32,
    pub surface: Color32,
    pub hover_surface: Color32,
    pub active_surface: Color32,
    pub focus_surface: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub human: Color32,
    pub agent: Color32,
    pub system: Color32,
}

impl Default for HistoryTheme {
    fn default() -> Self {
        Self {
            ink: Color32::from_rgb(14, 16, 20),
            panel: Color32::from_rgb(25, 28, 34),
            surface: Color32::from_rgb(34, 38, 46),
            hover_surface: Color32::from_rgb(57, 63, 74),
            active_surface: Color32::from_rgb(39, 91, 85),
            focus_surface: Color32::from_rgb(31, 56, 57),
            border: Color32::from_rgb(62, 68, 80),
            text: Color32::from_rgb(226, 230, 238),
            muted: Color32::from_rgb(145, 153, 169),
            accent: Color32::from_rgb(93, 216, 199),
            human: Color32::from_rgb(106, 162, 255),
            agent: Color32::from_rgb(184, 126, 255),
            system: Color32::from_rgb(235, 182, 92),
        }
    }
}

#[derive(Clone, Copy)]
pub struct HistoryGraph<'a> {
    pub root: RevisionId,
    pub current: RevisionId,
    pub revisions: &'a [Revision],
    pub sessions: &'a [Session],
}

pub fn history_header(ui: &mut Ui, subtitle: &str, theme: HistoryTheme) -> bool {
    let mut refresh = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                RichText::new("REVISION TREE")
                    .size(12.0)
                    .strong()
                    .color(theme.accent),
            );
            ui.label(RichText::new(subtitle).size(11.0).color(theme.muted));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            refresh = ui.button("Refresh").clicked();
            current_legend(ui, theme);
            legend(ui, "Agent", theme.agent, theme);
            legend(ui, "Human", theme.human, theme);
        });
    });
    refresh
}

pub fn history_tree(
    ui: &mut Ui,
    graph: HistoryGraph<'_>,
    selected: Option<RevisionId>,
    scroll_to_current: &mut bool,
    id_salt: impl Hash + std::fmt::Debug,
    theme: HistoryTheme,
) -> Option<RevisionId> {
    let positions = tree_layout(graph);
    let max_depth = positions
        .values()
        .map(|position| position.depth)
        .max()
        .unwrap_or(0);
    let max_lane = positions
        .values()
        .map(|position| position.lane)
        .max()
        .unwrap_or(0);
    let desired = Vec2::new(
        TREE_MARGIN * 2.0 + NODE_SIZE.x + max_lane as f32 * LANE_GAP,
        TREE_MARGIN * 2.0 + NODE_SIZE.y + max_depth as f32 * DEPTH_GAP,
    );
    let mut clicked = None;
    egui::ScrollArea::both()
        .id_salt(id_salt)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (_, canvas) = ui.allocate_space(desired);
            let node_rect = |id: RevisionId| {
                let position = positions[&id];
                Rect::from_min_size(
                    canvas.min
                        + Vec2::new(
                            TREE_MARGIN + position.lane as f32 * LANE_GAP,
                            TREE_MARGIN + position.depth as f32 * DEPTH_GAP,
                        ),
                    NODE_SIZE,
                )
            };
            paint_edges(ui, graph, &positions, &node_rect, theme);
            for revision in graph.revisions {
                let Some(position) = positions.get(&revision.id) else {
                    continue;
                };
                let rect = node_rect(revision.id);
                let response = ui.interact(
                    rect,
                    ui.id().with(("revision-node", revision.id.to_string())),
                    Sense::click(),
                );
                let is_current = revision.id == graph.current;
                paint_revision_node(
                    ui,
                    rect,
                    revision,
                    position.active,
                    is_current,
                    selected == Some(revision.id),
                    graph
                        .sessions
                        .iter()
                        .filter(|session| session.cursor == revision.id)
                        .count(),
                    response.hovered(),
                    theme,
                );
                if is_current && *scroll_to_current {
                    ui.scroll_to_rect(rect, Some(egui::Align::Center));
                    *scroll_to_current = false;
                }
                if response.clicked() {
                    clicked = Some(revision.id);
                }
            }
        });
    clicked
}

pub fn revision_details(
    ui: &mut Ui,
    graph: HistoryGraph<'_>,
    selected: Option<RevisionId>,
    theme: HistoryTheme,
) {
    let selected = selected.unwrap_or(graph.current);
    let Some((index, revision)) = graph
        .revisions
        .iter()
        .enumerate()
        .find(|(_, revision)| revision.id == selected)
    else {
        return;
    };
    ui.label(
        RichText::new(revision.label.as_deref().unwrap_or("Unlabeled revision"))
            .size(16.0)
            .strong()
            .color(theme.text),
    );
    ui.add_space(5.0);
    ui.label(
        RichText::new(format!(
            "Revision {} of {}  ·  {}",
            index + 1,
            graph.revisions.len(),
            short_id(revision.id)
        ))
        .monospace()
        .size(10.0)
        .color(theme.muted),
    );
    ui.add_space(14.0);
    detail_row(ui, "Created by", &revision.actor.display_name, theme);
    detail_row(ui, "Kind", actor_kind_label(revision.actor.kind), theme);
    detail_row(ui, "Commands", &revision.command_count.to_string(), theme);
    let children = graph
        .revisions
        .iter()
        .filter(|candidate| candidate.parent_id == Some(revision.id))
        .count();
    detail_row(ui, "Futures", &children.to_string(), theme);
    let sessions: Vec<_> = graph
        .sessions
        .iter()
        .filter(|session| session.cursor == revision.id)
        .collect();
    if !sessions.is_empty() {
        ui.add_space(14.0);
        ui.label(
            RichText::new("SESSIONS HERE")
                .size(10.0)
                .strong()
                .color(theme.muted),
        );
        for session in sessions.iter().take(5) {
            ui.label(
                RichText::new(format!(
                    "{} · {}",
                    session.actor.display_name,
                    actor_kind_label(session.actor.kind)
                ))
                .size(11.0)
                .color(actor_color(session.actor.kind, theme)),
            );
        }
        if sessions.len() > 5 {
            ui.label(
                RichText::new(format!("+{} more", sessions.len() - 5))
                    .size(10.0)
                    .color(theme.muted),
            );
        }
    }
    ui.add_space(18.0);
    ui.label(
        RichText::new(
            "Clicking a node moves your session there. Your next edit from an older node creates a new branch; existing futures stay intact.",
        )
        .size(11.0)
        .color(theme.muted),
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NodePosition {
    depth: usize,
    lane: usize,
    active: bool,
}

fn tree_layout(graph: HistoryGraph<'_>) -> HashMap<RevisionId, NodePosition> {
    let revisions: HashMap<_, _> = graph
        .revisions
        .iter()
        .map(|revision| (revision.id, revision))
        .collect();
    let mut active = HashSet::new();
    let mut cursor = Some(graph.current);
    while let Some(id) = cursor {
        active.insert(id);
        cursor = revisions.get(&id).and_then(|revision| revision.parent_id);
    }
    let mut children: HashMap<RevisionId, Vec<RevisionId>> = HashMap::new();
    for revision in graph.revisions {
        if let Some(parent) = revision.parent_id {
            children.entry(parent).or_default().push(revision.id);
        }
    }

    let mut positions = HashMap::new();
    let mut stack = vec![(graph.root, 0, 0)];
    let mut next_lane = 1;
    while let Some((id, depth, lane)) = stack.pop() {
        positions.insert(
            id,
            NodePosition {
                depth,
                lane,
                active: active.contains(&id),
            },
        );
        let Some(descendants) = children.get(&id) else {
            continue;
        };
        let queued = descendants
            .iter()
            .copied()
            .enumerate()
            .map(|(index, child)| {
                let child_lane = if index == 0 {
                    lane
                } else {
                    let child_lane = next_lane;
                    next_lane += 1;
                    child_lane
                };
                (child, depth + 1, child_lane)
            })
            .collect::<Vec<_>>();
        stack.extend(queued.into_iter().rev());
    }
    positions
}

fn paint_edges(
    ui: &Ui,
    graph: HistoryGraph<'_>,
    positions: &HashMap<RevisionId, NodePosition>,
    node_rect: &impl Fn(RevisionId) -> Rect,
    theme: HistoryTheme,
) {
    for revision in graph.revisions {
        let Some(parent) = revision.parent_id else {
            continue;
        };
        let (Some(parent_position), Some(child_position)) =
            (positions.get(&parent), positions.get(&revision.id))
        else {
            continue;
        };
        let start = node_rect(parent).center_bottom();
        let end = node_rect(revision.id).center_top();
        let middle_y = (start.y + end.y) * 0.5;
        let color = if parent_position.active && child_position.active {
            theme.accent
        } else {
            theme.border
        };
        let stroke = Stroke::new(if color == theme.accent { 2.0 } else { 1.25 }, color);
        ui.painter()
            .line_segment([start, Pos2::new(start.x, middle_y)], stroke);
        ui.painter().line_segment(
            [Pos2::new(start.x, middle_y), Pos2::new(end.x, middle_y)],
            stroke,
        );
        ui.painter()
            .line_segment([Pos2::new(end.x, middle_y), end], stroke);
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_revision_node(
    ui: &Ui,
    rect: Rect,
    revision: &Revision,
    active: bool,
    current: bool,
    selected: bool,
    sessions: usize,
    hovered: bool,
    theme: HistoryTheme,
) {
    let actor = actor_color(revision.actor.kind, theme);
    let fill = if current {
        theme.active_surface
    } else if selected || hovered {
        theme.hover_surface
    } else if active {
        theme.focus_surface
    } else {
        theme.surface
    };
    let stroke = if current {
        Stroke::new(2.5, theme.accent)
    } else if selected {
        Stroke::new(2.0, theme.text)
    } else {
        Stroke::new(1.0, if active { actor } else { theme.border })
    };
    ui.painter().rect_filled(rect, 9.0, fill);
    ui.painter()
        .rect_stroke(rect, 9.0, stroke, egui::StrokeKind::Inside);
    ui.painter().text(
        rect.left_top() + Vec2::new(14.0, 12.0),
        Align2::LEFT_TOP,
        truncate(
            revision.label.as_deref().unwrap_or("Unlabeled revision"),
            30,
        ),
        FontId::proportional(12.0),
        theme.text,
    );
    ui.painter().text(
        rect.left_bottom() + Vec2::new(24.0, -12.0),
        Align2::LEFT_BOTTOM,
        truncate(&revision.actor.display_name, 22),
        FontId::proportional(10.0),
        actor,
    );
    ui.painter()
        .circle_filled(rect.left_bottom() + Vec2::new(14.0, -13.0), 3.5, actor);
    if current {
        let badge = Rect::from_min_size(
            rect.right_bottom() + Vec2::new(-78.0, -23.0),
            Vec2::new(68.0, 17.0),
        );
        ui.painter().rect_filled(badge, 5.0, theme.accent);
        ui.painter().text(
            badge.center(),
            Align2::CENTER_CENTER,
            "YOU ARE HERE",
            FontId::monospace(8.0),
            theme.ink,
        );
    } else if sessions > 0 {
        ui.painter().text(
            rect.right_bottom() + Vec2::new(-10.0, -10.0),
            Align2::RIGHT_BOTTOM,
            format!("{sessions} SESSION{}", if sessions == 1 { "" } else { "S" }),
            FontId::monospace(8.0),
            actor,
        );
    }
}

fn legend(ui: &mut Ui, label: &str, color: Color32, theme: HistoryTheme) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.0, color);
        ui.label(RichText::new(label).size(10.0).color(theme.muted));
    });
}

fn current_legend(ui: &mut Ui, theme: HistoryTheme) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
        ui.painter()
            .circle_stroke(rect.center(), 4.5, Stroke::new(2.0, theme.accent));
        ui.painter().circle_filled(rect.center(), 1.5, theme.accent);
        ui.label(RichText::new("Current").size(10.0).color(theme.muted));
    });
}

fn detail_row(ui: &mut Ui, label: &str, value: &str, theme: HistoryTheme) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(10.0).color(theme.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(value).size(11.0).color(theme.text));
        });
    });
}

fn actor_color(kind: ActorKind, theme: HistoryTheme) -> Color32 {
    match kind {
        ActorKind::Human => theme.human,
        ActorKind::Agent => theme.agent,
        ActorKind::System => theme.system,
    }
}

fn actor_kind_label(kind: ActorKind) -> &'static str {
    match kind {
        ActorKind::Human => "Human",
        ActorKind::Agent => "Agent",
        ActorKind::System => "System",
    }
}

fn short_id(id: RevisionId) -> String {
    id.to_string().chars().take(8).collect()
}

fn truncate(value: &str, limit: usize) -> String {
    let mut characters = value.chars();
    let prefix: String = characters.by_ref().take(limit).collect();
    if characters.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[cfg(target_os = "macos")]
pub fn reserve_history_shortcut() {
    use std::sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    };

    use objc2::sel;
    use objc2_app_kit::{NSApplication, NSMenu};
    use objc2_foundation::{MainThreadMarker, NSString};

    static RESERVED: OnceLock<AtomicBool> = OnceLock::new();

    fn clear_hide_shortcut(menu: &NSMenu) -> bool {
        let mut cleared = false;
        for item in &menu.itemArray() {
            if item.action() == Some(sel!(hide:)) {
                item.setKeyEquivalent(&NSString::from_str(""));
                cleared = true;
            }
            if let Some(submenu) = item.submenu() {
                cleared |= clear_hide_shortcut(&submenu);
            }
        }
        cleared
    }

    let reserved = RESERVED.get_or_init(|| AtomicBool::new(false));
    if reserved.load(Ordering::Relaxed) {
        return;
    }
    let marker = MainThreadMarker::new().expect("Spectrum apps run on the macOS main thread");
    let application = NSApplication::sharedApplication(marker);
    let Some(menu) = application.mainMenu() else {
        return;
    };
    if clear_hide_shortcut(&menu) {
        reserved.store(true, Ordering::Relaxed);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn reserve_history_shortcut() {}

#[cfg(test)]
mod tests {
    use super::*;
    use spectrum_revisions::{Actor, ChangeSetId, SessionId, TrackId};

    fn revision(id: u8, parent: Option<u8>) -> Revision {
        Revision {
            id: RevisionId::from_bytes([id; 16]),
            track_id: TrackId::from_bytes([7; 16]),
            change_set_id: ChangeSetId::from_bytes([id; 16]),
            parent_id: parent.map(|parent| RevisionId::from_bytes([parent; 16])),
            actor: Actor {
                id: "person:test".into(),
                display_name: "Person".into(),
                kind: ActorKind::Human,
            },
            session_id: SessionId::from_bytes([9; 16]),
            created_at_ms: i64::from(id),
            application_version: "1".into(),
            label: Some(format!("Revision {id}")),
            command_count: 1,
        }
    }

    fn positions(revisions: &[Revision], current: u8) -> HashMap<RevisionId, NodePosition> {
        tree_layout(HistoryGraph {
            root: revisions[0].id,
            current: RevisionId::from_bytes([current; 16]),
            revisions,
            sessions: &[],
        })
    }

    #[test]
    fn switching_branches_preserves_every_nodes_geometry() {
        let revisions = vec![
            revision(1, None),
            revision(2, Some(1)),
            revision(3, Some(2)),
            revision(4, Some(2)),
            revision(5, Some(4)),
        ];
        let left = positions(&revisions, 3);
        let right = positions(&revisions, 5);
        for id in [1, 2, 3, 4, 5] {
            let id = RevisionId::from_bytes([id; 16]);
            assert_eq!(left[&id].depth, right[&id].depth);
            assert_eq!(left[&id].lane, right[&id].lane);
        }
        assert!(left[&RevisionId::from_bytes([3; 16])].active);
        assert!(!right[&RevisionId::from_bytes([3; 16])].active);
        assert!(right[&RevisionId::from_bytes([5; 16])].active);
    }

    #[test]
    fn long_labels_are_truncated_on_character_boundaries() {
        assert_eq!(truncate("abcdefgh", 5), "abcde…");
        assert_eq!(truncate("éclair", 2), "éc…");
        assert_eq!(truncate("short", 8), "short");
    }

    #[test]
    fn current_position_and_human_actor_have_distinct_colors() {
        let theme = HistoryTheme::default();
        assert_ne!(theme.human, theme.accent);
    }
}
