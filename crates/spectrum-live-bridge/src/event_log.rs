use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, Weak},
};

use serde_json::Value;

use crate::{
    BridgeError, BridgeEvent, BridgeResult, EVENT_LOG_MAX_BYTES, EVENT_LOG_MAX_EVENTS,
    MAX_SUBSCRIBER_BYTES, MAX_SUBSCRIBER_EVENTS,
};

#[derive(Default)]
struct SubscriberState {
    queue: VecDeque<BridgeEvent>,
    queued_bytes: usize,
    resync: Option<(u64, u64)>,
    closed: bool,
}

struct LogState {
    next_seq: u64,
    retained: VecDeque<(BridgeEvent, usize)>,
    retained_bytes: usize,
    subscribers: Vec<Weak<Mutex<SubscriberState>>>,
}

pub struct EventLog {
    state: Mutex<LogState>,
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
        }
    }

    pub fn append(
        &self,
        family: impl Into<String>,
        version: u32,
        payload: Value,
    ) -> BridgeResult<u64> {
        let mut state = self.state.lock().map_err(|_| BridgeError::Closed)?;
        let seq = state.next_seq;
        state.next_seq = state
            .next_seq
            .checked_add(1)
            .ok_or_else(|| BridgeError::Protocol("event sequence exhausted".into()))?;
        let event = BridgeEvent {
            seq,
            family: family.into(),
            version,
            payload,
        };
        event.validate()?;
        let encoded = event.encoded_len()?;
        if encoded > EVENT_LOG_MAX_BYTES {
            return Err(BridgeError::Limit("single event exceeds event log".into()));
        }
        state.retained.push_back((event.clone(), encoded));
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
            let Ok(mut subscriber) = subscriber.lock() else {
                return false;
            };
            if subscriber.closed || subscriber.resync.is_some() {
                return true;
            }
            if subscriber.queue.len() >= MAX_SUBSCRIBER_EVENTS
                || subscriber.queued_bytes.saturating_add(encoded) > MAX_SUBSCRIBER_BYTES
            {
                subscriber.queue.clear();
                subscriber.queued_bytes = 0;
                subscriber.resync = Some((oldest, newest));
            } else {
                subscriber.queued_bytes += encoded;
                subscriber.queue.push_back(event.clone());
            }
            true
        });
        Ok(seq)
    }

    pub fn subscribe(&self, after_seq: u64) -> BridgeResult<Subscription> {
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
        let subscriber = Arc::new(Mutex::new(SubscriberState::default()));
        {
            let mut target = subscriber.lock().map_err(|_| BridgeError::Closed)?;
            for (event, bytes) in state
                .retained
                .iter()
                .filter(|(event, _)| event.seq > after_seq)
            {
                if target.queue.len() >= MAX_SUBSCRIBER_EVENTS
                    || target.queued_bytes.saturating_add(*bytes) > MAX_SUBSCRIBER_BYTES
                {
                    return Err(BridgeError::ResyncRequired {
                        oldest_seq: oldest.unwrap_or(0),
                        newest_seq: newest,
                    });
                }
                target.queued_bytes += *bytes;
                target.queue.push_back(event.clone());
            }
        }
        state.subscribers.push(Arc::downgrade(&subscriber));
        Ok(Subscription { state: subscriber })
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
}

pub struct Subscription {
    state: Arc<Mutex<SubscriberState>>,
}

impl Subscription {
    pub fn try_next(&self) -> BridgeResult<Option<BridgeEvent>> {
        let mut state = self.state.lock().map_err(|_| BridgeError::Closed)?;
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
        if let Some(event) = &event {
            state.queued_bytes = state
                .queued_bytes
                .saturating_sub(event.encoded_len().unwrap_or(0));
        }
        Ok(event)
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            state.queue.clear();
            state.queued_bytes = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_is_strictly_ordered_and_gap_requires_resync() {
        let log = EventLog::new();
        for value in 0..1_100 {
            log.append("test", 1, serde_json::json!({"value": value}))
                .unwrap();
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
            log.append("test", 1, serde_json::json!({"value": value}))
                .unwrap();
            let event = fast.try_next().unwrap().unwrap();
            assert_eq!(event.seq, value + 1);
        }
        assert!(matches!(
            slow.try_next(),
            Err(BridgeError::ResyncRequired { .. })
        ));
    }
}
