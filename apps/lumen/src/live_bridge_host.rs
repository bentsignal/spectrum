use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
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
    LUMEN_LIVE_ACTION_FAMILY, LUMEN_LIVE_ACTION_VERSION, LumenLiveAction, LumenLiveApplyError,
    LumenLiveResult, LumenLiveSessions, Workspace, decode_live_action,
    live_bridge::LumenLiveCollaborationSync,
};

pub const LUMEN_LIVE_INGRESS_CAPACITY: usize = 32;
pub const LUMEN_LIVE_MAX_DEFERRED: usize = 16;
pub const LUMEN_LIVE_DEFERRED_TTL: Duration = Duration::from_secs(5);
pub const LUMEN_LIVE_DRAIN_COUNT_BUDGET: usize = 8;
pub const LUMEN_LIVE_DRAIN_TIME_BUDGET: Duration = Duration::from_millis(2);
#[cfg(not(test))]
const GUI_REPLY_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
const GUI_REPLY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LumenLiveInteractionState {
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

#[derive(Clone, Copy)]
struct BoundCatalogState {
    project_id: ProjectId,
    track_id: TrackId,
    session_id: SessionId,
    cursor: RevisionId,
}

struct PendingRequest {
    request: RequestEnvelope,
    action: LumenLiveAction,
    reply: Option<SyncSender<BridgeResult<HostApplyOutcome>>>,
    deferred_at: Option<Instant>,
    retained_bytes: usize,
    retained_total: Arc<AtomicUsize>,
}

impl Drop for PendingRequest {
    fn drop(&mut self) {
        self.retained_total
            .fetch_sub(self.retained_bytes, Ordering::AcqRel);
    }
}

pub struct LumenLiveHost {
    state: Mutex<HostState>,
    ingress: SyncSender<PendingRequest>,
    pending_ingress: Arc<AtomicUsize>,
    retained_request_bytes: Arc<AtomicUsize>,
    events: OnceLock<Arc<EventLog>>,
    wake_gui: Arc<dyn Fn() + Send + Sync>,
}

pub struct LumenLiveDrain {
    host: Arc<LumenLiveHost>,
    ingress: Receiver<PendingRequest>,
    pending_ingress: Arc<AtomicUsize>,
    deferred: VecDeque<PendingRequest>,
    sessions: LumenLiveSessions,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LumenLiveDrainReport {
    pub received: usize,
    pub last_receive_started_after: Duration,
    pub applied: usize,
    pub deferred: usize,
    pub refused: usize,
    pub workspace_changed: bool,
    pub outcome_unknown: bool,
    pub reopen_required: bool,
}

impl LumenLiveHost {
    pub fn new(
        workspace: &Workspace,
        binding_id: BindingId,
        binding_epoch: u64,
    ) -> Result<(Arc<Self>, LumenLiveDrain)> {
        Self::new_with_wake(workspace, binding_id, binding_epoch, Arc::new(|| {}))
    }

    pub fn new_with_wake(
        workspace: &Workspace,
        binding_id: BindingId,
        binding_epoch: u64,
        wake_gui: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<(Arc<Self>, LumenLiveDrain)> {
        let live = durable_state(workspace)?;
        let project_path = workspace
            .catalog_path
            .clone()
            .context("live binding requires a durable project path")?;
        let (sender, receiver) = mpsc::sync_channel(LUMEN_LIVE_INGRESS_CAPACITY);
        let pending_ingress = Arc::new(AtomicUsize::new(0));
        let retained_request_bytes = Arc::new(AtomicUsize::new(0));
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
            pending_ingress: Arc::clone(&pending_ingress),
            retained_request_bytes: Arc::clone(&retained_request_bytes),
            events: OnceLock::new(),
            wake_gui,
        });
        let drain = LumenLiveDrain {
            host: Arc::clone(&host),
            ingress: receiver,
            pending_ingress,
            deferred: VecDeque::new(),
            sessions: LumenLiveSessions::new(project_path),
        };
        Ok((host, drain))
    }

    pub fn attach_events(&self, events: Arc<EventLog>) -> Result<()> {
        self.events
            .set(events)
            .map_err(|_| anyhow::anyhow!("live event log is already attached"))
    }

    pub fn close(&self) {
        if self.mark_closed() {
            self.publish_closed();
        }
    }

    fn mark_closed(&self) -> bool {
        if let Ok(mut state) = self.state.lock()
            && !state.closed
        {
            state.closed = true;
            return true;
        }
        false
    }

    fn publish_closed(&self) {
        if let Some(events) = self.events.get() {
            let _ = events.append(BridgeEventKind::ProjectClosed);
        }
    }

    pub fn binding_id(&self) -> BindingId {
        self.state
            .lock()
            .expect("Lumen live host state poisoned")
            .binding_id
    }

    pub fn retained_request_bytes(&self) -> usize {
        self.retained_request_bytes.load(Ordering::Acquire)
    }

    pub fn observe_workspace(&self, workspace: &Workspace) -> Result<bool> {
        self.observe_workspace_interaction(workspace, None)
    }

    pub fn begin_workspace_interaction(
        &self,
        workspace: &Workspace,
        interaction_id: &str,
        interaction_kind: &str,
    ) -> Result<RevisionId> {
        let live = durable_state(workspace)?;
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("Lumen live host state is poisoned"))?;
        if state.closed {
            anyhow::bail!("Lumen live host is closed");
        }
        validate_bound_workspace(&state, &live)?;
        self.events
            .get()
            .context("Lumen live event log is not attached")?
            .append(BridgeEventKind::InteractionBegan {
                interaction_id: interaction_id.into(),
                interaction_kind: interaction_kind.into(),
            })?;
        Ok(live.cursor)
    }

    pub fn cancel_workspace_interaction(&self, interaction_id: &str) {
        if let Ok(state) = self.state.lock()
            && !state.closed
            && let Some(events) = self.events.get()
        {
            let _ = events.append(BridgeEventKind::InteractionCanceled {
                interaction_id: interaction_id.into(),
            });
        }
    }

    pub fn observe_workspace_interaction(
        &self,
        workspace: &Workspace,
        completed_interaction: Option<(&str, RevisionId)>,
    ) -> Result<bool> {
        let live = durable_state(workspace)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("Lumen live host state is poisoned"))?;
        if state.closed {
            return Ok(false);
        }
        validate_bound_workspace(&state, &live)?;
        let changed = live.cursor != state.human_cursor;
        if changed {
            publish_workspace_advance(
                self.events
                    .get()
                    .context("Lumen live event log is not attached")?,
                &state,
                &live,
            )?;
            state.human_cursor = live.cursor;
        }
        if let Some((interaction_id, started_at)) = completed_interaction {
            let events = self
                .events
                .get()
                .context("Lumen live event log is not attached")?;
            if started_at != live.cursor {
                events.append(BridgeEventKind::InteractionCommitted {
                    interaction_id: interaction_id.into(),
                    cursors: vec![CursorTransition {
                        track_id: live.track_id,
                        from_revision_id: started_at,
                        to_revision_id: live.cursor,
                    }],
                })?;
            } else {
                events.append(BridgeEventKind::InteractionCanceled {
                    interaction_id: interaction_id.into(),
                })?;
            }
        }
        Ok(changed)
    }

    fn cursors(state: &HostState) -> Vec<ExpectedCursor> {
        vec![ExpectedCursor {
            track_id: state.track_id,
            revision_id: state.human_cursor,
        }]
    }
}

