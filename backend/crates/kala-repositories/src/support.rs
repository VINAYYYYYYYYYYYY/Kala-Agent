//! Shared repository mechanics.
//!
//! Everything here is private. The public surface is the repository types,
//! [`crate::MemoryRecord`], and the error types.

use chrono::{DateTime, Utc};
use kala_contracts::{ContractModel, Identifier, Timestamp};
use kala_persistence::{Database, PersistenceError, timestamp_to_iso, truncate_to_micros};
use num_traits::ToPrimitive;
use rusqlite::{Connection, OptionalExtension, Params};

use crate::error::RepositoryError;

/// The Python repositories' default list limit.
pub const DEFAULT_LIST_LIMIT: i64 = 1_000;

/// The error used to abort the accepted transaction API when a repository
/// operation needs to surface an error type the accepted signature cannot
/// carry.
///
/// [`Database::transaction`] only accepts `PersistenceError` from its closure.
/// A repository records the real error beside the closure and returns this
/// sentinel, which guarantees the accepted transaction guard performs its
/// normal rollback (including savepoint rollback) before the recorded error is
/// returned to the caller. The sentinel is never observable through the public
/// API.
const TRANSACTION_ABORT: PersistenceError = PersistenceError::InvalidLimit;

/// Normalize a caller-supplied identifier exactly like Python's
/// `_validate_identifier`.
pub(crate) fn normalized_identifier(
    value: &str,
    field: &'static str,
) -> Result<String, RepositoryError> {
    Identifier::new(value)
        .map(|identifier| identifier.as_str().to_owned())
        .map_err(|_| RepositoryError::InvalidIdentifier { field })
}

/// Validate a list limit exactly like Python's `_validate_limit`.
///
/// Rust's typed `i64` parameter makes Python's `TypeError` for a non-integer
/// impossible at this boundary. Non-positive values still produce the same
/// dynamic `ValueError`-equivalent failure.
pub(crate) fn validated_limit(limit: i64) -> Result<i64, RepositoryError> {
    if limit < 1 {
        return Err(RepositoryError::InvalidLimit);
    }
    Ok(limit)
}

/// Run a repository write through the accepted [`Database::transaction`] API.
///
/// The accepted API starts `BEGIN IMMEDIATE` on an idle connection and a
/// `SAVEPOINT` inside a parent transaction, rolls back on every unsuccessful
/// exit (including a failed `COMMIT`/`RELEASE`) and releases its writer lock on
/// every path. This wrapper only adds room for [`RepositoryError`].
pub(crate) fn with_transaction<T>(
    database: &Database,
    operation: impl FnOnce(&Connection) -> Result<T, RepositoryError>,
) -> Result<T, RepositoryError> {
    let mut slot: Option<Result<T, RepositoryError>> = None;
    let outcome = database.transaction(|connection| match operation(connection) {
        Ok(value) => {
            slot = Some(Ok(value));
            Ok(())
        }
        Err(error) => {
            slot = Some(Err(error));
            Err(TRANSACTION_ABORT)
        }
    });
    match outcome {
        // The accepted transaction only returns `Ok` after the closure ran.
        Ok(()) => slot.expect("transaction closure stores its result before returning"),
        Err(error) => match slot {
            Some(Err(error)) => Err(error),
            _ => Err(RepositoryError::Persistence(error)),
        },
    }
}

/// Format one `AwareDatetime` for a repository `created_at`/`expires_at`
/// column, matching `Database._timestamp_to_iso`.
pub(crate) fn timestamp_column(timestamp: &Timestamp) -> Result<String, RepositoryError> {
    let micros = timestamp
        .absolute_micros()
        .to_i64()
        .ok_or(RepositoryError::TimestampOutOfRange)?;
    let value = DateTime::<Utc>::from_timestamp_micros(micros)
        .ok_or(RepositoryError::TimestampOutOfRange)?;
    timestamp_to_iso(&value).map_err(RepositoryError::Persistence)
}

/// Format the current instant for an expiry comparison.
pub(crate) fn current_timestamp(now: Option<&Timestamp>) -> Result<String, RepositoryError> {
    match now {
        Some(timestamp) => timestamp_column(timestamp),
        None => {
            timestamp_to_iso(&truncate_to_micros(Utc::now())).map_err(RepositoryError::Persistence)
        }
    }
}

/// Serialize a typed contract model for `model_json`.
pub(crate) fn encode_model<T: ContractModel>(model: &T) -> Result<String, RepositoryError> {
    model.to_wire_json().map_err(RepositoryError::Validation)
}

/// Deserialize a stored `model_json` value with the accepted contract
/// validation, including Pydantic-compatible duplicate-key handling.
pub(crate) fn decode_model<T: ContractModel>(
    model: &'static str,
    model_json: String,
) -> Result<T, RepositoryError> {
    T::from_json_str(&model_json).map_err(|source| RepositoryError::StoredModel { model, source })
}

/// Read one optional `model_json` column.
pub(crate) fn query_model<T: ContractModel>(
    connection: &Connection,
    sql: &str,
    params: impl Params,
    model: &'static str,
) -> Result<Option<T>, RepositoryError> {
    let model_json: Option<String> = connection
        .query_row(sql, params, |row| row.get(0))
        .optional()?;
    model_json
        .map(|model_json| decode_model(model, model_json))
        .transpose()
}

/// Read an ordered list of `model_json` columns.
pub(crate) fn query_models<T: ContractModel>(
    connection: &Connection,
    sql: &str,
    params: impl Params,
    model: &'static str,
) -> Result<Vec<T>, RepositoryError> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map(params, |row| row.get::<_, String>(0))?;
    let mut models = Vec::new();
    for row in rows {
        models.push(decode_model(model, row?)?);
    }
    Ok(models)
}
