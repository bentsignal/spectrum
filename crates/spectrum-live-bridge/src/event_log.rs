use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::{
    BridgeError, BridgeEvent, BridgeEventKind, BridgeResult, EVENT_LOG_MAX_BYTES,
    EVENT_LOG_MAX_EVENTS, MAX_QUEUED_BYTES_PER_CONNECTION, MAX_QUEUED_BYTES_PER_HOST,
    MAX_SUBSCRIBER_BYTES, MAX_SUBSCRIBER_EVENTS,
};

#[derive(Default)]
struct SubscriberState {
    queue: VecDeque<(BridgeEvent, usize)>,
    queued_bytes: usize,
    resync: Option<(u64, u64)>,
    closed: bool,
}

struct Subscriber {
    state: Mutex<SubscriberState>,
    wake: Condvar,
    connection_queued: Arc<AtomicUsize>,
    host_queued: Arc<AtomicUsize>,
}

struct LogState {
    next_seq: u64,
    retained: VecDeque<(BridgeEvent, usize)>,
    retained_bytes: usize,
    subscribers: Vec<Weak<Subscriber>>,
}

pub struct EventLog {
    state: Mutex<LogState>,
    host_queued: Arc<AtomicUsize>,
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new()
    }
}

impl EventLog {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(LogState {
                next_seq: 1,
                retained: VecDeque::new(),
                retained_bytes: 0,
                subscribers: Vec::new(),
            }),
            host_queued: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn append(&self, event: BridgeEventKind) -> BridgeResult<u64> {
        let mut state = self.state.lock().map_err(|_| BridgeError::Closed)?;
        let seq = state.next_seq;
        let published = BridgeEvent { seq, event };
        published.validate()?;
        let encoded = published.encoded_len()?;
        if encoded > EVENT_LOG_MAX_BYTES {
            return Err(BridgeError::Limit("single event exceeds event log".into()));
        }
        let next_seq = seq
            .checked_add(1)
            .ok_or_else(|| BridgeError::Protocol("event sequence exhausted".into()))?;

        state.retained.push_back((published.clone(), encoded));
        state.retained_bytes += encoded;
        while state.retained.len() > EVENT_LOG_MAX_EVENTS
            || state.retained_bytes > EVENT_LOG_MAX_BYTES
        {
            if let Some((_, removed)) = state.retained.pop_front() {
                state.retained_bytes -= removed;
            }
        }
        let oldest = state.retained.front().map_or(seq, |(event, _)| event.seq);
        let newest = seq;
        state.subscribers.retain(|weak| {
            let Some(subscriber) = weak.upgrade() else {
                return false;
            };
            let Ok(mut target) = subscriber.state.lock() else {
                return false;
            };
            if target.closed || target.resync.is_some() {
                return true;
            }
            if target.queue.len() >= MAX_SUBSCRIBER_EVENTS
                || target.queued_bytes.saturating_add(encoded) > MAX_SUBSCRIBER_BYTES
                || !reserve_atomic(
                    &subscriber.connection_queued,
                    encoded,
                    MAX_QUEUED_BYTES_PER_CONNECTION,
                )
            {
                mark_resync(&subscriber, &mut target, oldest, newest);
                return true;
            }
            if !reserve_atomic(&subscriber.host_queued, encoded, MAX_QUEUED_BYTES_PER_HOST) {
                subscriber
                    .connection_queued
                    .fetch_sub(encoded, Ordering::AcqRel);
                mark_resync(&subscriber, &mut target, oldest, newest);
                return true;
            }
            target.queued_bytes += encoded;
            target.queue.push_back((published.clone(), encoded));
            subscriber.wake.notify_one();
            true
        });
        state.next_seq = next_seq;
        Ok(seq)
    }

    pub fn subscribe(&self, after_seq: u64) -> BridgeResult<Subscription> {
        self.subscribe_with_budget(after_seq, Arc::new(AtomicUsize::new(0)))
    }

