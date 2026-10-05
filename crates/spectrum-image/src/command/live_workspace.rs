use anyhow::Result;
use spectrum_revisions::{ProjectId, RevisionId, SessionId, TrackId};

use super::Workspace;

impl Workspace {
    pub fn live_state_for_track(
        &self,
        track_id: TrackId,
    ) -> Result<Option<crate::LiveWorkspaceState>> {
        self.durable
            .as_ref()
            .map(|durable| durable.live_state_for_track(track_id))
            .transpose()
    }

    pub fn live_catalog_identity(&self) -> Option<(ProjectId, TrackId, RevisionId, SessionId)> {
        self.durable
            .as_ref()
            .map(crate::DurableCatalog::catalog_identity)
    }
}
