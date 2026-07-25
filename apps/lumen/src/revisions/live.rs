use anyhow::{Context, Result};
use spectrum_revisions::{Actor, ProjectId, Revision, RevisionId, SessionId, TrackId};

use super::DurableCatalog;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveWorkspaceState {
    pub project_id: ProjectId,
    pub catalog_track_id: TrackId,
    pub catalog_cursor: RevisionId,
    pub photo_id: u64,
    pub photo_track_id: TrackId,
    pub session_id: SessionId,
    pub photo_cursor: RevisionId,
    pub actor: Actor,
    pub revision: Revision,
}

impl DurableCatalog {
    pub(crate) fn live_state_for_track(&self, track_id: TrackId) -> Result<LiveWorkspaceState> {
        let photo_id = self
            .photo_id_for_track(track_id)
            .with_context(|| format!("track {track_id} is not a Lumen photo track"))?;
        let photo_cursor = self.photo_cursor(photo_id)?;
        let revision = self
            .store
            .store()
            .revision(photo_cursor)?
            .context("current Lumen photo revision is missing")?;
        Ok(LiveWorkspaceState {
            project_id: self.info.project_id,
            catalog_track_id: self.info.default_track_id,
            catalog_cursor: self.catalog_cursor,
            photo_id,
            photo_track_id: track_id,
            session_id: self.session_id,
            photo_cursor,
            actor: self.actor.clone(),
            revision,
        })
    }

    pub(crate) fn catalog_identity(&self) -> (ProjectId, TrackId, RevisionId, SessionId) {
        (
            self.info.project_id,
            self.info.default_track_id,
            self.catalog_cursor,
            self.session_id,
        )
    }
}
