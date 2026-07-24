use sha2::{Digest, Sha256};

use crate::{
    Compatibility, Encoding, Payload, RevisionError, RevisionId, RevisionResult, RevisionStore,
};

impl RevisionStore {
    pub fn compatible_operation_payload(
        &self,
        revision: RevisionId,
        compatibility: &impl Compatibility,
    ) -> RevisionResult<Option<Payload>> {
        self.best_payload("operation_payloads", revision, |encoding| {
            compatibility.supports_operations(encoding)
        })
    }

    pub(crate) fn best_payload(
        &self,
        table: &str,
        revision: RevisionId,
        supports: impl Fn(&Encoding) -> bool,
    ) -> RevisionResult<Option<Payload>> {
        let sql = format!(
            "SELECT family, version, capabilities_json, bytes, sha256
             FROM {table} WHERE revision_id = ?1 ORDER BY version DESC, family"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map([revision.as_bytes().as_slice()], |row| {
            let family: String = row.get(0)?;
            let version: u32 = row.get(1)?;
            let capabilities_json: String = row.get(2)?;
            let bytes: Vec<u8> = row.get(3)?;
            let hash: Vec<u8> = row.get(4)?;
            Ok((family, version, capabilities_json, bytes, hash))
        })?;
        for row in rows {
            let (family, version, capabilities_json, bytes, hash) = row?;
            if Sha256::digest(&bytes).as_slice() != hash {
                return Err(RevisionError::Corrupt(format!(
                    "payload hash mismatch in {table}"
                )));
            }
            let encoding = Encoding {
                family,
                version,
                required_capabilities: serde_json::from_str(&capabilities_json)?,
            };
            if supports(&encoding) {
                return Ok(Some(Payload { encoding, bytes }));
            }
        }
        Ok(None)
    }
}
