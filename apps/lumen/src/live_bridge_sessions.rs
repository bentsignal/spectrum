use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use spectrum_revisions::{
    ActorKind, Collaboration, CollaborationMode, CollaborationStatus, SessionId,
};

use crate::{
    LiveWorkspaceState, LumenLiveAction, LumenLiveApplied, LumenLiveResult, LumenLiveState,
    Workspace,
};

#[derive(Debug)]
pub enum LumenLiveApplyError {
    DefinitelyUnapplied(anyhow::Error),
    OutcomeUnknown(anyhow::Error),
}

impl LumenLiveApplyError {
    fn definitely_unapplied(error: impl Into<anyhow::Error>) -> Self {
        Self::DefinitelyUnapplied(error.into())
    }

    fn outcome_unknown(error: impl Into<anyhow::Error>) -> Self {
        Self::OutcomeUnknown(error.into())
    }
}

impl std::fmt::Display for LumenLiveApplyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DefinitelyUnapplied(error) | Self::OutcomeUnknown(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for LumenLiveApplyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::DefinitelyUnapplied(error) | Self::OutcomeUnknown(error) => error.source(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LumenLiveTestFault {
    AfterAgentMutation,
    AfterHumanSync,
}

pub struct LumenLiveSessions {
    project_path: PathBuf,
    #[cfg(test)]
    test_fault: Option<LumenLiveTestFault>,
}

impl LumenLiveSessions {
    pub fn new(project_path: impl Into<PathBuf>) -> Self {
        Self {
            project_path: project_path.into(),
            #[cfg(test)]
            test_fault: None,
        }
    }

    pub fn project_path(&self) -> &Path {
        &self.project_path
    }

    pub fn apply(
        &mut self,
        human: &mut Workspace,
        agent_session: SessionId,
        action: LumenLiveAction,
    ) -> std::result::Result<LumenLiveResult, LumenLiveApplyError> {
        action
            .validate()
            .map_err(LumenLiveApplyError::definitely_unapplied)?;
        self.require_same_project_path(human)
            .map_err(LumenLiveApplyError::definitely_unapplied)?;
        let collaboration = Workspace::collaboration(&self.project_path, agent_session)
            .map_err(LumenLiveApplyError::definitely_unapplied)?;
        let human_before = durable_state(human, collaboration.track_id, "bound human workspace")
            .map_err(LumenLiveApplyError::definitely_unapplied)?;
        authorize(&human_before, &collaboration, agent_session)
            .map_err(LumenLiveApplyError::definitely_unapplied)?;
        let mut agent = Workspace::open_session(&self.project_path, agent_session)
            .map_err(LumenLiveApplyError::definitely_unapplied)?;
        let agent_before = durable_state(&agent, collaboration.track_id, "agent workspace")
            .map_err(LumenLiveApplyError::definitely_unapplied)?;
        authorize_agent_state(&human_before, &agent_before, &collaboration)
            .map_err(LumenLiveApplyError::definitely_unapplied)?;
        let before = LumenLiveState::new(&human_before, &agent_before, collaboration.clone());

        if matches!(action, LumenLiveAction::State) {
            return Ok(LumenLiveResult::State(Box::new(before)));
        }
        let expectation = action
            .expectation()
            .context("mutating live action lost its expectation")
            .map_err(LumenLiveApplyError::definitely_unapplied)?;
        check_expectation(expectation, &human_before, &agent_before, &collaboration)
            .map_err(LumenLiveApplyError::definitely_unapplied)?;

        let mutation = match action {
            LumenLiveAction::State => unreachable!(),
            LumenLiveAction::ExecuteBatch { commands, .. } => agent.execute_batch(commands),
            LumenLiveAction::Undo { .. } => {
                agent.project.selected = Some(agent_before.photo_id);
                agent
                    .execute(crate::Command::Undo)
                    .map(|output| vec![output])
            }
            LumenLiveAction::Redo { .. } => {
                agent.project.selected = Some(agent_before.photo_id);
                agent
                    .execute(crate::Command::Redo)
                    .map(|output| vec![output])
            }
            LumenLiveAction::MoveAgentCursor { target, .. } => agent
                .move_photo_to_revision(agent_before.photo_id, target)
                .map(|_| Vec::new()),
        };
        let outputs = match mutation {
            Ok(outputs) => outputs,
            Err(error) => {
                let recovery = Workspace::open_session(&self.project_path, agent_session).and_then(
                    |workspace| {
                        durable_state(&workspace, collaboration.track_id, "recovered agent")
                    },
                );
                return Err(match recovery {
                    Ok(recovered) if recovered.photo_cursor == agent_before.photo_cursor => {
                        LumenLiveApplyError::definitely_unapplied(error)
                    }
                    Ok(recovered) => LumenLiveApplyError::outcome_unknown(error.context(format!(
                        "agent cursor advanced from {} to {}",
                        agent_before.photo_cursor, recovered.photo_cursor
                    ))),
                    Err(recovery_error) => {
                        LumenLiveApplyError::outcome_unknown(error.context(format!(
                            "could not recover the agent session to classify the outcome: {recovery_error:#}"
                        )))
                    }
                });
            }
        };
        self.maybe_fail(LumenLiveTestFault::AfterAgentMutation)?;

        let collaboration_sync = if collaboration.mode == CollaborationMode::Together {
            let sync = human
                .sync_together()
                .map_err(LumenLiveApplyError::outcome_unknown)?;
            self.maybe_fail(LumenLiveTestFault::AfterHumanSync)?;
            Some(sync.into())
        } else {
            Some(crate::LumenLiveCollaborationSync::Split)
        };
        let collaboration = Workspace::collaboration(&self.project_path, agent_session)
            .map_err(LumenLiveApplyError::outcome_unknown)?;
        let human_after = durable_state(human, collaboration.track_id, "bound human workspace")
            .map_err(LumenLiveApplyError::outcome_unknown)?;
        let agent_after = durable_state(&agent, collaboration.track_id, "agent workspace")
            .map_err(LumenLiveApplyError::outcome_unknown)?;
        let committed_revision = (agent_after.photo_cursor != agent_before.photo_cursor
            && agent_after.revision.parent_id == Some(agent_before.photo_cursor))
        .then(|| agent_after.revision.clone());
        let after = LumenLiveState::new(&human_after, &agent_after, collaboration);
        Ok(LumenLiveResult::Applied(Box::new(LumenLiveApplied {
            before,
            after,
            outputs,
            committed_revision,
            collaboration_sync,
        })))
    }

    #[cfg(test)]
    pub(crate) fn inject_fault(&mut self, fault: LumenLiveTestFault) {
        self.test_fault = Some(fault);
    }

    #[cfg(test)]
    fn maybe_fail(
        &mut self,
        phase: LumenLiveTestFault,
    ) -> std::result::Result<(), LumenLiveApplyError> {
        if self.test_fault == Some(phase) {
            self.test_fault = None;
            return Err(LumenLiveApplyError::outcome_unknown(anyhow::anyhow!(
                "injected Lumen live failure at {phase:?}"
            )));
        }
        Ok(())
    }

    #[cfg(not(test))]
    fn maybe_fail(
        &mut self,
        _phase: LumenLiveTestFault,
    ) -> std::result::Result<(), LumenLiveApplyError> {
        Ok(())
    }

    fn require_same_project_path(&self, human: &Workspace) -> Result<()> {
        let human_path = human
            .catalog_path
            .as_deref()
            .context("bound human workspace has no durable catalog path")?;
        let expected = std::fs::canonicalize(&self.project_path)?;
        let actual = std::fs::canonicalize(human_path)?;
        if actual != expected {
            bail!("live binding project path no longer matches the human workspace");
        }
        Ok(())
    }
}

fn durable_state(
    workspace: &Workspace,
    track_id: spectrum_revisions::TrackId,
    label: &str,
) -> Result<LiveWorkspaceState> {
    workspace
        .live_state_for_track(track_id)?
        .with_context(|| format!("{label} is not a durable Lumen project"))
}

fn authorize(
    human: &LiveWorkspaceState,
    collaboration: &Collaboration,
    requested_session: SessionId,
) -> Result<()> {
    if human.actor.kind != ActorKind::Human {
        bail!("live binding is not owned by a human session");
    }
    if collaboration.agent_session != requested_session
        || collaboration.source_session != human.session_id
        || collaboration.track_id != human.photo_track_id
    {
        bail!("agent collaboration does not belong to this bound human project session");
    }
    if collaboration.status == CollaborationStatus::Superseded {
        bail!("agent collaboration has been superseded");
    }
    Ok(())
}

fn authorize_agent_state(
    human: &LiveWorkspaceState,
    agent: &LiveWorkspaceState,
    collaboration: &Collaboration,
) -> Result<()> {
    if agent.actor.kind != ActorKind::Agent
        || agent.session_id != collaboration.agent_session
        || agent.project_id != human.project_id
        || agent.photo_track_id != human.photo_track_id
    {
        bail!("agent session identity does not match the bound project");
    }
    Ok(())
}

fn check_expectation(
    expectation: &crate::LumenLiveActionExpectation,
    human: &LiveWorkspaceState,
    agent: &LiveWorkspaceState,
    collaboration: &Collaboration,
) -> Result<()> {
    if expectation.photo_id != human.photo_id
        || expectation.track_id != collaboration.track_id
        || expectation.track_id != human.photo_track_id
    {
        bail!("live expectation does not identify the collaboration photo track");
    }
    if expectation.agent_revision != agent.photo_cursor {
        bail!(
            "stale agent cursor: expected {}, current {}",
            expectation.agent_revision,
            agent.photo_cursor
        );
    }
    match collaboration.mode {
        CollaborationMode::Together => {
            let source = expectation
                .source_revision
                .context("Together actions require an exact source cursor")?;
            if source != human.photo_cursor || source != collaboration.followed_revision {
                bail!(
                    "stale Together source cursor: expected {source}, current {}",
                    human.photo_cursor
                );
            }
            if collaboration.status != CollaborationStatus::Active {
                bail!("Together collaboration is no longer active");
            }
        }
        CollaborationMode::Separate => {
            if expectation.source_revision.is_some() {
                bail!("Separate actions must not claim a source cursor");
            }
        }
    }
    Ok(())
}
