use std::time::Duration;

use serde_json::Value;

use crate::{BridgeError, BridgeResult};

pub const PROTOCOL_FAMILY: &str = "spectrum.live_bridge";
pub const DISCOVERY_FAMILY: &str = "spectrum.live_bridge.discovery";
pub const PROTOCOL_VERSION: u16 = 2;
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_ACTION_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_STRING_BYTES: usize = 4_096;
pub const MAX_ERROR_BYTES: usize = 8_192;
pub const MAX_JSON_DEPTH: usize = 64;
pub const MAX_BATCH_ITEMS: usize = 128;
pub const MAX_JSON_NODES: usize = 65_536;
pub const MAX_CURSOR_EXPECTATIONS: usize = 64;
pub const MAX_AUTHENTICATED_CONNECTIONS: usize = 8;
pub const MAX_SUBSCRIPTIONS_PER_CONNECTION: usize = 8;
pub const MAX_IN_FLIGHT_PER_CONNECTION: usize = 32;
pub const MAX_DEFERRED_PER_CONNECTION: usize = 16;
pub const MAX_QUEUED_BYTES_PER_CONNECTION: usize = 8 * 1024 * 1024;
pub const MAX_QUEUED_BYTES_PER_HOST: usize = 32 * 1024 * 1024;
pub const MAX_SUBSCRIBER_EVENTS: usize = 256;
pub const MAX_SUBSCRIBER_BYTES: usize = 2 * 1024 * 1024;
pub const EVENT_LOG_MAX_EVENTS: usize = 1_024;
pub const EVENT_LOG_MAX_BYTES: usize = 8 * 1024 * 1024;
pub const REQUEST_CACHE_MAX_ENTRIES: usize = 1_024;
pub const REQUEST_CACHE_MAX_BYTES: usize = 8 * 1024 * 1024;
pub const REQUEST_TOMBSTONE_MAX_ENTRIES: usize = 8_192;
pub const REQUEST_TOMBSTONE_BLOOM_WORDS: usize = 2_048;
pub const MAX_INGRESS_BYTES_PER_SECOND: usize = 8 * 1024 * 1024;
pub const MAX_INGRESS_BURST_BYTES: usize = 16 * 1024 * 1024;
pub const AUTH_DEADLINE: Duration = Duration::from_secs(2);
pub const DISCOVERY_REFRESH: Duration = Duration::from_secs(5);
pub const DISCOVERY_EXPIRY: Duration = Duration::from_secs(15);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
pub const REQUEST_CACHE_TTL: Duration = Duration::from_secs(10 * 60);

pub fn validate_json_limits(value: &Value) -> BridgeResult<()> {
    fn walk(value: &Value, depth: usize, nodes: &mut usize) -> BridgeResult<()> {
        *nodes = nodes
            .checked_add(1)
            .ok_or_else(|| BridgeError::Limit("JSON node count overflow".into()))?;
        if *nodes > MAX_JSON_NODES {
            return Err(BridgeError::Limit(format!(
                "JSON exceeds {MAX_JSON_NODES} aggregate values"
            )));
        }
        if depth > MAX_JSON_DEPTH {
            return Err(BridgeError::Limit("JSON nesting exceeds 64 levels".into()));
        }
        match value {
            Value::String(string) if string.len() > MAX_STRING_BYTES => {
                Err(BridgeError::Limit("JSON string exceeds 4096 bytes".into()))
            }
            Value::Array(items) => {
                if items.len() > MAX_BATCH_ITEMS {
                    return Err(BridgeError::Limit("JSON array exceeds 128 items".into()));
                }
                for item in items {
                    walk(item, depth + 1, nodes)?;
                }
                Ok(())
            }
            Value::Object(fields) => {
                for (key, value) in fields {
                    if key.len() > MAX_STRING_BYTES {
                        return Err(BridgeError::Limit("JSON key exceeds 4096 bytes".into()));
                    }
                    walk(value, depth + 1, nodes)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    let mut nodes = 0;
    walk(value, 0, &mut nodes)
}
