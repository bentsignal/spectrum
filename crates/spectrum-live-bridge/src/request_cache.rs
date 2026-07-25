use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    BridgeError, BridgeResult, REQUEST_CACHE_MAX_BYTES, REQUEST_CACHE_MAX_ENTRIES,
    REQUEST_CACHE_TTL, REQUEST_TOMBSTONE_BLOOM_WORDS, REQUEST_TOMBSTONE_MAX_ENTRIES, RequestId,
    ResponseEnvelope,
};

#[derive(Clone, Debug, PartialEq)]
pub struct CachedResponse {
    pub response: ResponseEnvelope,
    pub inserted: Instant,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RequestLookup {
    Miss,
    Cached(CachedResponse),
    OutcomeUnknown,
}

struct Entry {
    fingerprint: [u8; 32],
    response: ResponseEnvelope,
    inserted: Instant,
    encoded_bytes: usize,
}

/// A per-binding, once-only RequestId ledger.
///
/// Exact results are retained for the configured entry/TTL window. Once an
/// exact result expires, a bounded exact tombstone and then a fixed-size Bloom
/// filter preserve the safety invariant: an old RequestId can become
/// `outcome_unknown`, but it can never become a fresh mutation again during
/// the binding lifetime.
pub struct RequestCache {
    entries: HashMap<RequestId, Entry>,
    order: VecDeque<RequestId>,
    pending: HashMap<RequestId, [u8; 32]>,
    tombstones: HashMap<RequestId, [u8; 32]>,
    tombstone_order: VecDeque<RequestId>,
    expired_bloom: Vec<u64>,
    maximum: usize,
    maximum_bytes: usize,
    ttl: Duration,
    retained_bytes: usize,
}

impl Default for RequestCache {
    fn default() -> Self {
        Self::with_limits(
            REQUEST_CACHE_MAX_ENTRIES,
            REQUEST_CACHE_TTL,
            REQUEST_CACHE_MAX_BYTES,
        )
    }
}

impl RequestCache {
    pub fn new(maximum: usize, ttl: Duration) -> Self {
        Self::with_limits(maximum, ttl, REQUEST_CACHE_MAX_BYTES)
    }

    pub fn with_limits(maximum: usize, ttl: Duration, maximum_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            pending: HashMap::new(),
            tombstones: HashMap::new(),
            tombstone_order: VecDeque::new(),
            expired_bloom: vec![0; REQUEST_TOMBSTONE_BLOOM_WORDS],
            maximum,
            maximum_bytes,
            ttl,
            retained_bytes: 0,
        }
    }

    pub fn lookup<T: Serialize>(
        &mut self,
        request_id: RequestId,
        request: &T,
    ) -> BridgeResult<RequestLookup> {
        self.prune();
        let fingerprint = fingerprint(request)?;
        if let Some(entry) = self.entries.get(&request_id) {
            if entry.fingerprint != fingerprint {
                return Err(reused_request_id());
            }
            return Ok(RequestLookup::Cached(CachedResponse {
                response: entry.response.clone(),
                inserted: entry.inserted,
            }));
        }
        if let Some(pending) = self.pending.get(&request_id) {
            if pending != &fingerprint {
                return Err(reused_request_id());
            }
            return Ok(RequestLookup::OutcomeUnknown);
        }
        if let Some(expired) = self.tombstones.get(&request_id) {
            if expired != &fingerprint {
                return Err(reused_request_id());
            }
            return Ok(RequestLookup::OutcomeUnknown);
        }
        if self.bloom_contains(request_id) {
            return Ok(RequestLookup::OutcomeUnknown);
        }
        Ok(RequestLookup::Miss)
    }

    /// Reserve a fresh id before the host is allowed to mutate.
    ///
    /// A reservation must be canceled only while mutation is still proven not
    /// to have started. After authority passes to the host, failures must call
    /// `mark_outcome_unknown` instead.
    pub fn reserve<T: Serialize>(
        &mut self,
        request_id: RequestId,
        request: &T,
    ) -> BridgeResult<()> {
        self.prune();
        if !matches!(self.lookup(request_id, request)?, RequestLookup::Miss) {
            return Err(BridgeError::Protocol(
                "request id is already reserved or completed".into(),
            ));
        }
        self.pending.insert(request_id, fingerprint(request)?);
        Ok(())
    }

