use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex, OnceLock,
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use spectrum_live_bridge::{
    BindingId, BridgeError, BridgeEventKind, BridgeHost, BridgeResult, CursorTransition,
    EventActor, EventActorKind, EventLog, ExpectedCursor, HostApplyOutcome, InteractionPolicy,
    ProtocolRange, RequestEnvelope, ResponseBody, StateSnapshot,
};
use spectrum_revisions::{ActorKind, ProjectId, RevisionId, SessionId, TrackId};

use crate::{
    LiveWorkspaceState, PRISM_LIVE_ACTION_FAMILY, PRISM_LIVE_ACTION_VERSION, PrismLiveAction,
    PrismLiveResult, PrismLiveSessions, Workspace, decode_live_action,
    live_bridge::PrismLiveCollaborationSync,
};

const INGRESS_CAPACITY: usize = 32;
const MAX_DEFERRED: usize = 16;
const DEFERRED_TTL: Duration = Duration::from_secs(5);
const DRAIN_COUNT_BUDGET: usize = 8;
const DRAIN_TIME_BUDGET: Duration = Duration::from_millis(2);
#[cfg(not(test))]
const GUI_REPLY_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
const GUI_REPLY_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrismLiveInteractionState {
    Idle,
    Active,
}

struct HostState {
    project_id: ProjectId,
    binding_id: BindingId,
    binding_epoch: u64,
    track_id: TrackId,
    human_session: SessionId,
    human_cursor: RevisionId,
    closed: bool,
}

struct PendingRequest {
    request: RequestEnvelope,
    action: PrismLiveAction,
    reply: Option<SyncSender<BridgeResult<HostApplyOutcome>>>,
    deferred_at: Option<Instant>,
}

pub struct PrismLiveHost {
    state: Mutex<HostState>,
    ingress: SyncSender<PendingRequest>,
    events: OnceLock<Arc<EventLog>>,
}

pub struct PrismLiveDrain {
    host: Arc<PrismLiveHost>,
    ingress: Receiver<PendingRequest>,
    deferred: VecDeque<PendingRequest>,
    sessions: PrismLiveSessions,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrismLiveDrainReport {
    pub received: usize,
    pub applied: usize,
    pub deferred: usize,
    pub refused: usize,
}

impl PrismLiveHost {
    pub fn new(
        workspace: &Workspace,
        binding_id: BindingId,
        binding_epoch: u64,
    ) -> Result<(Arc<Self>, PrismLiveDrain)> {
        let live = durable_state(workspace)?;
        let project_path = workspace
            .project_path
            .clone()
            .context("live binding requires a durable project path")?;
        let (sender, receiver) = mpsc::sync_channel(INGRESS_CAPACITY);
        let host = Arc::new(Self {
            state: Mutex::new(HostState {
                project_id: live.project_id,
                binding_id,
                binding_epoch,
                track_id: live.track_id,
                human_session: live.session_id,
                human_cursor: live.cursor,
                closed: false,
            }),
            ingress: sender,
            events: OnceLock::new(),
        });
        let drain = PrismLiveDrain {
            host: Arc::clone(&host),
            ingress: receiver,
            deferred: VecDeque::new(),
            sessions: PrismLiveSessions::new(project_path),
        };
        Ok((host, drain))
    }

    pub fn attach_events(&self, events: Arc<EventLog>) -> Result<()> {
        self.events
            .set(events)
            .map_err(|_| anyhow::anyhow!("live event log is already attached"))
    }

    pub fn close(&self) {
        if let Ok(mut state) = self.state.lock()
            && !state.closed
        {
            if let Some(events) = self.events.get() {
                let _ = events.append(BridgeEventKind::ProjectClosed);
            }
            state.closed = true;
        }
    }

    pub fn binding_id(&self) -> BindingId {
        self.state
            .lock()
            .expect("Prism live host state poisoned")
            .binding_id
    }

    fn cursors(state: &HostState) -> Vec<ExpectedCursor> {
        vec![ExpectedCursor {
            track_id: state.track_id,
            revision_id: state.human_cursor,
        }]
    }
}

impl BridgeHost for PrismLiveHost {
    fn current_cursors(&self) -> BridgeResult<Vec<ExpectedCursor>> {
        let state = self.state.lock().map_err(|_| BridgeError::Closed)?;
        if state.closed {
            return Err(BridgeError::Closed);
        }
        Ok(Self::cursors(&state))
    }

