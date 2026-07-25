use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use spectrum_revisions::{
    ActorKind, Collaboration, CollaborationMode, CollaborationStatus, SessionId,
};

use crate::{
    LiveWorkspaceState, PrismLiveAction, PrismLiveApplied, PrismLiveResult, PrismLiveState,
    Workspace,
};

#[derive(Debug)]
pub enum PrismLiveApplyError {
    DefinitelyUnapplied(anyhow::Error),
    OutcomeUnknown(anyhow::Error),
}

impl PrismLiveApplyError {
    fn definitely_unapplied(error: impl Into<anyhow::Error>) -> Self {
        Self::DefinitelyUnapplied(error.into())
    }

    fn outcome_unknown(error: impl Into<anyhow::Error>) -> Self {
        Self::OutcomeUnknown(error.into())
    }
}

impl std::fmt::Display for PrismLiveApplyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DefinitelyUnapplied(error) | Self::OutcomeUnknown(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PrismLiveApplyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::DefinitelyUnapplied(error) | Self::OutcomeUnknown(error) => error.source(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrismLiveTestFault {
    AfterAgentMutation,
    AfterHumanSync,
}

pub struct PrismLiveSessions {
    project_path: PathBuf,
    #[cfg(test)]
    test_fault: Option<PrismLiveTestFault>,
}

impl PrismLiveSessions {
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
        action: PrismLiveAction,
    ) -> std::result::Result<PrismLiveResult, PrismLiveApplyError> {
        action
            .validate()
            .map_err(PrismLiveApplyError::definitely_unapplied)?;
        let human_before = durable_state(human, "bound human workspace")
            .map_err(PrismLiveApplyError::definitely_unapplied)?;
        self.require_same_project_path(human)
            .map_err(PrismLiveApplyError::definitely_unapplied)?;
        let collaboration = Workspace::collaboration(&self.project_path, agent_session)
            .map_err(PrismLiveApplyError::definitely_unapplied)?;
        authorize(&human_before, &collaboration, agent_session)
            .map_err(PrismLiveApplyError::definitely_unapplied)?;
        let mut agent = Workspace::open_session(&self.project_path, agent_session)
            .map_err(PrismLiveApplyError::definitely_unapplied)?;
        let agent_before = durable_state(&agent, "agent workspace")
            .map_err(PrismLiveApplyError::definitely_unapplied)?;
        authorize_agent_state(&human_before, &agent_before, &collaboration)
            .map_err(PrismLiveApplyError::definitely_unapplied)?;
        let before = PrismLiveState::new(&human_before, &agent_before, collaboration.clone());

        if matches!(action, PrismLiveAction::State) {
            return Ok(PrismLiveResult::State(before));
        }
        let expectation = action
            .expectation()
            .context("mutating live action lost its expectation")
            .map_err(PrismLiveApplyError::definitely_unapplied)?;
        check_expectation(expectation, &human_before, &agent_before, &collaboration)
            .map_err(PrismLiveApplyError::definitely_unapplied)?;

        let prepared = match action {
            PrismLiveAction::State => unreachable!(),
            PrismLiveAction::ExecuteBatch { commands, .. } => agent.prepare_live_batch(commands),
            PrismLiveAction::Undo { .. } => agent.prepare_live_undo(),
            PrismLiveAction::Redo { .. } => agent.prepare_live_redo(),
            PrismLiveAction::MoveAgentCursor { target, .. } => agent.prepare_live_move(target),
        }
        .map_err(PrismLiveApplyError::definitely_unapplied)?;
        let outputs = agent
            .commit_live_mutation(prepared)
            .map_err(PrismLiveApplyError::outcome_unknown)?;
        self.maybe_fail(PrismLiveTestFault::AfterAgentMutation)?;

        let sync = if collaboration.mode == CollaborationMode::Together {
            let sync = human
                .sync_together()
                .map_err(PrismLiveApplyError::outcome_unknown)?;
            self.maybe_fail(PrismLiveTestFault::AfterHumanSync)?;
            Some(sync)
        } else {
            None
        };
        let collaboration = Workspace::collaboration(&self.project_path, agent_session)
            .map_err(PrismLiveApplyError::outcome_unknown)?;
        let human_after = durable_state(human, "bound human workspace")
            .map_err(PrismLiveApplyError::outcome_unknown)?;
        let agent_after = durable_state(&agent, "agent workspace")
            .map_err(PrismLiveApplyError::outcome_unknown)?;
        let committed_revision = (agent_after.cursor != agent_before.cursor
            && agent_after.revision.parent_id == Some(agent_before.cursor))
        .then(|| agent_after.revision.clone());
        let after = PrismLiveState::new(&human_after, &agent_after, collaboration);
        Ok(PrismLiveResult::Applied(Box::new(PrismLiveApplied {
            before,
            after,
            outputs,
            committed_revision,
            collaboration_sync: sync.map(Into::into),
        })))
    }

    #[cfg(test)]
    pub(crate) fn inject_fault(&mut self, fault: PrismLiveTestFault) {
        self.test_fault = Some(fault);
    }

    #[cfg(test)]
    fn maybe_fail(
        &mut self,
        phase: PrismLiveTestFault,
    ) -> std::result::Result<(), PrismLiveApplyError> {
        if self.test_fault == Some(phase) {
            self.test_fault = None;
            return Err(PrismLiveApplyError::outcome_unknown(anyhow::anyhow!(
                "injected Prism live failure at {phase:?}"
            )));
        }
        Ok(())
    }

    #[cfg(not(test))]
    fn maybe_fail(
        &mut self,
        _phase: PrismLiveTestFault,
    ) -> std::result::Result<(), PrismLiveApplyError> {
        Ok(())
    }

    fn require_same_project_path(&self, human: &Workspace) -> Result<()> {
        let human_path = human
            .project_path
            .as_deref()
            .context("bound human workspace has no durable project path")?;
        let expected = std::fs::canonicalize(&self.project_path)?;
        let actual = std::fs::canonicalize(human_path)?;
        if actual != expected {
            bail!("live binding project path no longer matches the human workspace");
        }
        Ok(())
    }
}

fn durable_state(workspace: &Workspace, label: &str) -> Result<LiveWorkspaceState> {
    workspace
        .live_state()?
        .with_context(|| format!("{label} is not a durable Prism project"))
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
        || collaboration.track_id != human.track_id
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
        || agent.track_id != human.track_id
    {
        bail!("agent session identity does not match the bound project");
    }
    Ok(())
}

fn check_expectation(
    expectation: &crate::PrismLiveActionExpectation,
    human: &LiveWorkspaceState,
    agent: &LiveWorkspaceState,
    collaboration: &Collaboration,
) -> Result<()> {
    if expectation.agent_revision != agent.cursor {
        bail!(
            "stale agent cursor: expected {}, current {}",
            expectation.agent_revision,
            agent.cursor
        );
    }
    match collaboration.mode {
        CollaborationMode::Together => {
            let source = expectation
                .source_revision
                .context("Together actions require an exact source cursor")?;
            if source != human.cursor || source != collaboration.followed_revision {
                bail!(
                    "stale Together source cursor: expected {source}, current {}",
                    human.cursor
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