    pub(crate) fn subscribe_with_budget(
        &self,
        after_seq: u64,
        connection_queued: Arc<AtomicUsize>,
    ) -> BridgeResult<Subscription> {
        let mut state = self.state.lock().map_err(|_| BridgeError::Closed)?;
        let oldest = state.retained.front().map(|(event, _)| event.seq);
        let newest = state.retained.back().map_or(0, |(event, _)| event.seq);
        if let Some(oldest) = oldest
            && after_seq.saturating_add(1) < oldest
        {
            return Err(BridgeError::ResyncRequired {
                oldest_seq: oldest,
                newest_seq: newest,
            });
        }
        let subscriber = Arc::new(Subscriber {
            state: Mutex::new(SubscriberState::default()),
            wake: Condvar::new(),
            connection_queued,
            host_queued: Arc::clone(&self.host_queued),
        });
        {
            let mut target = subscriber.state.lock().map_err(|_| BridgeError::Closed)?;
            for (event, bytes) in state
                .retained
                .iter()
                .filter(|(event, _)| event.seq > after_seq)
            {
                if target.queue.len() >= MAX_SUBSCRIBER_EVENTS
                    || target.queued_bytes.saturating_add(*bytes) > MAX_SUBSCRIBER_BYTES
                    || !reserve_atomic(
                        &subscriber.connection_queued,
                        *bytes,
                        MAX_QUEUED_BYTES_PER_CONNECTION,
                    )
                {
                    release_queue(&subscriber, &mut target);
                    return Err(BridgeError::ResyncRequired {
                        oldest_seq: oldest.unwrap_or(0),
                        newest_seq: newest,
                    });
                }
                if !reserve_atomic(&subscriber.host_queued, *bytes, MAX_QUEUED_BYTES_PER_HOST) {
                    subscriber
                        .connection_queued
                        .fetch_sub(*bytes, Ordering::AcqRel);
                    release_queue(&subscriber, &mut target);
                    return Err(BridgeError::ResyncRequired {
                        oldest_seq: oldest.unwrap_or(0),
                        newest_seq: newest,
                    });
                }
                target.queued_bytes += *bytes;
                target.queue.push_back((event.clone(), *bytes));
            }
        }
        state.subscribers.push(Arc::downgrade(&subscriber));
        Ok(Subscription { inner: subscriber })
    }

    pub fn range(&self) -> (u64, u64) {
        let Ok(state) = self.state.lock() else {
            return (0, 0);
        };
        (
            state.retained.front().map_or(0, |(event, _)| event.seq),
            state.retained.back().map_or(0, |(event, _)| event.seq),
        )
    }

    pub fn current_seq(&self) -> u64 {
        self.state
            .lock()
            .ok()
            .map_or(0, |state| state.next_seq.saturating_sub(1))
    }

    pub fn retained_state_bytes(&self) -> usize {
        let retained = self
            .state
            .lock()
            .ok()
            .map_or(0, |state| state.retained_bytes);
        retained.saturating_add(self.host_queued.load(Ordering::Acquire))
    }
}

pub struct Subscription {
    inner: Arc<Subscriber>,
}

impl Subscription {
    pub fn try_next(&self) -> BridgeResult<Option<BridgeEvent>> {
        let mut state = self.inner.state.lock().map_err(|_| BridgeError::Closed)?;
        pop_next(&self.inner, &mut state)
    }

    pub fn recv_timeout(&self, timeout: Duration) -> BridgeResult<Option<BridgeEvent>> {
        let mut state = self.inner.state.lock().map_err(|_| BridgeError::Closed)?;
        if state.queue.is_empty() && state.resync.is_none() && !state.closed {
            let (next, _) = self
                .inner
                .wake
                .wait_timeout(state, timeout)
                .map_err(|_| BridgeError::Closed)?;
            state = next;
        }
        pop_next(&self.inner, &mut state)
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.closed = true;
            release_queue(&self.inner, &mut state);
            self.inner.wake.notify_all();
        }
    }
}

fn pop_next(
    subscriber: &Subscriber,
    state: &mut SubscriberState,
) -> BridgeResult<Option<BridgeEvent>> {
    if let Some((oldest_seq, newest_seq)) = state.resync.take() {
        state.closed = true;
        return Err(BridgeError::ResyncRequired {
            oldest_seq,
            newest_seq,
        });
    }
    if state.closed {
        return Err(BridgeError::Closed);
    }
    let event = state.queue.pop_front();
    if let Some((_, bytes)) = &event {
        state.queued_bytes = state.queued_bytes.saturating_sub(*bytes);
        subscriber
            .connection_queued
            .fetch_sub(*bytes, Ordering::AcqRel);
        subscriber.host_queued.fetch_sub(*bytes, Ordering::AcqRel);
    }
    Ok(event.map(|(event, _)| event))
}

fn mark_resync(
    subscriber: &Subscriber,
    state: &mut SubscriberState,
    oldest_seq: u64,
    newest_seq: u64,
) {
    release_queue(subscriber, state);
    state.resync = Some((oldest_seq, newest_seq));
    subscriber.wake.notify_one();
}

