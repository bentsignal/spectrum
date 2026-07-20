use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::{RevisionError, RevisionResult, SessionId};

pub(crate) type StorageStateId = [u8; 16];

pub(crate) fn write_meta(
    transaction: &Transaction<'_>,
    key: &str,
    value: &[u8],
) -> RevisionResult<()> {
    transaction.execute(
        "INSERT INTO spectrum_meta(key, value) VALUES (?1, ?2)",
        params![key, value],
    )?;
    Ok(())
}

fn upsert_meta(transaction: &Transaction<'_>, key: &str, value: &[u8]) -> RevisionResult<()> {
    transaction.execute(
        "INSERT INTO spectrum_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub(crate) fn bump_generation(transaction: &Transaction<'_>) -> RevisionResult<u64> {
    let next = generation_in(transaction)?.saturating_add(1);
    upsert_meta(
        transaction,
        "storage_generation",
        next.to_string().as_bytes(),
    )?;
    upsert_meta(transaction, "storage_state_id", SessionId::new().as_bytes())?;
    Ok(next)
}

pub(crate) fn generation_in(connection: &Connection) -> RevisionResult<u64> {
    let value: Option<Vec<u8>> = connection
        .query_row(
            "SELECT value FROM spectrum_meta WHERE key = 'storage_generation'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    value.map_or(Ok(0), |value| {
        String::from_utf8(value)
            .map_err(|_| RevisionError::Corrupt("storage generation is not UTF-8".into()))?
            .parse()
            .map_err(|_| RevisionError::Corrupt("storage generation is invalid".into()))
    })
}

pub(crate) fn state_id_in(connection: &Connection) -> RevisionResult<Option<StorageStateId>> {
    let value: Option<Vec<u8>> = connection
        .query_row(
            "SELECT value FROM spectrum_meta WHERE key = 'storage_state_id'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    value
        .map(|value| {
            value
                .try_into()
                .map_err(|_| RevisionError::Corrupt("storage state id has the wrong length".into()))
        })
        .transpose()
}
