use std::{collections::BTreeMap, fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use spectrum_revisions::{ChangeSetId, ProjectId, RevisionId, SessionId, TrackId};
use uuid::Uuid;

use crate::{
    BridgeError, BridgeResult, MAX_ACTION_BYTES, MAX_CURSOR_EXPECTATIONS, MAX_ERROR_BYTES,
    MAX_FRAME_BYTES, MAX_STRING_BYTES, PROTOCOL_FAMILY, PROTOCOL_VERSION, validate_json_limits,
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
        for capability in &self.capabilities {
            bounded("action capability", capability, MAX_STRING_BYTES)?;
        }
        if self.capabilities.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(BridgeError::Protocol(
                "action capabilities must be sorted and unique".into(),
            ));
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
        if self.expected_cursors.is_empty() {
            return Err(BridgeError::Protocol(
                "expected cursors must be nonempty".into(),
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
    OutcomeUnknown,
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
        match &self.body {
            ResponseBody::Applied { result, cursors } => {
                validate_json_limits(result)?;
                if serde_json::to_vec(result)?.len() > MAX_ACTION_BYTES {
                    return Err(BridgeError::Limit(
                        "application result exceeds 4 MiB".into(),
                    ));
                }
                validate_cursors("applied cursors", cursors, true)?;
            }
            ResponseBody::Refused { reason } => {
                bounded("refusal reason", reason, MAX_ERROR_BYTES)?;
                if reason.is_empty() {
                    return Err(BridgeError::Protocol("refusal reason is empty".into()));
                }
            }
            ResponseBody::Conflict { current } => {
                validate_cursors("conflict cursors", current, true)?;
            }
            ResponseBody::Error { code, message } => {
                bounded("error code", code, MAX_STRING_BYTES)?;
                bounded("error message", message, MAX_ERROR_BYTES)?;
                if code.is_empty() || message.is_empty() {
                    return Err(BridgeError::Protocol(
                        "error code and message must be nonempty".into(),
                    ));
                }
            }
            ResponseBody::Deferred | ResponseBody::OutcomeUnknown => {}
        }
        if serde_json::to_vec(self)?.len() > MAX_FRAME_BYTES {
            return Err(BridgeError::Limit("response frame exceeds 8 MiB".into()));
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
    pub current_event_seq: u64,
    #[serde(default)]
    pub application_protocols: BTreeMap<String, ProtocolRange>,
    #[serde(default)]
    pub application_state: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolRange {
    pub minimum: u32,
    pub maximum: u32,
}

impl StateSnapshot {
    pub fn validate(&self) -> BridgeResult<()> {
        validate_cursors("snapshot cursors", &self.cursors, false)?;
        for (family, range) in &self.application_protocols {
            bounded("application protocol family", family, MAX_STRING_BYTES)?;
            if family.is_empty() || range.minimum == 0 || range.minimum > range.maximum {
                return Err(BridgeError::Protocol(
                    "invalid application protocol range".into(),
                ));
            }
        }
        for (family, state) in &self.application_state {
            bounded("application state family", family, MAX_STRING_BYTES)?;
            if family.is_empty() {
                return Err(BridgeError::Protocol(
                    "application state family is empty".into(),
                ));
            }
            validate_json_limits(state)?;
        }
        if serde_json::to_vec(self)?.len() > MAX_FRAME_BYTES {
            return Err(BridgeError::Limit("snapshot frame exceeds 8 MiB".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CursorTransition {
    pub track_id: TrackId,
    pub from_revision_id: RevisionId,
    pub to_revision_id: RevisionId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventActorKind {
    Human,
    Agent,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventActor {
    pub id: String,
    pub display_name: String,
    pub kind: EventActorKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BridgeEventKind {
    RevisionCommitted {
        request_id: Option<RequestId>,
        change_set_id: ChangeSetId,
        actor: EventActor,
        session_id: SessionId,
        action_label: String,
        cursors: Vec<CursorTransition>,
    },
    CursorMoved {
        session_id: SessionId,
        cursors: Vec<CursorTransition>,
    },
    CollaborationAdvanced {
        agent_session_id: SessionId,
        source_session_id: SessionId,
        cursors: Vec<CursorTransition>,
    },
    CollaborationSplit {
        agent_session_id: SessionId,
        source_session_id: SessionId,
        cursors: Vec<CursorTransition>,
    },
    InteractionBegan {
        interaction_id: String,
        interaction_kind: String,
    },
    InteractionCommitted {
        interaction_id: String,
        cursors: Vec<CursorTransition>,
    },
    InteractionCanceled {
        interaction_id: String,
    },
    ProjectClosed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeEvent {
    pub seq: u64,
    pub event: BridgeEventKind,
}

impl BridgeEvent {
    pub fn encoded_len(&self) -> BridgeResult<usize> {
        Ok(serde_json::to_vec(self)?.len())
    }

    pub fn validate(&self) -> BridgeResult<()> {
        if self.seq == 0 {
            return Err(BridgeError::Protocol(
                "event sequence must be nonzero".into(),
            ));
        }
        match &self.event {
            BridgeEventKind::RevisionCommitted {
                actor,
                action_label,
                cursors,
                ..
            } => {
                bounded("actor id", &actor.id, MAX_STRING_BYTES)?;
                bounded("actor display name", &actor.display_name, MAX_STRING_BYTES)?;
                bounded("action label", action_label, MAX_STRING_BYTES)?;
                if actor.id.is_empty() || actor.display_name.is_empty() || action_label.is_empty() {
                    return Err(BridgeError::Protocol(
                        "event actor and action label must be nonempty".into(),
                    ));
                }
                validate_transitions(cursors)
            }
            BridgeEventKind::CursorMoved { cursors, .. }
            | BridgeEventKind::CollaborationAdvanced { cursors, .. }
            | BridgeEventKind::CollaborationSplit { cursors, .. }
            | BridgeEventKind::InteractionCommitted { cursors, .. } => {
                validate_transitions(cursors)
            }
            BridgeEventKind::InteractionBegan {
                interaction_id,
                interaction_kind,
            } => {
                bounded("interaction id", interaction_id, MAX_STRING_BYTES)?;
                bounded("interaction kind", interaction_kind, MAX_STRING_BYTES)?;
                if interaction_id.is_empty() || interaction_kind.is_empty() {
                    return Err(BridgeError::Protocol(
                        "interaction identity and kind must be nonempty".into(),
                    ));
                }
                Ok(())
            }
            BridgeEventKind::InteractionCanceled { interaction_id } => {
                bounded("interaction id", interaction_id, MAX_STRING_BYTES)?;
                if interaction_id.is_empty() {
                    return Err(BridgeError::Protocol(
                        "interaction identity must be nonempty".into(),
                    ));
                }
                Ok(())
            }
            BridgeEventKind::ProjectClosed => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Request(Box<RequestEnvelope>),
    Subscribe {
        after_seq: u64,
    },
    Ping {
        nonce: u64,
        #[serde(default)]
        padding: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Response(ResponseEnvelope),
    Snapshot(StateSnapshot),
    Event(BridgeEvent),
    ResyncRequired {
        oldest_seq: u64,
        newest_seq: u64,
    },
    RateLimited {
        retry_after_millis: u64,
        disconnect: bool,
    },
    Pong {
        nonce: u64,
    },
}

fn validate_transitions(cursors: &[CursorTransition]) -> BridgeResult<()> {
    if cursors.is_empty() || cursors.len() > MAX_CURSOR_EXPECTATIONS {
        return Err(BridgeError::Protocol(
            "event cursor transitions must contain 1 through 64 entries".into(),
        ));
    }
    if cursors
        .windows(2)
        .any(|pair| pair[0].track_id.as_bytes() >= pair[1].track_id.as_bytes())
    {
        return Err(BridgeError::Protocol(
            "event cursor transitions must be sorted and unique".into(),
        ));
    }
    if cursors
        .iter()
        .any(|cursor| cursor.from_revision_id == cursor.to_revision_id)
    {
        return Err(BridgeError::Protocol(
            "event cursor transition does not move".into(),
        ));
    }
    Ok(())
}

fn validate_cursors(
    name: &str,
    cursors: &[ExpectedCursor],
    require_nonempty: bool,
) -> BridgeResult<()> {
    if require_nonempty && cursors.is_empty() {
        return Err(BridgeError::Protocol(format!("{name} must be nonempty")));
    }
    if cursors.len() > MAX_CURSOR_EXPECTATIONS {
        return Err(BridgeError::Limit(format!(
            "{name} exceeds {MAX_CURSOR_EXPECTATIONS} entries"
        )));
    }
    if cursors
        .windows(2)
        .any(|pair| pair[0].track_id.as_bytes() >= pair[1].track_id.as_bytes())
    {
        return Err(BridgeError::Protocol(format!(
            "{name} must be sorted and unique"
        )));
    }
    Ok(())
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
    fn walk(value: &Value, depth: usize, nodes: &mut usize) -> BridgeResult<()> {
        *nodes = nodes
            .checked_add(1)
            .ok_or_else(|| BridgeError::Limit("JSON node count overflow".into()))?;
        if *nodes > crate::MAX_JSON_NODES {
            return Err(BridgeError::Limit(format!(
                "action JSON exceeds {} aggregate values",
                crate::MAX_JSON_NODES
            )));
        }
        if depth > crate::MAX_JSON_DEPTH {
            return Err(BridgeError::Limit("JSON nesting exceeds 64 levels".into()));
        }
        match value {
            Value::Array(items) => {
                if items.len() > crate::MAX_BATCH_ITEMS {
                    return Err(BridgeError::Limit("JSON array exceeds 128 items".into()));
                }
                for item in items {
                    walk(item, depth + 1, nodes)?;
                }
            }
            Value::Object(fields) => {
                for (key, value) in fields {
                    bounded("action key", key, MAX_STRING_BYTES)?;
                    walk(value, depth + 1, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut nodes = 0;
    walk(value, 0, &mut nodes)
}