fn release_queue(subscriber: &Subscriber, state: &mut SubscriberState) {
    let bytes = state.queued_bytes;
    state.queue.clear();
    state.queued_bytes = 0;
    if bytes > 0 {
        subscriber
            .connection_queued
            .fetch_sub(bytes, Ordering::AcqRel);
        subscriber.host_queued.fetch_sub(bytes, Ordering::AcqRel);
    }
}

fn reserve_atomic(counter: &AtomicUsize, bytes: usize, maximum: usize) -> bool {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current
                .checked_add(bytes)
                .filter(|updated| *updated <= maximum)
        })
        .is_ok()
}

#[cfg(test)]
mod tests {
    use spectrum_revisions::{RevisionId, SessionId, TrackId};

    use super::*;
    use crate::CursorTransition;

    fn event(index: u64, padding: usize) -> BridgeEventKind {
        BridgeEventKind::InteractionBegan {
            interaction_id: format!("{index}-{}", "x".repeat(padding)),
            interaction_kind: "benchmark".into(),
        }
    }

    #[test]
    fn replay_is_strictly_ordered_and_gap_requires_resync() {
        let log = EventLog::new();
        for value in 0..1_100 {
            log.append(event(value, 0)).unwrap();
        }
        assert!(matches!(
            log.subscribe(0),
            Err(BridgeError::ResyncRequired { .. })
        ));
        let (_, newest) = log.range();
        let replay_from = newest.saturating_sub(MAX_SUBSCRIBER_EVENTS as u64);
        let subscription = log.subscribe(replay_from).unwrap();
        let mut seen = Vec::new();
        while let Some(event) = subscription.try_next().unwrap() {
            seen.push(event.seq);
        }
        assert_eq!(seen.first(), Some(&(replay_from + 1)));
        assert_eq!(seen.last(), Some(&newest));
        assert!(seen.windows(2).all(|pair| pair[1] == pair[0] + 1));
    }

    #[test]
    fn slow_subscriber_is_resynced_without_harming_fast_one() {
        let log = EventLog::new();
        let slow = log.subscribe(0).unwrap();
        let fast = log.subscribe(0).unwrap();
        for value in 0..300 {
            log.append(event(value, 0)).unwrap();
            let event = fast.try_next().unwrap().unwrap();
            assert_eq!(event.seq, value + 1);
        }
        assert!(matches!(
            slow.try_next(),
            Err(BridgeError::ResyncRequired { .. })
        ));
    }

    #[test]
    fn invalid_event_does_not_consume_a_sequence() {
        let log = EventLog::new();
        assert!(
            log.append(BridgeEventKind::CursorMoved {
                session_id: SessionId::new(),
                cursors: Vec::new(),
            })
            .is_err()
        );
        let seq = log
            .append(BridgeEventKind::CursorMoved {
                session_id: SessionId::new(),
                cursors: vec![CursorTransition {
                    track_id: TrackId::new(),
                    from_revision_id: RevisionId::new(),
                    to_revision_id: RevisionId::new(),
                }],
            })
            .unwrap();
        assert_eq!(seq, 1);
    }

    #[test]
    fn connection_and_host_aggregate_queue_budgets_force_resync() {
        let log = EventLog::new();
        let shared_connection = Arc::new(AtomicUsize::new(0));
        let same_connection = (0..crate::MAX_SUBSCRIPTIONS_PER_CONNECTION)
            .map(|_| {
                log.subscribe_with_budget(0, Arc::clone(&shared_connection))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for index in 0..300 {
            log.append(event(index, 3_900)).unwrap();
        }
        assert!(same_connection.iter().any(|subscription| matches!(
            subscription.try_next(),
            Err(BridgeError::ResyncRequired { .. })
        )));
        assert!(shared_connection.load(Ordering::Acquire) <= MAX_QUEUED_BYTES_PER_CONNECTION);

        let log = EventLog::new();
        let many_connections = (0..40)
            .map(|_| {
                log.subscribe_with_budget(0, Arc::new(AtomicUsize::new(0)))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for index in 0..300 {
            log.append(event(index, 3_900)).unwrap();
        }
        assert!(many_connections.iter().any(|subscription| matches!(
            subscription.try_next(),
            Err(BridgeError::ResyncRequired { .. })
        )));
        assert!(log.host_queued.load(Ordering::Acquire) <= MAX_QUEUED_BYTES_PER_HOST);
    }
}