fn validate_bound_workspace(state: &HostState, live: &BoundCatalogState) -> Result<()> {
    if live.project_id != state.project_id
        || live.track_id != state.track_id
        || live.session_id != state.human_session
    {
        anyhow::bail!("Lumen live binding no longer matches its workspace");
    }
    Ok(())
}

fn publish_workspace_advance(
    events: &EventLog,
    state: &HostState,
    live: &BoundCatalogState,
) -> Result<()> {
    let previous = state.human_cursor;
    let transition = vec![CursorTransition {
        track_id: live.track_id,
        from_revision_id: previous,
        to_revision_id: live.cursor,
    }];
    events.append(BridgeEventKind::CursorMoved {
        session_id: live.session_id,
        cursors: transition,
    })?;
    Ok(())
}

impl BridgeHost for LumenLiveHost {
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
            LUMEN_LIVE_ACTION_FAMILY.into(),
            ProtocolRange {
                minimum: LUMEN_LIVE_ACTION_VERSION,
                maximum: LUMEN_LIVE_ACTION_VERSION,
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
                code: "lumen_live_not_ready".into(),
                message: "Lumen live event publication is not attached".into(),
            }));
        }
        let action = match decode_live_action(&request.action) {
            Ok(action) => action,
            Err(error) => {
                return Ok(HostApplyOutcome::Applied(ResponseBody::Error {
                    code: "invalid_lumen_action".into(),
                    message: format!("{error:#}"),
                }));
            }
        };
        let (reply, response) = mpsc::sync_channel(1);
        let retained_bytes = retained_request_bytes(request, &action);
        let state = self.state.lock().map_err(|_| BridgeError::Closed)?;
        if state.closed {
            return Err(BridgeError::Closed);
        }
        self.pending_ingress.fetch_add(1, Ordering::AcqRel);
        self.retained_request_bytes
            .fetch_add(retained_bytes, Ordering::AcqRel);
        if let Err(error) = self.ingress.try_send(PendingRequest {
            request: request.clone(),
            action,
            reply: Some(reply),
            deferred_at: None,
            retained_bytes,
            retained_total: Arc::clone(&self.retained_request_bytes),
        }) {
            decrement_pending_ingress(&self.pending_ingress);
            return Err(match error {
                mpsc::TrySendError::Full(_) => BridgeError::RateLimited {
                    retry_after_millis: 10,
                    disconnect: false,
                },
                mpsc::TrySendError::Disconnected(_) => BridgeError::Closed,
            });
        }
        drop(state);
        (self.wake_gui)();
        response
            .recv_timeout(GUI_REPLY_TIMEOUT)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => BridgeError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Lumen GUI did not answer the live request within 30 seconds",
                )),
                mpsc::RecvTimeoutError::Disconnected => BridgeError::Closed,
            })?
    }
}

