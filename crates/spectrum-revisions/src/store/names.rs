//! Names on revisions: checkpoints a person or agent named, which also name
//! the branch they sit on in a history view. Kept in the document's metadata,
//! so every document has room for them; naming is not an edit and adds no
//! revision.
use rusqlite::{TransactionBehavior, params};

use super::RevisionStore;
use crate::{RevisionError, RevisionId, RevisionResult, metadata::bump_generation};

const PREFIX: &str = "revision_name:";
/// Longest name, in characters.
pub const MAX_NAME: usize = 80;

impl RevisionStore {
    /// Names `revision`, or with `None` (or a blank name) clears its name.
    pub fn name_revision(
        &mut self,
        revision: RevisionId,
        name: Option<&str>,
    ) -> RevisionResult<()> {
        self.require_revision(revision)?;
        let name = name.map(str::trim).filter(|name| !name.is_empty());
        if name.is_some_and(|name| name.chars().count() > MAX_NAME) {
            return Err(RevisionError::Invalid(format!(
                "a revision name has at most {MAX_NAME} characters"
            )));
        }
        let key = format!("{PREFIX}{revision}");
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("DELETE FROM spectrum_meta WHERE key = ?1", [&key])?;
        if let Some(name) = name {
            transaction.execute(
                "INSERT INTO spectrum_meta(key, value) VALUES (?1, ?2)",
                params![key, name.as_bytes()],
            )?;
        }
        bump_generation(&transaction)?;
        transaction.commit()?;
        self.finish_write()?;
        Ok(())
    }

    /// Every named revision and its name.
    pub fn revision_names(&self) -> RevisionResult<Vec<(RevisionId, String)>> {
        let mut statement = self.connection.prepare(
            "SELECT key, value FROM spectrum_meta WHERE key >= ?1 AND key < ?2 ORDER BY key",
        )?;
        // Keys between the prefix and the prefix with its last byte raised.
        let end = format!("{};", &PREFIX[..PREFIX.len() - 1]);
        let rows = statement.query_map([PREFIX, end.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        let mut names = Vec::new();
        for row in rows {
            let (key, value) = row?;
            let revision = key[PREFIX.len()..]
                .parse::<RevisionId>()
                .map_err(|_| RevisionError::Corrupt(format!("bad revision name key {key}")))?;
            let name = String::from_utf8(value)
                .map_err(|_| RevisionError::Corrupt(format!("revision name {key} is not text")))?;
            names.push((revision, name));
        }
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, RevisionStore, RevisionId) {
        let directory = tempfile::tempdir().unwrap();
        let project = crate::NewProject {
            application_id: "spectrum.names-test".into(),
            application_version: "1".into(),
            actor: crate::Actor {
                id: "person:test".into(),
                display_name: "Tester".into(),
                kind: crate::ActorKind::Human,
            },
            session_id: crate::SessionId::new(),
            root_label: None,
            track_kind: "test".into(),
            track_label: "test".into(),
            initial_snapshots: vec![crate::Payload::new(
                crate::Encoding::new("test.snapshot", 1),
                b"root".to_vec(),
            )],
            assets: Vec::new(),
        };
        let (store, info) =
            RevisionStore::create(&directory.path().join("names.spectrum"), project).unwrap();
        (directory, store, info.root_revision)
    }

    #[test]
    fn names_are_set_listed_renamed_and_cleared() {
        let (_directory, mut store, root) = store();
        assert!(store.revision_names().unwrap().is_empty());
        store.name_revision(root, Some("  First look ")).unwrap();
        assert_eq!(
            store.revision_names().unwrap(),
            vec![(root, "First look".to_string())]
        );
        store.name_revision(root, Some("Final")).unwrap();
        assert_eq!(
            store.revision_names().unwrap(),
            vec![(root, "Final".to_string())]
        );
        store.name_revision(root, Some("   ")).unwrap();
        assert!(store.revision_names().unwrap().is_empty());
        store.name_revision(root, Some("Again")).unwrap();
        store.name_revision(root, None).unwrap();
        assert!(store.revision_names().unwrap().is_empty());
    }

    #[test]
    fn names_are_bounded_and_need_a_revision() {
        let (_directory, mut store, root) = store();
        let long = "x".repeat(MAX_NAME + 1);
        assert!(store.name_revision(root, Some(&long)).is_err());
        assert!(
            store
                .name_revision(RevisionId::new(), Some("Lost"))
                .is_err()
        );
        // Other metadata is not mistaken for names.
        assert!(store.revision_names().unwrap().is_empty());
    }
}