    pub fn cancel_reservation(&mut self, request_id: RequestId) {
        self.pending.remove(&request_id);
    }

    pub fn mark_outcome_unknown(&mut self, request_id: RequestId) -> BridgeResult<()> {
        let fingerprint = self
            .pending
            .remove(&request_id)
            .ok_or_else(|| BridgeError::Protocol("request id has no pending reservation".into()))?;
        self.insert_tombstone(request_id, fingerprint);
        Ok(())
    }

    pub fn complete<T: Serialize>(
        &mut self,
        request_id: RequestId,
        request: &T,
        response: ResponseEnvelope,
    ) -> BridgeResult<()> {
        self.prune();
        let fingerprint = fingerprint(request)?;
        let pending = self
            .pending
            .get(&request_id)
            .ok_or_else(|| BridgeError::Protocol("request id has no pending reservation".into()))?;
        if pending != &fingerprint {
            return Err(reused_request_id());
        }
        let encoded_bytes = serde_json::to_vec(&response)?.len() + 64;
        if encoded_bytes > self.maximum_bytes {
            return Err(BridgeError::Limit(
                "single request result exceeds cache byte budget".into(),
            ));
        }
        while self.entries.len() >= self.maximum
            || self.retained_bytes.saturating_add(encoded_bytes) > self.maximum_bytes
        {
            if self.entries.is_empty() {
                return Err(BridgeError::Limit(
                    "request cache cannot retain a result within its configured budget".into(),
                ));
            }
            self.expire_oldest();
        }
        self.pending.remove(&request_id);
        self.retained_bytes = self.retained_bytes.saturating_add(encoded_bytes);
        self.entries.insert(
            request_id,
            Entry {
                fingerprint,
                response,
                inserted: Instant::now(),
                encoded_bytes,
            },
        );
        self.order.push_back(request_id);
        Ok(())
    }

    pub fn insert<T: Serialize>(
        &mut self,
        request_id: RequestId,
        request: &T,
        response: ResponseEnvelope,
    ) -> BridgeResult<()> {
        match self.lookup(request_id, request)? {
            RequestLookup::Cached(_) => return Ok(()),
            RequestLookup::OutcomeUnknown => {
                return Err(BridgeError::Protocol(
                    "cannot attach a new result to an expired request id".into(),
                ));
            }
            RequestLookup::Miss => {}
        }
        self.reserve(request_id, request)?;
        if let Err(error) = self.complete(request_id, request, response) {
            self.cancel_reservation(request_id);
            return Err(error);
        }
        Ok(())
    }

    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
            .saturating_add(self.pending.len().saturating_mul(64))
            .saturating_add(self.tombstones.len().saturating_mul(64))
            .saturating_add(self.expired_bloom.len() * std::mem::size_of::<u64>())
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
            self.expire_oldest();
        }
    }

    fn expire_oldest(&mut self) {
        let Some(request_id) = self.order.pop_front() else {
            return;
        };
        let Some(entry) = self.entries.remove(&request_id) else {
            return;
        };
        self.retained_bytes = self.retained_bytes.saturating_sub(entry.encoded_bytes);
        self.insert_tombstone(request_id, entry.fingerprint);
    }

    fn insert_tombstone(&mut self, request_id: RequestId, fingerprint: [u8; 32]) {
        self.tombstones.insert(request_id, fingerprint);
        self.tombstone_order.push_back(request_id);
        while self.tombstones.len() > REQUEST_TOMBSTONE_MAX_ENTRIES {
            let Some(oldest) = self.tombstone_order.pop_front() else {
                break;
            };
            if self.tombstones.remove(&oldest).is_some() {
                self.bloom_insert(oldest);
            }
        }
    }

    fn bloom_insert(&mut self, request_id: RequestId) {
        for bit in bloom_bits(request_id, self.expired_bloom.len() * 64) {
            self.expired_bloom[bit / 64] |= 1_u64 << (bit % 64);
        }
    }

    fn bloom_contains(&self, request_id: RequestId) -> bool {
        bloom_bits(request_id, self.expired_bloom.len() * 64)
            .into_iter()
            .all(|bit| self.expired_bloom[bit / 64] & (1_u64 << (bit % 64)) != 0)
    }
}