    fn with_snapshot<R>(
        &self,
        attach: impl FnOnce(StateSnapshot) -> BridgeResult<R>,
    ) -> BridgeResult<R> {
        let state = self.state.lock().map_err(|_| BridgeError::Closed)?;
        if state.closed {
            return Err(BridgeError::Closed);
        }
        let mut application_protocols = BTreeMap::new();
        application_protocols.insert(
            PRISM_LIVE_ACTION_FAMILY.into(),
            ProtocolRange {
                minimum: PRISM_LIVE_ACTION_VERSION,
                maximum: PRISM_LIVE_ACTION_VERSION,
            },
        );
        attach(StateSnapshot {
            project_id: state.project_id,
            binding_id: state.binding_id,
            binding_epoch: state.binding_epoch,
            cursors: Self::cursors(&state),
            current_event_seq: 0,
            application_protocols,
            application_state: BTreeMap::new(),
        })
    }

    fn apply_if_current(&self, request: &RequestEnvelope) -> BridgeResult<HostApplyOutcome> {
        if self.events.get().is_none() {
            return Ok(HostApplyOutcome::Applied(ResponseBody::Error {
                code: "prism_live_not_ready".into(),
                message: "Prism live event publication is not attached".into(),
            }));
        }
        let action = match decode_live_action(&request.action) {
            Ok(action) => action,
            Err(error) => {
                return Ok(HostApplyOutcome::Applied(ResponseBody::Error {
                    code: "invalid_prism_action".into(),
                    message: format!("{error:#}"),
                }));
            }
        };
        let (reply, response) = mpsc::sync_channel(1);
        self.ingress
            .try_send(PendingRequest {
                request: request.clone(),
                action,
                reply: Some(reply),
                deferred_at: None,
            })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => BridgeError::RateLimited {
                    retry_after_millis: 10,
                    disconnect: false,
                },
                mpsc::TrySendError::Disconnected(_) => BridgeError::Closed,
            })?;
        response
            .recv_timeout(GUI_REPLY_TIMEOUT)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => BridgeError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Prism GUI did not answer the live request within 30 seconds",
                )),
                mpsc::RecvTimeoutError::Disconnected => BridgeError::Closed,
            })?
    }
}

impl PrismLiveDrain {
    pub fn drain(
        &mut self,
        workspace: &mut Workspace,
        interaction: PrismLiveInteractionState,
    ) -> PrismLiveDrainReport {
        let started = Instant::now();
        let mut report = PrismLiveDrainReport::default();
        self.expire_deferred(&mut report);
        if interaction == PrismLiveInteractionState::Idle {
            self.release_one_deferred(workspace, &mut report);
            if report.applied > 0 {
                return report;
            }
        }
        while report.received < DRAIN_COUNT_BUDGET && started.elapsed() < DRAIN_TIME_BUDGET {
            let pending = match self.ingress.try_recv() {
                Ok(pending) => pending,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            };
            report.received += 1;
            if pending.request.interaction == InteractionPolicy::RequireUserConfirmation
                && !matches!(pending.action, PrismLiveAction::State)
            {
                send_reply(
                    pending.reply,
                    Ok(HostApplyOutcome::Applied(ResponseBody::Refused {
                        reason: "Prism does not yet provide live-action confirmation UI".into(),
                    })),
                );
                report.refused += 1;
            } else if interaction == PrismLiveInteractionState::Active
                && !matches!(pending.action, PrismLiveAction::State)
            {
                self.handle_busy(pending, &mut report);
            } else {
                self.apply_one(workspace, pending, &mut report);
                if report.applied > 0 {
                    break;
                }
            }
        }
        report
    }

    pub fn has_pending(&self) -> bool {
        !self.deferred.is_empty()
    }

    pub fn close(&mut self) {
        while let Ok(mut pending) = self.ingress.try_recv() {
            self.cancel_deferred_event(&pending);
            send_reply(
                pending.reply.take(),
                Ok(HostApplyOutcome::Applied(ResponseBody::Refused {
                    reason: "Prism live binding closed before the request could run".into(),
                })),
            );
        }
        while let Some(pending) = self.deferred.pop_front() {
            self.cancel_deferred_event(&pending);
        }
        self.host.close();
    }

    fn handle_busy(&mut self, mut pending: PendingRequest, report: &mut PrismLiveDrainReport) {
        match pending.request.interaction {
            InteractionPolicy::Deferred if self.deferred.len() < MAX_DEFERRED => {
                pending.deferred_at = Some(Instant::now());
                if let Err(error) = self.begin_deferred_event(&pending) {
                    send_reply(pending.reply.take(), Err(error));
                    report.refused += 1;
                    return;
                }
                send_reply(
                    pending.reply.take(),
                    Ok(HostApplyOutcome::Applied(ResponseBody::Deferred)),
                );
                self.deferred.push_back(pending);
                report.deferred += 1;
            }
            InteractionPolicy::Immediate
            | InteractionPolicy::Deferred
            | InteractionPolicy::RequireUserConfirmation => {
                send_reply(
                    pending.reply.take(),
                    Ok(HostApplyOutcome::Applied(ResponseBody::Refused {
                        reason: if pending.request.interaction == InteractionPolicy::Deferred {
                            "Prism live deferred queue is full"
                        } else {
                            "Prism is in an active semantic interaction"
                        }
                        .into(),
                    })),
                );
                report.refused += 1;
            }
        }
    }

