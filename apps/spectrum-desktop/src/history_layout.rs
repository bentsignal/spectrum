//! Where each revision of a history sits in the History view: one column per
//! step from the first revision, and one row (lane) per branch. A revision's
//! first change continues its row; later changes start rows below, after all
//! of the first one's own branches, so no two lines cross.
use spectrum_document::{History, RevisionId};
use std::collections::HashMap;

pub struct Node {
    pub id: RevisionId,
    pub parent: Option<usize>,
    /// Steps from the first revision: the column.
    pub depth: u32,
    /// The row.
    pub lane: u32,
    /// What the edit did, as the engine describes it.
    pub label: Option<String>,
    pub actor: String,
    pub agent: bool,
    pub at_ms: i64,
    pub name: Option<String>,
}

/// A branch: a revision with no later ones, and what it is called.
pub struct Branch {
    pub tip: usize,
    /// Its name: the newest name on its own part (since it forked), if any.
    pub name: Option<String>,
}

pub struct Layout {
    pub nodes: Vec<Node>,
    pub index: HashMap<RevisionId, usize>,
    pub branches: Vec<Branch>,
    pub lanes: u32,
    pub depth: u32,
}

impl Layout {
    pub fn node(&self, id: RevisionId) -> Option<&Node> {
        self.index.get(&id).map(|&at| &self.nodes[at])
    }

    /// What a branch is called: its name, or who made its newest revision.
    pub fn branch_title(&self, branch: &Branch) -> String {
        branch.name.clone().unwrap_or_else(|| {
            let tip = &self.nodes[branch.tip];
            if tip.agent {
                format!("{}'s branch", tip.actor)
            } else {
                "Your branch".into()
            }
        })
    }
}

pub fn layout(history: &History) -> Layout {
    let names: HashMap<RevisionId, &str> = history
        .names
        .iter()
        .map(|(id, name)| (*id, name.as_str()))
        .collect();
    let mut index = HashMap::with_capacity(history.revisions.len());
    for (at, revision) in history.revisions.iter().enumerate() {
        index.insert(revision.id, at);
    }
    // Each revision's later ones, oldest first (revisions come in time order).
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); history.revisions.len()];
    for (at, revision) in history.revisions.iter().enumerate() {
        if let Some(parent) = revision.parent_id.and_then(|p| index.get(&p)) {
            children[*parent].push(at);
        }
    }
    let mut nodes: Vec<Node> = history
        .revisions
        .iter()
        .map(|revision| Node {
            id: revision.id,
            parent: revision.parent_id.and_then(|p| index.get(&p).copied()),
            depth: 0,
            lane: 0,
            label: revision.label.clone(),
            actor: revision.actor.display_name.clone(),
            agent: !matches!(revision.actor.kind, spectrum_document::ActorKind::Human),
            at_ms: revision.created_at_ms,
            name: names.get(&revision.id).map(|name| name.to_string()),
        })
        .collect();
    // Depth first, without recursion: a history can be thousands deep.
    let mut lanes = 0;
    let mut depth = 0;
    let roots: Vec<usize> = (0..nodes.len())
        .filter(|&at| nodes[at].parent.is_none())
        .collect();
    let mut stack: Vec<(usize, u32, u32)> = Vec::new();
    for root in roots.into_iter().rev() {
        stack.push((root, 0, u32::MAX));
    }
    while let Some((at, step, lane)) = stack.pop() {
        let lane = if lane == u32::MAX {
            lanes += 1;
            lanes - 1
        } else {
            lane
        };
        nodes[at].depth = step;
        nodes[at].lane = lane;
        depth = depth.max(step);
        // The first change continues this row; the rest take new rows when
        // they come up, after everything the first one leads to.
        for (order, &child) in children[at].iter().enumerate().rev() {
            stack.push((child, step + 1, if order == 0 { lane } else { u32::MAX }));
        }
    }
    let branches = (0..nodes.len())
        .filter(|&at| children[at].is_empty())
        .map(|tip| {
            // Back to where it forked, taking the newest name on the way.
            let mut at = tip;
            let mut name = None;
            loop {
                if name.is_none() {
                    name = nodes[at].name.clone();
                }
                match nodes[at].parent {
                    Some(parent) if children[parent].len() == 1 => at = parent,
                    Some(parent) if children[parent].first() == Some(&at) => at = parent,
                    _ => break,
                }
            }
            Branch { tip, name }
        })
        .collect();
    Layout {
        nodes,
        index,
        branches,
        lanes,
        depth,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectrum_document::{Actor, ActorKind, Revision, SessionId};

    fn revision(id: RevisionId, parent: Option<RevisionId>, at: i64, who: &str) -> Revision {
        Revision {
            id,
            track_id: Default::default(),
            change_set_id: Default::default(),
            parent_id: parent,
            actor: Actor {
                id: who.into(),
                display_name: who.into(),
                kind: if who == "You" {
                    ActorKind::Human
                } else {
                    ActorKind::Agent
                },
            },
            session_id: SessionId::new(),
            created_at_ms: at,
            application_version: "test".into(),
            label: Some(format!("edit {at}")),
            command_count: 1,
        }
    }

    /// root - a - b - c
    ///          \- d - e      (an agent's branch from a)
    ///          \- f          (another from a)
    #[test]
    fn rows_never_cross_and_branches_take_their_names() {
        let ids: Vec<RevisionId> = (0..7).map(|_| RevisionId::new()).collect();
        let [root, a, b, c, d, e, f] = ids[..] else {
            unreachable!()
        };
        let history = History {
            root,
            current: c,
            revisions: vec![
                revision(root, None, 0, "You"),
                revision(a, Some(root), 1, "You"),
                revision(b, Some(a), 2, "You"),
                revision(d, Some(a), 3, "Helper"),
                revision(c, Some(b), 4, "You"),
                revision(e, Some(d), 5, "Helper"),
                revision(f, Some(a), 6, "You"),
            ],
            sessions: Vec::new(),
            names: vec![(d, "Moody".into()), (a, "Base".into())],
        };
        let layout = layout(&history);
        let place = |id| {
            let node = layout.node(id).unwrap();
            (node.depth, node.lane)
        };
        assert_eq!(place(root), (0, 0));
        assert_eq!(place(a), (1, 0));
        assert_eq!(place(b), (2, 0));
        assert_eq!(place(c), (3, 0));
        assert_eq!(place(d), (2, 1));
        assert_eq!(place(e), (3, 1));
        assert_eq!(place(f), (2, 2));
        assert_eq!((layout.lanes, layout.depth), (3, 3));
        let title = |tip| {
            let branch = layout
                .branches
                .iter()
                .find(|b| layout.nodes[b.tip].id == tip)
                .unwrap();
            layout.branch_title(branch)
        };
        // The first branch runs back to the start and takes "Base".
        assert_eq!(title(c), "Base");
        assert_eq!(title(e), "Moody");
        // A branch with no name of its own is called after who made it.
        assert_eq!(title(f), "Your branch");
    }

    #[test]
    fn a_long_history_lays_out_without_recursion() {
        let mut revisions = Vec::new();
        let mut parent = None;
        for at in 0..50_000 {
            let id = RevisionId::new();
            revisions.push(revision(id, parent, at, "You"));
            parent = Some(id);
        }
        let history = History {
            root: revisions[0].id,
            current: parent.unwrap(),
            revisions,
            sessions: Vec::new(),
            names: Vec::new(),
        };
        let layout = layout(&history);
        assert_eq!((layout.lanes, layout.depth), (1, 49_999));
        assert_eq!(layout.branches.len(), 1);
    }
}