fn reused_request_id() -> BridgeError {
    BridgeError::Protocol("request id was reused with different content".into())
}

fn fingerprint<T: Serialize>(value: &T) -> BridgeResult<[u8; 32]> {
    Ok(Sha256::digest(serde_json::to_vec(value)?).into())
}

fn bloom_bits(request_id: RequestId, bit_count: usize) -> [usize; 4] {
    let digest = Sha256::digest(request_id.as_bytes());
    std::array::from_fn(|index| {
        let offset = index * 4;
        u32::from_be_bytes(digest[offset..offset + 4].try_into().expect("fixed digest")) as usize
            % bit_count
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ResponseBody, ResponseEnvelope};

    #[test]
    fn exact_retry_returns_result_changed_retry_fails_and_expiry_is_unknown() {
        let request_id = RequestId::new();
        let response = ResponseEnvelope {
            request_id,
            body: ResponseBody::Deferred,
        };
        let mut cache = RequestCache::new(1, Duration::ZERO);
        cache.insert(request_id, &"same", response.clone()).unwrap();
        assert_eq!(
            cache.lookup(request_id, &"same").unwrap(),
            RequestLookup::OutcomeUnknown
        );
        assert!(cache.lookup(request_id, &"different").is_err());
    }

    #[test]
    fn capacity_eviction_never_turns_an_old_id_into_a_miss() {
        let first = RequestId::new();
        let second = RequestId::new();
        let mut cache = RequestCache::new(1, Duration::from_secs(60));
        for request_id in [first, second] {
            cache
                .insert(
                    request_id,
                    &request_id.to_string(),
                    ResponseEnvelope {
                        request_id,
                        body: ResponseBody::Deferred,
                    },
                )
                .unwrap();
        }
        assert_eq!(
            cache.lookup(first, &first.to_string()).unwrap(),
            RequestLookup::OutcomeUnknown
        );
        assert!(matches!(
            cache.lookup(second, &second.to_string()).unwrap(),
            RequestLookup::Cached(_)
        ));
    }

    #[test]
    fn byte_eviction_becomes_outcome_unknown_and_pending_failure_is_tombstoned() {
        let first = RequestId::new();
        let second = RequestId::new();
        let response = |request_id, padding: usize| ResponseEnvelope {
            request_id,
            body: ResponseBody::Applied {
                result: serde_json::json!({"padding": "x".repeat(padding)}),
                cursors: vec![crate::ExpectedCursor {
                    track_id: spectrum_revisions::TrackId::new(),
                    revision_id: spectrum_revisions::RevisionId::new(),
                }],
            },
        };
        let mut cache = RequestCache::with_limits(8, Duration::from_secs(60), 1_024);
        cache.insert(first, &"first", response(first, 600)).unwrap();
        cache
            .insert(second, &"second", response(second, 600))
            .unwrap();
        assert_eq!(
            cache.lookup(first, &"first").unwrap(),
            RequestLookup::OutcomeUnknown
        );
        assert!(matches!(
            cache.lookup(second, &"second").unwrap(),
            RequestLookup::Cached(_)
        ));
        assert!(
            cache.retained_bytes()
                <= 1_024 + 64 + REQUEST_TOMBSTONE_BLOOM_WORDS * std::mem::size_of::<u64>()
        );

        let pending = RequestId::new();
        cache.reserve(pending, &"pending").unwrap();
        cache.mark_outcome_unknown(pending).unwrap();
        assert_eq!(
            cache.lookup(pending, &"pending").unwrap(),
            RequestLookup::OutcomeUnknown
        );
        assert!(cache.lookup(pending, &"changed").is_err());
    }
}
