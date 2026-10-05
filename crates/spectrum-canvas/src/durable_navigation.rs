use super::*;

pub(crate) struct PreparedNavigation {
    target: RevisionId,
    document: Document,
    snapshot_tail: SnapshotTail,
    remember_child: Option<(RevisionId, RevisionId)>,
}

impl PreparedNavigation {
    pub(crate) fn changes_cursor(&self, current: RevisionId) -> bool {
        self.target != current
    }
}

impl DurableProject {
    pub(crate) fn prepare_move_to(&self, target: RevisionId) -> Result<PreparedNavigation> {
        let target = self
            .store
            .store()
            .newest_compatible_ancestor(target, &PrismCompatibility)?;
        let (document, snapshot_tail) = self.load(target)?;
        Ok(PreparedNavigation {
            target,
            document,
            snapshot_tail,
            remember_child: None,
        })
    }

    pub(crate) fn prepare_undo(&self) -> Result<PreparedNavigation> {
        let current = self
            .store
            .store()
            .revision(self.cursor)?
            .context("current Prism revision is missing")?;
        let parent = current.parent_id.context("nothing to undo")?;
        let mut prepared = self.prepare_move_to(parent)?;
        prepared.remember_child = Some((parent, self.cursor));
        Ok(prepared)
    }

    pub(crate) fn prepare_redo(&self) -> Result<PreparedNavigation> {
        let preferred = self
            .store
            .store()
            .preferred_child(self.session_id, self.cursor)?;
        let target = match preferred {
            Some(preferred) => preferred,
            None => {
                let children = self.store.store().children(self.cursor)?;
                match children.as_slice() {
                    [only] => only.id,
                    [] => bail!("nothing to redo"),
                    _ => bail!("choose which future to follow"),
                }
            }
        };
        self.prepare_move_to(target)
    }

    pub(crate) fn commit_navigation(&mut self, prepared: PreparedNavigation) -> Result<Document> {
        if prepared.target != self.cursor {
            if let Some((parent, child)) = prepared.remember_child {
                self.store
                    .mutate(|store| store.remember_child(self.session_id, parent, child))?;
            }
            self.store.mutate(|store| {
                store.move_session(self.session_id, self.cursor, prepared.target)
            })?;
            self.cursor = prepared.target;
        }
        self.snapshot_tail = prepared.snapshot_tail;
        Ok(prepared.document)
    }
}
