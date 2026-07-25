use std::{thread, time::Duration};

use crate::{
    AuthChallenge, BridgeError, BridgeResult, Capability, ClientMessage, EndpointAddress,
    LocalStream, RequestEnvelope, ResponseEnvelope, ServerMessage, StateSnapshot, read_frame,
    write_frame,
};

#[derive(Clone, Debug)]
pub struct ClientConfig {
    pub endpoint: EndpointAddress,
    pub initial_backoff: Duration,
    pub maximum_backoff: Duration,
    pub attempts: usize,
}

impl ClientConfig {
    pub fn local(endpoint: EndpointAddress) -> Self {
        Self {
            endpoint,
            initial_backoff: Duration::from_millis(50),
            maximum_backoff: Duration::from_secs(2),
            attempts: 8,
        }
    }
}

pub struct BridgeClient {
    stream: LocalStream,
}

impl BridgeClient {
    pub fn connect(config: &ClientConfig, capability: &Capability) -> BridgeResult<Self> {
        let mut backoff = config.initial_backoff;
        let mut last_error = None;
        for attempt in 0..config.attempts.max(1) {
            match LocalStream::connect(&config.endpoint) {
                Ok(mut stream) => {
                    stream.set_read_timeout(Some(crate::AUTH_DEADLINE))?;
                    stream.set_write_timeout(Some(crate::AUTH_DEADLINE))?;
                    let challenge: AuthChallenge = read_frame(&mut stream)?;
                    let proof = capability.prove(&challenge)?;
                    write_frame(&mut stream, &proof)?;
                    stream.set_read_timeout(Some(crate::IDLE_TIMEOUT))?;
                    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
                    return Ok(Self { stream });
                }
                Err(error) => last_error = Some(error),
            }
            if attempt + 1 < config.attempts {
                thread::sleep(backoff);
                backoff = backoff.saturating_mul(2).min(config.maximum_backoff);
            }
        }
        Err(last_error.unwrap_or(BridgeError::Closed))
    }

    pub fn request(&mut self, request: RequestEnvelope) -> BridgeResult<ResponseEnvelope> {
        write_frame(&mut self.stream, &ClientMessage::Request(Box::new(request)))?;
        match read_frame(&mut self.stream)? {
            ServerMessage::Response(response) => Ok(response),
            _ => Err(BridgeError::Protocol(
                "server did not answer request with a response".into(),
            )),
        }
    }

    pub fn ping(&mut self, nonce: u64) -> BridgeResult<()> {
        write_frame(&mut self.stream, &ClientMessage::Ping { nonce })?;
        match read_frame(&mut self.stream)? {
            ServerMessage::Pong { nonce: received } if received == nonce => Ok(()),
            _ => Err(BridgeError::Protocol("invalid ping response".into())),
        }
    }

    pub fn subscribe(&mut self, after_seq: u64) -> BridgeResult<StateSnapshot> {
        write_frame(&mut self.stream, &ClientMessage::Subscribe { after_seq })?;
        match read_frame(&mut self.stream)? {
            ServerMessage::Snapshot(snapshot) => Ok(snapshot),
            _ => Err(BridgeError::Protocol(
                "subscription did not begin with a snapshot".into(),
            )),
        }
    }

    pub fn read_subscription_message(&mut self) -> BridgeResult<ServerMessage> {
        read_frame(&mut self.stream)
    }
}