impl LumenLiveDrain {
    pub fn drain(
        &mut self,
        workspace: &mut Workspace,
        interaction: LumenLiveInteractionState,
    ) -> LumenLiveDrainReport {
        let started = Instant::now();
        let mut report = LumenLiveDrainReport::default();
        self.expire_deferred(&mut report);
        if interaction == LumenLiveInteractionState::Idle {
            self.release_one_deferred(workspace, &mut report);
            if report.applied > 0 {
                return report;
            }
        }
        loop {
            let receive_started_after = started.elapsed();
            if report.received >= LUMEN_LIVE_DRAIN_COUNT_BUDGET
                || (report.received > 0 && receive_started_after >= LUMEN_LIVE_DRAIN_TIME_BUDGET)
            {
                break;
            }
            let mut pending = match self.try_receive() {
                Ok(pending) => pending,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            };
            report.received += 1;
            report.last_receive_started_after = receive_started_after;
            if pending.request.interaction == InteractionPolicy::RequireUserConfirmation
                && !matches!(pending.action, LumenLiveAction::State)
            {
                send_reply(
                    pending.reply.take(),
                    Ok(HostApplyOutcome::Applied(ResponseBody::Refused {
                        reason: "Lumen does not yet provide live-action confirmation UI".into(),
                    })),
                );
                report.refused += 1;
            } else if interaction == LumenLiveInteractionState::Active
                && !matches!(pending.action, LumenLiveAction::State)
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
        self.pending_ingress.load(Ordering::Acquire) > 0 || !self.deferred.is_empty()
    }

    pub fn close(&mut self) {
        let publish_closed = self.host.mark_closed();
        while let Ok(mut pending) = self.try_receive() {
            self.cancel_deferred_event(&pending);
            send_reply(
                pending.reply.take(),
                Ok(HostApplyOutcome::Applied(ResponseBody::Refused {
                    reason: "Lumen live binding closed before the request could run".into(),
                })),
            );
        }
        while let Some(pending) = self.deferred.pop_front() {
            self.cancel_deferred_event(&pending);
        }
        debug_assert_eq!(self.pending_ingress.load(Ordering::Acquire), 0);
        if publish_closed {
            self.host.publish_closed();
        }
    }

    fn try_receive(&self) -> Result<PendingRequest, TryRecvError> {
        let pending = self.ingress.try_recv()?;
        decrement_pending_ingress(&self.pending_ingress);
        Ok(pending)
    }

    pub fn pending_ingress_count(&self) -> usize {
        self.pending_ingress.load(Ordering::Acquire)
    }

    pub fn deferred_count(&self) -> usize {
        self.deferred.len()
    }

    fn handle_busy(&mut self, mut pending: PendingRequest, report: &mut LumenLiveDrainReport) {
        match pending.request.interaction {
            InteractionPolicy::Deferred if self.deferred.len() < LUMEN_LIVE_MAX_DEFERRED => {
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
                            "Lumen live deferred queue is full"
                        } else {
                            "Lumen is in an active semantic interaction"
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
        report: &mut LumenLiveDrainReport,
    ) {
        if let Some(pending) = self.deferred.pop_front() {
            self.apply_one(workspace, pending, report);
        }
    }

    fn expire_deferred(&mut self, report: &mut LumenLiveDrainReport) {
        let now = Instant::now();
        while self.deferred.front().is_some_and(|pending| {
            pending
                .deferred_at
                .is_some_and(|created| now.duration_since(created) >= LUMEN_LIVE_DEFERRED_TTL)
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
        report: &mut LumenLiveDrainReport,
    ) {
        let was_deferred = pending.deferred_at.is_some();
        let project_before = workspace.project.clone();
        let result = self.apply_locked(workspace, &pending);
        let outcome_unknown = result.is_err();
        let workspace_changed = workspace.project != project_before;
        report.workspace_changed |= workspace_changed;
        report.outcome_unknown |= outcome_unknown;
        report.reopen_required |= outcome_unknown;
        let mutation_applied = !matches!(pending.action, LumenLiveAction::State)
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
        if was_deferred && !mutation_applied && !outcome_unknown {
            self.cancel_deferred_event(&pending);
        } else if was_deferred && outcome_unknown {
            self.outcome_unknown_deferred_event(&pending);
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
        let live = match durable_state(workspace) {
            Ok(live) => live,
            Err(error) => {
                return Ok(HostApplyOutcome::Applied(ResponseBody::Refused {
                    reason: bounded_reason(&format!("{error:#}")),
                }));
            }
        };
        if live.project_id != host.project_id
            || live.track_id != host.track_id
            || live.session_id != host.human_session
        {
            return Ok(HostApplyOutcome::Applied(ResponseBody::Refused {
                reason: "live binding no longer matches the active Lumen workspace".into(),
            }));
        }
        host.human_cursor = live.cursor;
        let current = LumenLiveHost::cursors(&host);
        if pending.request.expected_cursors != current {
            return Ok(HostApplyOutcome::Conflict(current));
        }
        let result = match self.sessions.apply(
            workspace,
            pending.request.session_id,
            pending.action.clone(),
        ) {
            Ok(result) => result,
            Err(LumenLiveApplyError::DefinitelyUnapplied(error)) => {
                return Ok(HostApplyOutcome::Applied(ResponseBody::Refused {
                    reason: bounded_reason(&format!("{error:#}")),
                }));
            }
            Err(LumenLiveApplyError::OutcomeUnknown(error)) => {
                if let Ok(after) = durable_state(workspace) {
                    host.human_cursor = after.cursor;
                }
                host.closed = true;
                return Err(BridgeError::Protocol(format!(
                    "Lumen live mutation outcome is unknown; inspect current state before retrying: {}",
                    bounded_reason(&format!("{error:#}"))
                )));
            }
        };
        let after = durable_state(workspace).map_err(protocol_error)?;
        host.human_cursor = after.cursor;
        let value = serde_json::to_value(&result).map_err(BridgeError::Json)?;
        self.publish_events(&result, pending)?;
        Ok(HostApplyOutcome::Applied(ResponseBody::Applied {
            result: value,
            cursors: LumenLiveHost::cursors(&host),
        }))
    }

    fn publish_events(
        &self,
        result: &LumenLiveResult,
        pending: &PendingRequest,
    ) -> BridgeResult<()> {
        let Some(events) = self.host.events.get() else {
            return Err(BridgeError::Protocol(
                "Lumen live event log is not attached".into(),
            ));
        };
        let LumenLiveResult::Applied(applied) = result else {
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
                    .unwrap_or_else(|| "Lumen edit".into()),
                cursors: vec![CursorTransition {
                    track_id: applied.after.photo_track_id,
                    from_revision_id: applied.before.agent_photo_cursor,
                    to_revision_id: applied.after.agent_photo_cursor,
                }],
            })?;
        } else if applied.before.agent_photo_cursor != applied.after.agent_photo_cursor {
            events.append(BridgeEventKind::CursorMoved {
                session_id: applied.after.agent_session,
                cursors: vec![CursorTransition {
                    track_id: applied.after.photo_track_id,
                    from_revision_id: applied.before.agent_photo_cursor,
                    to_revision_id: applied.after.agent_photo_cursor,
                }],
            })?;
        }
        if let Some(sync) = &applied.collaboration_sync {
            let transition = || {
                vec![CursorTransition {
                    track_id: applied.after.photo_track_id,
                    from_revision_id: applied.before.human_photo_cursor,
                    to_revision_id: applied.after.human_photo_cursor,
                }]
            };
            match sync {
                LumenLiveCollaborationSync::Advanced { .. }
                    if applied.before.human_photo_cursor != applied.after.human_photo_cursor =>
                {
                    events.append(BridgeEventKind::CollaborationAdvanced {
                        agent_session_id: applied.after.agent_session,
                        source_session_id: applied.after.human_session,
                        cursors: transition(),
                    })?;
                }
                LumenLiveCollaborationSync::Split
                    if applied.before.agent_photo_cursor != applied.after.agent_photo_cursor =>
                {
                    events.append(BridgeEventKind::CollaborationSplit {
                        agent_session_id: applied.after.agent_session,
                        source_session_id: applied.after.human_session,
                        cursors: vec![CursorTransition {
                            track_id: applied.after.photo_track_id,
                            from_revision_id: applied.before.agent_photo_cursor,
                            to_revision_id: applied.after.agent_photo_cursor,
                        }],
                    })?;
                }
                _ => {}
            }
        }
        if pending.deferred_at.is_some() {
            let cursors = if applied.before.agent_photo_cursor != applied.after.agent_photo_cursor {
                vec![CursorTransition {
                    track_id: applied.after.photo_track_id,
                    from_revision_id: applied.before.agent_photo_cursor,
                    to_revision_id: applied.after.agent_photo_cursor,
                }]
            } else {
                vec![CursorTransition {
                    track_id: applied.after.photo_track_id,
                    from_revision_id: applied.before.human_photo_cursor,
                    to_revision_id: applied.after.human_photo_cursor,
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
            .ok_or_else(|| BridgeError::Protocol("Lumen live event log is not attached".into()))?
            .append(BridgeEventKind::InteractionBegan {
                interaction_id: deferred_interaction_id(&pending.request),
                interaction_kind: "deferred_lumen_action".into(),
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

    fn outcome_unknown_deferred_event(&self, pending: &PendingRequest) {
        if let Ok(_state) = self.host.state.lock()
            && let Some(events) = self.host.events.get()
        {
            let _ = events.append(BridgeEventKind::InteractionOutcomeUnknown {
                interaction_id: deferred_interaction_id(&pending.request),
            });
        }
    }

    #[cfg(test)]
    pub(crate) fn inject_session_fault(
        &mut self,
        fault: crate::live_bridge_sessions::LumenLiveTestFault,
    ) {
        self.sessions.inject_fault(fault);
    }
}

fn durable_state(workspace: &Workspace) -> Result<BoundCatalogState> {
    let (project_id, track_id, cursor, session_id) = workspace
        .live_catalog_identity()
        .context("Lumen live bridge requires a durable workspace")?;
    Ok(BoundCatalogState {
        project_id,
        track_id,
        session_id,
        cursor,
    })
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

fn decrement_pending_ingress(pending: &AtomicUsize) {
    pending
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            count.checked_sub(1)
        })
        .expect("Lumen live ingress accounting underflow");
}

fn retained_request_bytes(request: &RequestEnvelope, action: &LumenLiveAction) -> usize {
    std::mem::size_of::<PendingRequest>()
        + serde_json::to_vec(request).map_or(0, |encoded| encoded.len())
        + serde_json::to_vec(action).map_or(0, |encoded| encoded.len())
}

fn deferred_interaction_id(request: &RequestEnvelope) -> String {
    format!("deferred-request:{}", request.request_id)
}