    fn release_one_deferred(
        &mut self,
        workspace: &mut Workspace,
        report: &mut PrismLiveDrainReport,
    ) {
        if let Some(pending) = self.deferred.pop_front() {
            self.apply_one(workspace, pending, report);
        }
    }

    fn expire_deferred(&mut self, report: &mut PrismLiveDrainReport) {
        let now = Instant::now();
        while self.deferred.front().is_some_and(|pending| {
            pending
                .deferred_at
                .is_some_and(|created| now.duration_since(created) >= DEFERRED_TTL)
        }) {
            if let Some(pending) = self.deferred.pop_front() {
                self.cancel_deferred_event(&pending);
            }
            report.refused += 1;
        }
    }

    fn apply_one(
        &mut self,
        workspace: &mut Workspace,
        mut pending: PendingRequest,
        report: &mut PrismLiveDrainReport,
    ) {
        let was_deferred = pending.deferred_at.is_some();
        let result = self.apply_locked(workspace, &pending);
        let mutation_applied = !matches!(pending.action, PrismLiveAction::State)
            && matches!(
                &result,
                Ok(HostApplyOutcome::Applied(ResponseBody::Applied { .. }))
            );
        if mutation_applied {
            report.applied += 1;
        } else if matches!(
            &result,
            Err(_)
                | Ok(HostApplyOutcome::Applied(
                    ResponseBody::Refused { .. } | ResponseBody::Error { .. }
                ))
        ) {
            report.refused += 1;
        }
        if was_deferred && !mutation_applied {
            self.cancel_deferred_event(&pending);
        }
        send_reply(pending.reply.take(), result);
    }

    fn apply_locked(
        &mut self,
        workspace: &mut Workspace,
        pending: &PendingRequest,
    ) -> BridgeResult<HostApplyOutcome> {
        let mut host = self.host.state.lock().map_err(|_| BridgeError::Closed)?;
        if host.closed {
            return Err(BridgeError::Closed);
        }
        let live = durable_state(workspace).map_err(protocol_error)?;
        if live.project_id != host.project_id
            || live.track_id != host.track_id
            || live.session_id != host.human_session
        {
            return Ok(HostApplyOutcome::Applied(ResponseBody::Refused {
                reason: "live binding no longer matches the active Prism workspace".into(),
            }));
        }
        host.human_cursor = live.cursor;
        let current = PrismLiveHost::cursors(&host);
        if pending.request.expected_cursors != current {
            return Ok(HostApplyOutcome::Conflict(current));
        }
        let result = match self.sessions.apply(
            workspace,
            pending.request.session_id,
            pending.action.clone(),
        ) {
            Ok(result) => result,
            Err(error) => {
                return Ok(HostApplyOutcome::Applied(ResponseBody::Refused {
                    reason: bounded_reason(&format!("{error:#}")),
                }));
            }
        };
        let after = durable_state(workspace).map_err(protocol_error)?;
        host.human_cursor = after.cursor;
        self.publish_events(&result, pending)?;
        let value = serde_json::to_value(&result).map_err(BridgeError::Json)?;
        Ok(HostApplyOutcome::Applied(ResponseBody::Applied {
            result: value,
            cursors: PrismLiveHost::cursors(&host),
        }))
    }

