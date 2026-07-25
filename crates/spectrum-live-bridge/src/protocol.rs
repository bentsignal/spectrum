use std::{collections::BTreeMap, fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use spectrum_revisions::{ProjectId, RevisionId, SessionId, TrackId};
use uuid::Uuid;

use crate::{
    BridgeError, BridgeResult, MAX_ACTION_BYTES, MAX_CURSOR_EXPECTATIONS, MAX_ERROR_BYTES,
    MAX_STRING_BYTES, PROTOCOL_FAMILY, PROTOCOL_VERSION, validate_json_limits,
};

macro_rules! bridge_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(Uuid::from_bytes(bytes))
            }

            pub fn as_bytes(&self) -> &[u8; 16] {
                self.0.as_bytes()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self)
            }
        }
    };
}

bridge_id!(InstanceId);
bridge_id!(BindingId);
bridge_id!(RequestId);
bridge_id!(CapabilityId);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionPolicy {
    Immediate,
    Deferred,
    RequireUserConfirmation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedCursor {
    pub track_id: TrackId,
    pub revision_id: RevisionId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionEnvelope {
    pub family: String,
    pub version: u32,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub action: Value,
}

impl ActionEnvelope {
    pub fn validate(&self) -> BridgeResult<()> {
        bounded("action family", &self.family, MAX_STRING_BYTES)?;
        if self.family.is_empty() {
            return Err(BridgeError::Protocol("action family is empty".into()));
        }
        if self.capabilities.len() > MAX_CURSOR_EXPECTATIONS {
            return Err(BridgeError::Limit("too many action capabilities".into()));
        }
        let encoded = serde_json::to_vec(&self.action)?;
        if encoded.len() > MAX_ACTION_BYTES {
            return Err(BridgeError::Limit(
                "opaque application action exceeds 4 MiB".into(),
            ));
        }
        validate_json_limits_except_action(&self.action)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestEnvelope {
    pub protocol: String,
    pub version: u16,
    pub request_id: RequestId,
    pub binding_id: BindingId,
    pub binding_epoch: u64,
    pub project_id: ProjectId,
    pub application: String,
    pub session_id: SessionId,
    pub expected_cursors: Vec<ExpectedCursor>,
    pub actor_label: String,
    pub interaction: InteractionPolicy,
    pub action: ActionEnvelope,
}

impl RequestEnvelope {
    pub fn validate(&self) -> BridgeResult<()> {
        if self.protocol != PROTOCOL_FAMILY || self.version != PROTOCOL_VERSION {
            return Err(BridgeError::Protocol(
                "unsupported protocol family or version".into(),
            ));
        }
        bounded("application", &self.application, MAX_STRING_BYTES)?;
        bounded("actor label", &self.actor_label, MAX_STRING_BYTES)?;
        if self.application.is_empty() || self.actor_label.is_empty() {
            return Err(BridgeError::Protocol(
                "application and actor label must be nonempty".into(),
            ));
        }
        if self.expected_cursors.len() > MAX_CURSOR_EXPECTATIONS {
            return Err(BridgeError::Limit("more than 64 expected cursors".into()));
        }
        for pair in self.expected_cursors.windows(2) {
            if pair[0].track_id.as_bytes() >= pair[1].track_id.as_bytes() {
                return Err(BridgeError::Protocol(
                    "expected cursors must be sorted and unique".into(),
                ));
            }
        }
        self.action.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponseBody {
    Applied {
        result: Value,
        cursors: Vec<ExpectedCursor>,
    },
    Deferred,
    Refused {
        reason: String,
    },
    Conflict {
        current: Vec<ExpectedCursor>,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseEnvelope {
    pub request_id: RequestId,
    pub body: ResponseBody,
}

impl ResponseEnvelope {
    pub fn validate(&self) -> BridgeResult<()> {
        if let ResponseBody::Error { code, message } = &self.body {
            bounded("error code", code, MAX_STRING_BYTES)?;
            bounded("error message", message, MAX_ERROR_BYTES)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateSnapshot {
    pub project_id: ProjectId,
    pub binding_id: BindingId,
    pub binding_epoch: u64,
    pub cursors: Vec<ExpectedCursor>,
    #[serde(default)]
    pub application_state: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeEvent {
    pub seq: u64,
    pub family: String,
    pub version: u32,
    pub payload: Value,
}

impl BridgeEvent {
    pub fn encoded_len(&self) -> BridgeResult<usize> {
        Ok(serde_json::to_vec(self)?.len())
    }

    pub fn validate(&self) -> BridgeResult<()> {
        bounded("event family", &self.family, MAX_STRING_BYTES)?;
        validate_json_limits(&self.payload)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Request(Box<RequestEnvelope>),
    Subscribe { after_seq: u64 },
    Ping { nonce: u64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Response(ResponseEnvelope),
    Snapshot(StateSnapshot),
    Event(BridgeEvent),
    ResyncRequired { oldest_seq: u64, newest_seq: u64 },
    Pong { nonce: u64 },
}

fn bounded(name: &str, value: &str, maximum: usize) -> BridgeResult<()> {
    if value.len() > maximum {
        return Err(BridgeError::Limit(format!(
            "{name} exceeds {maximum} bytes"
        )));
    }
    Ok(())
}

fn validate_json_limits_except_action(value: &Value) -> BridgeResult<()> {
    fn walk(value: &Value, depth: usize) -> BridgeResult<()> {
        if depth > crate::MAX_JSON_DEPTH {
            return Err(BridgeError::Limit("JSON nesting exceeds 64 levels".into()));
        }
        match value {
            Value::Array(items) => {
                if items.len() > crate::MAX_BATCH_ITEMS {
                    return Err(BridgeError::Limit("JSON array exceeds 128 items".into()));
                }
                for item in items {
                    walk(item, depth + 1)?;
                }
            }
            Value::Object(fields) => {
                for (key, value) in fields {
                    bounded("action key", key, MAX_STRING_BYTES)?;
                    walk(value, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    walk(value, 0)
}
