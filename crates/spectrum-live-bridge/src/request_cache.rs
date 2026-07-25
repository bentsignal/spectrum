use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    BridgeError, BridgeResult, REQUEST_CACHE_MAX_ENTRIES, REQUEST_CACHE_TTL, RequestId,
    ResponseEnvelope,
};

#[derive(Clone, Debug, PartialEq)]
pub struct CachedResponse {
    pub response: ResponseEnvelope,
    pub inserted: Instant,
}

struct Entry {
    fingerprint: [u8; 32],
    response: ResponseEnvelope,
    inserted: Instant,
}

pub struct RequestCache {
    entries: HashMap<RequestId, Entry>,
    order: VecDeque<RequestId>,
    maximum: usize,
    ttl: Duration,
}

impl Default for RequestCache {
    fn default() -> Self {
        Self::new(REQUEST_CACHE_MAX_ENTRIES, REQUEST_CACHE_TTL)
    }
}

impl RequestCache {
    pub fn new(maximum: usize, ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            maximum,
            ttl,
        }
    }

    pub fn lookup<T: Serialize>(
        &mut self,
        request_id: RequestId,
        request: &T,
    ) -> BridgeResult<Option<CachedResponse>> {
        self.prune();
        let fingerprint = fingerprint(request)?;
        let Some(entry) = self.entries.get(&request_id) else {
            return Ok(None);
        };
        if entry.fingerprint != fingerprint {
            return Err(BridgeError::Protocol(
                "request id was reused with different content".into(),
            ));
        }
        Ok(Some(CachedResponse {
            response: entry.response.clone(),
            inserted: entry.inserted,
        }))
    }

    pub fn insert<T: Serialize>(
        &mut self,
        request_id: RequestId,
        request: &T,
        response: ResponseEnvelope,
    ) -> BridgeResult<()> {
        self.prune();
        let fingerprint = fingerprint(request)?;
        if let Some(existing) = self.entries.get(&request_id) {
            if existing.fingerprint != fingerprint {
                return Err(BridgeError::Protocol(
                    "request id was reused with different content".into(),
                ));
            }
            return Ok(());
        }
        self.entries.insert(
            request_id,
            Entry {
                fingerprint,
                response,
                inserted: Instant::now(),
            },
        );
        self.order.push_back(request_id);
        while self.entries.len() > self.maximum {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        Ok(())
    }

    fn prune(&mut self) {
        let now = Instant::now();
        while let Some(request_id) = self.order.front().copied() {
            let expired = self
                .entries
                .get(&request_id)
                .is_none_or(|entry| now.duration_since(entry.inserted) >= self.ttl);
            if !expired {
                break;
            }
            self.order.pop_front();
            self.entries.remove(&request_id);
        }
    }
}

fn fingerprint<T: Serialize>(value: &T) -> BridgeResult<[u8; 32]> {
    Ok(Sha256::digest(serde_json::to_vec(value)?).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ResponseBody, ResponseEnvelope};

    #[test]
    fn exact_retry_returns_result_and_changed_retry_fails() {
        let request_id = RequestId::new();
        let response = ResponseEnvelope {
            request_id,
            body: ResponseBody::Deferred,
        };
        let mut cache = RequestCache::default();
        cache.insert(request_id, &"same", response.clone()).unwrap();
        assert_eq!(
            cache.lookup(request_id, &"same").unwrap().unwrap().response,
            response
        );
        assert!(cache.lookup(request_id, &"different").is_err());
    }
}
