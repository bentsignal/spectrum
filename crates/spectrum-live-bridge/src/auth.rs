use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use spectrum_revisions::ProjectId;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    BindingId, BridgeError, BridgeResult, CapabilityId, InstanceId, PROTOCOL_FAMILY,
    PROTOCOL_VERSION,
};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthChallenge {
    pub protocol: String,
    pub version: u16,
    pub binding_id: BindingId,
    pub binding_epoch: u64,
    pub instance_id: InstanceId,
    pub project_id: ProjectId,
    pub capability_id: CapabilityId,
    pub server_nonce: [u8; 32],
    pub issued_unix_millis: u64,
}

impl AuthChallenge {
    pub fn new(
        binding_id: BindingId,
        binding_epoch: u64,
        instance_id: InstanceId,
        project_id: ProjectId,
        capability_id: CapabilityId,
    ) -> BridgeResult<Self> {
        let mut server_nonce = [0_u8; 32];
        getrandom::fill(&mut server_nonce)
            .map_err(|error| BridgeError::Authentication(error.to_string()))?;
        Ok(Self {
            protocol: PROTOCOL_FAMILY.into(),
            version: PROTOCOL_VERSION,
            binding_id,
            binding_epoch,
            instance_id,
            project_id,
            capability_id,
            server_nonce,
            issued_unix_millis: unix_millis()?,
        })
    }

    pub fn validate(&self) -> BridgeResult<()> {
        if self.protocol != PROTOCOL_FAMILY || self.version != PROTOCOL_VERSION {
            return Err(BridgeError::Authentication(
                "protocol downgrade or family mismatch".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthProof {
    pub client_nonce: [u8; 32],
    pub proof: [u8; 32],
}

pub struct Capability {
    id: CapabilityId,
    secret: [u8; 32],
}

impl Capability {
    pub fn generate() -> BridgeResult<Self> {
        let mut secret = [0_u8; 32];
        getrandom::fill(&mut secret)
            .map_err(|error| BridgeError::Authentication(error.to_string()))?;
        Ok(Self {
            id: CapabilityId::new(),
            secret,
        })
    }

    pub fn from_secret(id: CapabilityId, secret: [u8; 32]) -> Self {
        Self { id, secret }
    }

    pub fn id(&self) -> CapabilityId {
        self.id
    }

    pub fn prove(&self, challenge: &AuthChallenge) -> BridgeResult<AuthProof> {
        challenge.validate()?;
        if challenge.capability_id != self.id {
            return Err(BridgeError::Authentication(
                "capability identity mismatch".into(),
            ));
        }
        let mut client_nonce = [0_u8; 32];
        getrandom::fill(&mut client_nonce)
            .map_err(|error| BridgeError::Authentication(error.to_string()))?;
        let proof = calculate(&self.secret, challenge, &client_nonce)?;
        Ok(AuthProof {
            client_nonce,
            proof,
        })
    }

    pub(crate) fn secret(&self) -> &[u8; 32] {
        &self.secret
    }

    pub(crate) fn copy_secret(&self) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(self.secret.to_vec())
    }
}

impl Drop for Capability {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
}

pub fn verify_proof(
    capability: &Capability,
    challenge: &AuthChallenge,
    proof: &AuthProof,
) -> BridgeResult<()> {
    challenge.validate()?;
    if challenge.capability_id != capability.id {
        return Err(BridgeError::Authentication(
            "capability identity mismatch".into(),
        ));
    }
    let mut mac = keyed(capability.secret())?;
    update_domain(&mut mac, challenge, &proof.client_nonce);
    mac.verify_slice(&proof.proof)
        .map_err(|_| BridgeError::Authentication("bad proof".into()))
}

fn calculate(
    secret: &[u8; 32],
    challenge: &AuthChallenge,
    client_nonce: &[u8; 32],
) -> BridgeResult<[u8; 32]> {
    let mut mac = keyed(secret)?;
    update_domain(&mut mac, challenge, client_nonce);
    Ok(mac.finalize().into_bytes().into())
}

fn keyed(secret: &[u8; 32]) -> BridgeResult<HmacSha256> {
    HmacSha256::new_from_slice(secret)
        .map_err(|_| BridgeError::Authentication("invalid capability".into()))
}

fn update_domain(mac: &mut HmacSha256, challenge: &AuthChallenge, client_nonce: &[u8; 32]) {
    mac.update(PROTOCOL_FAMILY.as_bytes());
    mac.update(&challenge.version.to_be_bytes());
    mac.update(challenge.binding_id.as_bytes());
    mac.update(&challenge.binding_epoch.to_be_bytes());
    mac.update(challenge.instance_id.as_bytes());
    mac.update(challenge.project_id.as_bytes());
    mac.update(challenge.capability_id.as_bytes());
    mac.update(&challenge.server_nonce);
    mac.update(client_nonce);
}

fn unix_millis() -> BridgeResult<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| BridgeError::Authentication("system clock precedes epoch".into()))?
        .as_millis();
    u64::try_from(millis).map_err(|_| BridgeError::Authentication("system clock overflow".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_is_bound_to_every_identity_and_nonce() {
        let capability = Capability::generate().unwrap();
        let challenge = AuthChallenge::new(
            BindingId::new(),
            4,
            InstanceId::new(),
            ProjectId::new(),
            capability.id(),
        )
        .unwrap();
        let proof = capability.prove(&challenge).unwrap();
        verify_proof(&capability, &challenge, &proof).unwrap();

        let mut wrong = challenge.clone();
        wrong.binding_epoch += 1;
        assert!(verify_proof(&capability, &wrong, &proof).is_err());
        let mut replayed = proof;
        replayed.client_nonce[0] ^= 1;
        assert!(verify_proof(&capability, &challenge, &replayed).is_err());
    }

    #[test]
    fn downgrade_is_rejected() {
        let capability = Capability::generate().unwrap();
        let mut challenge = AuthChallenge::new(
            BindingId::new(),
            1,
            InstanceId::new(),
            ProjectId::new(),
            capability.id(),
        )
        .unwrap();
        challenge.version = 0;
        assert!(capability.prove(&challenge).is_err());
    }
}