    fn publish_events(
        &self,
        result: &PrismLiveResult,
        pending: &PendingRequest,
    ) -> BridgeResult<()> {
        let Some(events) = self.host.events.get() else {
            return Err(BridgeError::Protocol(
                "Prism live event log is not attached".into(),
            ));
        };
        let PrismLiveResult::Applied(applied) = result else {
            return Ok(());
        };
        if let Some(revision) = &applied.committed_revision {
            events.append(BridgeEventKind::RevisionCommitted {
                request_id: Some(pending.request.request_id),
                change_set_id: revision.change_set_id,
                actor: EventActor {
                    id: revision.actor.id.clone(),
                    display_name: revision.actor.display_name.clone(),
                    kind: actor_kind(revision.actor.kind),
                },
                session_id: revision.session_id,
                action_label: revision
                    .label
                    .clone()
                    .unwrap_or_else(|| "Prism edit".into()),
                cursors: vec![CursorTransition {
                    track_id: applied.after.track_id,
                    from_revision_id: applied.before.agent_cursor,
                    to_revision_id: applied.after.agent_cursor,
                }],
            })?;
        } else if applied.before.agent_cursor != applied.after.agent_cursor {
            events.append(BridgeEventKind::CursorMoved {
                session_id: applied.after.agent_session,
                cursors: vec![CursorTransition {
                    track_id: applied.after.track_id,
                    from_revision_id: applied.before.agent_cursor,
                    to_revision_id: applied.after.agent_cursor,
                }],
            })?;
        }
        if let Some(sync) = &applied.collaboration_sync {
            let transition = || {
                vec![CursorTransition {
                    track_id: applied.after.track_id,
                    from_revision_id: applied.before.human_cursor,
                    to_revision_id: applied.after.human_cursor,
                }]
            };
            match sync {
                PrismLiveCollaborationSync::Advanced { .. }
                    if applied.before.human_cursor != applied.after.human_cursor =>
                {
                    events.append(BridgeEventKind::CollaborationAdvanced {
                        agent_session_id: applied.after.agent_session,
                        source_session_id: applied.after.human_session,
                        cursors: transition(),
                    })?;
                }
                PrismLiveCollaborationSync::Split
                    if applied.before.agent_cursor != applied.after.agent_cursor =>
                {
                    events.append(BridgeEventKind::CollaborationSplit {
                        agent_session_id: applied.after.agent_session,
                        source_session_id: applied.after.human_session,
                        cursors: vec![CursorTransition {
                            track_id: applied.after.track_id,
                            from_revision_id: applied.before.agent_cursor,
                            to_revision_id: applied.after.agent_cursor,
                        }],
                    })?;
                }
                _ => {}
            }
        }
        if pending.deferred_at.is_some() {
            let cursors = if applied.before.agent_cursor != applied.after.agent_cursor {
                vec![CursorTransition {
                    track_id: applied.after.track_id,
                    from_revision_id: applied.before.agent_cursor,
                    to_revision_id: applied.after.agent_cursor,
                }]
            } else {
                vec![CursorTransition {
                    track_id: applied.after.track_id,
                    from_revision_id: applied.before.human_cursor,
                    to_revision_id: applied.after.human_cursor,
                }]
            };
            if cursors[0].from_revision_id != cursors[0].to_revision_id {
                events.append(BridgeEventKind::InteractionCommitted {
                    interaction_id: deferred_interaction_id(&pending.request),
                    cursors,
                })?;
            } else {
                events.append(BridgeEventKind::InteractionCanceled {
                    interaction_id: deferred_interaction_id(&pending.request),
                })?;
            }
        }
        Ok(())
    }

    fn begin_deferred_event(&self, pending: &PendingRequest) -> BridgeResult<()> {
        let _state = self.host.state.lock().map_err(|_| BridgeError::Closed)?;
        self.host
            .events
            .get()
            .ok_or_else(|| BridgeError::Protocol("Prism live event log is not attached".into()))?
            .append(BridgeEventKind::InteractionBegan {
                interaction_id: deferred_interaction_id(&pending.request),
                interaction_kind: "deferred_prism_action".into(),
            })?;
        Ok(())
    }

    fn cancel_deferred_event(&self, pending: &PendingRequest) {
        if pending.deferred_at.is_none() {
            return;
        }
        if let Ok(_state) = self.host.state.lock()
            && let Some(events) = self.host.events.get()
        {
            let _ = events.append(BridgeEventKind::InteractionCanceled {
                interaction_id: deferred_interaction_id(&pending.request),
            });
        }
    }
}

fn durable_state(workspace: &Workspace) -> Result<LiveWorkspaceState> {
    workspace
        .live_state()?
        .context("Prism live bridge requires a durable workspace")
}

fn actor_kind(kind: ActorKind) -> EventActorKind {
    match kind {
        ActorKind::Human => EventActorKind::Human,
        ActorKind::Agent => EventActorKind::Agent,
        ActorKind::System => EventActorKind::System,
    }
}

fn protocol_error(error: anyhow::Error) -> BridgeError {
    BridgeError::Protocol(format!("{error:#}"))
}

fn bounded_reason(reason: &str) -> String {
    if reason.len() <= spectrum_live_bridge::MAX_ERROR_BYTES {
        return reason.into();
    }
    let mut end = spectrum_live_bridge::MAX_ERROR_BYTES;
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    reason[..end].into()
}

fn send_reply(
    reply: Option<SyncSender<BridgeResult<HostApplyOutcome>>>,
    result: BridgeResult<HostApplyOutcome>,
) {
    if let Some(reply) = reply {
        let _ = reply.send(result);
    }
}

fn deferred_interaction_id(request: &RequestEnvelope) -> String {
    format!("deferred-request:{}", request.request_id)
}
