//! Native typed repositories over the accepted persistence schema.
//!
//! Every SQL statement, ordering, tie breaker, limit default and transaction
//! boundary mirrors `kala_agent.persistence.repositories`. The repositories
//! intentionally share the accepted [`Database`] (one serialized SQLite
//! writer); there is no second schema, connection pool or DB abstraction here.
//!
//! Stored models are always decoded through the accepted contract validation.
//! `model_json` bytes read from the database are never rewritten on read, and
//! writers generate the same compact Pydantic-style JSON as
//! `model.model_dump_json()`.

use kala_contracts::{
    ApprovalRequest, ContractModel, DesignRevision, DesignSpec, Timestamp, ToolCall, ToolResult,
};
use kala_persistence::Database;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    error::{ApprovalAlreadyDecidedError, HeadMovedError, RepositoryError},
    memory::MemoryRecord,
    support::{
        DEFAULT_LIST_LIMIT, current_timestamp, encode_model, normalized_identifier, query_model,
        query_models, timestamp_column, validated_limit, with_transaction,
    },
};

// ---------------------------------------------------------------------------
// DesignSpecRepository
// ---------------------------------------------------------------------------

/// Repository for the latest declarative state of each design.
#[derive(Clone, Copy, Debug)]
pub struct DesignSpecRepository<'a> {
    database: &'a Database,
}

impl<'a> DesignSpecRepository<'a> {
    /// Create a repository over the shared database.
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    /// Insert or replace the stored spec, as the Python repository does.
    pub fn save(&self, spec: &DesignSpec) -> Result<(), RepositoryError> {
        let model_json = encode_model(spec)?;
        let id = spec.id().to_owned();
        with_transaction(self.database, |connection| {
            connection.execute(
                r#"
                INSERT INTO design_specs(id, model_json) VALUES (?, ?)
                ON CONFLICT(id) DO UPDATE SET model_json = excluded.model_json
                "#,
                params![id, model_json],
            )?;
            Ok(())
        })
    }

    /// Read one spec, or `None` when absent.
    pub fn get(&self, spec_id: &str) -> Result<Option<DesignSpec>, RepositoryError> {
        let spec_id = normalized_identifier(spec_id, "spec_id")?;
        self.database
            .with_connection(|connection| {
                query_model(
                    connection,
                    "SELECT model_json FROM design_specs WHERE id = ?",
                    params![spec_id],
                    DesignSpec::NAME,
                )
            })
            .map_err(RepositoryError::Persistence)?
    }

    /// List specs ordered by identifier with the Python default limit.
    pub fn list(&self) -> Result<Vec<DesignSpec>, RepositoryError> {
        self.list_with_limit(DEFAULT_LIST_LIMIT)
    }

    /// List specs ordered by identifier with an explicit positive limit.
    pub fn list_with_limit(&self, limit: i64) -> Result<Vec<DesignSpec>, RepositoryError> {
        let limit = validated_limit(limit)?;
        self.database
            .with_connection(|connection| {
                query_models(
                    connection,
                    "SELECT model_json FROM design_specs ORDER BY id LIMIT ?",
                    params![limit],
                    DesignSpec::NAME,
                )
            })
            .map_err(RepositoryError::Persistence)?
    }
}

// ---------------------------------------------------------------------------
// DesignRevisionRepository
// ---------------------------------------------------------------------------

/// The append-only revision store, whose only write path is a head swap.
///
/// There is deliberately no unconditional `save`. A revision row names the
/// design it extends and the sequence it occupies; writing one without first
/// proving the head is still the one the writer prepared against is what
/// forges a fork or an orphan. [`Self::commit_revision`] is therefore the only
/// way in, and the insertion it performs is private to this implementation.
#[derive(Clone, Copy, Debug)]
pub struct DesignRevisionRepository<'a> {
    database: &'a Database,
}

impl<'a> DesignRevisionRepository<'a> {
    /// Create a repository over the shared database.
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    /// Append one revision onto the head the caller prepared against.
    ///
    /// `expected_head_revision_id` is required, so a caller cannot write
    /// without stating what it believes the head is. The head read, sequence
    /// validation, spec insert and revision insert run in one immediate
    /// transaction. Two writers that prepared against the same head cannot both
    /// extend it; the loser receives [`RepositoryError::HeadMoved`] and its
    /// transaction is rolled back before anything is written.
    pub fn commit_revision(
        &self,
        revision: &DesignRevision,
        expected_head_revision_id: Option<&str>,
    ) -> Result<DesignRevision, RepositoryError> {
        let expected_head = expected_head_revision_id
            .map(|value| normalized_identifier(value, "expected_head_revision_id"))
            .transpose()?;
        let design_id = revision.design_id().to_owned();
        with_transaction(self.database, |connection| {
            let head = query_head(connection, &design_id)?;
            let actual_head = head.as_ref().map(|(revision_id, _)| revision_id.clone());
            let actual_sequence = head
                .as_ref()
                .map_or_else(|| BigInt::from(0), |(_, sequence)| BigInt::from(*sequence));
            if actual_head.as_deref() != expected_head.as_deref() {
                return Err(Box::new(HeadMovedError {
                    message: format!(
                        "the head of design {design_id} moved before the revision was written"
                    ),
                    design_id,
                    expected_head_revision_id: expected_head,
                    actual_head_revision_id: actual_head,
                    expected_sequence: revision.sequence().as_bigint().clone(),
                    actual_sequence,
                })
                .into());
            }
            let expected_next = head.as_ref().map_or_else(
                || BigInt::from(0),
                |(_, sequence)| BigInt::from(*sequence) + 1,
            );
            if revision.sequence().as_bigint() != &expected_next {
                return Err(Box::new(HeadMovedError {
                    message: format!(
                        "revision {} does not extend the head of design {design_id}",
                        revision.revision_id()
                    ),
                    design_id,
                    expected_head_revision_id: expected_head,
                    actual_head_revision_id: actual_head,
                    expected_sequence: revision.sequence().as_bigint().clone(),
                    actual_sequence,
                })
                .into());
            }
            let sequence = revision
                .sequence()
                .as_bigint()
                .to_i64()
                .ok_or(RepositoryError::SequenceOutOfRange)?;
            let spec_json = encode_model(revision.spec())?;
            let revision_json = encode_model(revision)?;
            connection.execute(
                r#"
                INSERT INTO design_specs(id, model_json) VALUES (?, ?)
                ON CONFLICT(id) DO NOTHING
                "#,
                params![revision.spec().id(), spec_json],
            )?;
            connection.execute(
                r#"
                INSERT INTO design_revisions(
                    revision_id, design_id, sequence, model_json, created_at
                ) VALUES (?, ?, ?, ?, ?)
                "#,
                params![
                    revision.revision_id(),
                    design_id,
                    sequence,
                    revision_json,
                    timestamp_column(revision.created_at())?,
                ],
            )?;
            Ok(())
        })?;
        Ok(revision.clone())
    }

    /// Read one revision, or `None` when absent.
    pub fn get(&self, revision_id: &str) -> Result<Option<DesignRevision>, RepositoryError> {
        let revision_id = normalized_identifier(revision_id, "revision_id")?;
        self.database
            .with_connection(|connection| {
                query_model(
                    connection,
                    "SELECT model_json FROM design_revisions WHERE revision_id = ?",
                    params![revision_id],
                    DesignRevision::NAME,
                )
            })
            .map_err(RepositoryError::Persistence)?
    }

    /// List one design's revisions ascending by sequence with the default
    /// limit.
    pub fn list_for_design(&self, design_id: &str) -> Result<Vec<DesignRevision>, RepositoryError> {
        self.list_for_design_with_limit(design_id, DEFAULT_LIST_LIMIT)
    }

    /// List one design's revisions ascending by sequence with an explicit
    /// positive limit.
    pub fn list_for_design_with_limit(
        &self,
        design_id: &str,
        limit: i64,
    ) -> Result<Vec<DesignRevision>, RepositoryError> {
        let design_id = normalized_identifier(design_id, "design_id")?;
        let limit = validated_limit(limit)?;
        self.database
            .with_connection(|connection| {
                query_models(
                    connection,
                    r#"
                    SELECT model_json
                    FROM design_revisions
                    WHERE design_id = ?
                    ORDER BY sequence ASC
                    LIMIT ?
                    "#,
                    params![design_id, limit],
                    DesignRevision::NAME,
                )
            })
            .map_err(RepositoryError::Persistence)?
    }

    /// Read the revision with the greatest sequence for a design.
    pub fn latest_for_design(
        &self,
        design_id: &str,
    ) -> Result<Option<DesignRevision>, RepositoryError> {
        let design_id = normalized_identifier(design_id, "design_id")?;
        self.database
            .with_connection(|connection| {
                query_model(
                    connection,
                    r#"
                    SELECT model_json
                    FROM design_revisions
                    WHERE design_id = ?
                    ORDER BY sequence DESC
                    LIMIT 1
                    "#,
                    params![design_id],
                    DesignRevision::NAME,
                )
            })
            .map_err(RepositoryError::Persistence)?
    }
}

fn query_head(
    connection: &Connection,
    design_id: &str,
) -> Result<Option<(String, i64)>, RepositoryError> {
    connection
        .query_row(
            r#"
            SELECT revision_id, sequence
            FROM design_revisions
            WHERE design_id = ?
            ORDER BY sequence DESC
            LIMIT 1
            "#,
            params![design_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(RepositoryError::from)
}

// ---------------------------------------------------------------------------
// ApprovalRequestRepository
// ---------------------------------------------------------------------------

/// Repository for approval checkpoints.
#[derive(Clone, Copy, Debug)]
pub struct ApprovalRequestRepository<'a> {
    database: &'a Database,
}

impl<'a> ApprovalRequestRepository<'a> {
    /// Create a repository over the shared database.
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    /// Insert an approval, or update one that is still pending.
    ///
    /// A decided approval is final: rewriting it with the same record is a
    /// no-op, and any other write, including one that would reopen it as
    /// pending, is refused.
    pub fn save(&self, request: &ApprovalRequest) -> Result<(), RepositoryError> {
        let model_json = encode_model(request)?;
        let request_id = request.request_id().to_owned();
        let workflow_id = request.workflow_id().to_owned();
        let status = request.status().as_str().to_owned();
        let created_at = timestamp_column(request.created_at())?;
        with_transaction(self.database, |connection| {
            let changed = connection.execute(
                r#"
                INSERT INTO approval_requests(
                    request_id, workflow_id, status, created_at, model_json
                ) VALUES (?, ?, ?, ?, ?)
                ON CONFLICT(request_id) DO UPDATE SET
                    workflow_id = excluded.workflow_id,
                    status = excluded.status,
                    created_at = excluded.created_at,
                    model_json = excluded.model_json
                WHERE approval_requests.status = 'pending'
                    OR approval_requests.model_json = excluded.model_json
                "#,
                params![request_id, workflow_id, status, created_at, model_json],
            )?;
            if changed == 0 {
                return Err(ApprovalAlreadyDecidedError { request_id }.into());
            }
            Ok(())
        })
    }

    /// Read one approval, or `None` when absent.
    pub fn get(&self, request_id: &str) -> Result<Option<ApprovalRequest>, RepositoryError> {
        let request_id = normalized_identifier(request_id, "request_id")?;
        self.database
            .with_connection(|connection| {
                query_model(
                    connection,
                    "SELECT model_json FROM approval_requests WHERE request_id = ?",
                    params![request_id],
                    ApprovalRequest::NAME,
                )
            })
            .map_err(RepositoryError::Persistence)?
    }

    /// List a workflow's approvals by `created_at`, then `request_id`, with
    /// the Python default limit.
    pub fn list_for_workflow(
        &self,
        workflow_id: &str,
    ) -> Result<Vec<ApprovalRequest>, RepositoryError> {
        self.list_for_workflow_with_limit(workflow_id, DEFAULT_LIST_LIMIT)
    }

    /// List a workflow's approvals with an explicit positive limit.
    pub fn list_for_workflow_with_limit(
        &self,
        workflow_id: &str,
        limit: i64,
    ) -> Result<Vec<ApprovalRequest>, RepositoryError> {
        let workflow_id = normalized_identifier(workflow_id, "workflow_id")?;
        let limit = validated_limit(limit)?;
        self.database
            .with_connection(|connection| {
                query_models(
                    connection,
                    r#"
                    SELECT model_json
                    FROM approval_requests
                    WHERE workflow_id = ?
                    ORDER BY created_at ASC, request_id ASC
                    LIMIT ?
                    "#,
                    params![workflow_id, limit],
                    ApprovalRequest::NAME,
                )
            })
            .map_err(RepositoryError::Persistence)?
    }
}

// ---------------------------------------------------------------------------
// ToolCallRepository
// ---------------------------------------------------------------------------

/// Repository for tool-call requests.
#[derive(Clone, Copy, Debug)]
pub struct ToolCallRepository<'a> {
    database: &'a Database,
}

impl<'a> ToolCallRepository<'a> {
    /// Create a repository over the shared database.
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    /// Insert or replace a tool call.
    pub fn save(&self, call: &ToolCall) -> Result<(), RepositoryError> {
        let model_json = encode_model(call)?;
        let call_id = call.call_id().to_owned();
        let tool_id = call.tool_id().to_owned();
        let issued_at = timestamp_column(call.issued_at())?;
        with_transaction(self.database, |connection| {
            connection.execute(
                r#"
                INSERT INTO tool_calls(call_id, tool_id, issued_at, model_json)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(call_id) DO UPDATE SET
                    tool_id = excluded.tool_id,
                    issued_at = excluded.issued_at,
                    model_json = excluded.model_json
                "#,
                params![call_id, tool_id, issued_at, model_json],
            )?;
            Ok(())
        })
    }

    /// Read one call, or `None` when absent.
    pub fn get(&self, call_id: &str) -> Result<Option<ToolCall>, RepositoryError> {
        let call_id = normalized_identifier(call_id, "call_id")?;
        self.database
            .with_connection(|connection| {
                query_model(
                    connection,
                    "SELECT model_json FROM tool_calls WHERE call_id = ?",
                    params![call_id],
                    ToolCall::NAME,
                )
            })
            .map_err(RepositoryError::Persistence)?
    }

    /// List a tool's calls by `issued_at`, then `call_id`, with the Python
    /// default limit.
    pub fn list_for_tool(&self, tool_id: &str) -> Result<Vec<ToolCall>, RepositoryError> {
        self.list_for_tool_with_limit(tool_id, DEFAULT_LIST_LIMIT)
    }

    /// List a tool's calls with an explicit positive limit.
    pub fn list_for_tool_with_limit(
        &self,
        tool_id: &str,
        limit: i64,
    ) -> Result<Vec<ToolCall>, RepositoryError> {
        let tool_id = normalized_identifier(tool_id, "tool_id")?;
        let limit = validated_limit(limit)?;
        self.database
            .with_connection(|connection| {
                query_models(
                    connection,
                    r#"
                    SELECT model_json
                    FROM tool_calls
                    WHERE tool_id = ?
                    ORDER BY issued_at ASC, call_id ASC
                    LIMIT ?
                    "#,
                    params![tool_id, limit],
                    ToolCall::NAME,
                )
            })
            .map_err(RepositoryError::Persistence)?
    }
}

// ---------------------------------------------------------------------------
// ToolResultRepository
// ---------------------------------------------------------------------------

/// Repository for tool results, whose foreign key and identity must agree with
/// the stored call.
#[derive(Clone, Copy, Debug)]
pub struct ToolResultRepository<'a> {
    database: &'a Database,
}

impl<'a> ToolResultRepository<'a> {
    /// Create a repository over the shared database.
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    /// Insert or replace a result after checking that its call exists and its
    /// `tool_id` matches, atomically with the write.
    pub fn save(&self, result: &ToolResult) -> Result<(), RepositoryError> {
        let call_id = result.call_id().to_owned();
        with_transaction(self.database, |connection| {
            let call_tool_id: Option<String> = connection
                .query_row(
                    "SELECT tool_id FROM tool_calls WHERE call_id = ?",
                    params![call_id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(call_tool_id) = call_tool_id else {
                return Err(RepositoryError::ToolResultMissingCall);
            };
            if call_tool_id != result.tool_id() {
                return Err(RepositoryError::ToolResultToolMismatch);
            }
            let model_json = encode_model(result)?;
            connection.execute(
                r#"
                INSERT INTO tool_results(call_id, tool_id, status, model_json)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(call_id) DO UPDATE SET
                    tool_id = excluded.tool_id,
                    status = excluded.status,
                    model_json = excluded.model_json
                "#,
                params![
                    call_id,
                    result.tool_id(),
                    result.status().as_str(),
                    model_json
                ],
            )?;
            Ok(())
        })
    }

    /// Read one result, or `None` when absent.
    pub fn get(&self, call_id: &str) -> Result<Option<ToolResult>, RepositoryError> {
        let call_id = normalized_identifier(call_id, "call_id")?;
        self.database
            .with_connection(|connection| {
                query_model(
                    connection,
                    "SELECT model_json FROM tool_results WHERE call_id = ?",
                    params![call_id],
                    ToolResult::NAME,
                )
            })
            .map_err(RepositoryError::Persistence)?
    }
}

// ---------------------------------------------------------------------------
// MemoryRepository
// ---------------------------------------------------------------------------

/// Options for [`MemoryRepository::get_with_options`].
#[derive(Clone, Debug, Default)]
pub struct MemoryGetOptions {
    /// The instant to compare against `expires_at`; defaults to now.
    pub now: Option<Timestamp>,
    /// Whether an expired record is still returned.
    pub include_expired: bool,
}

impl MemoryGetOptions {
    /// Use a fixed comparison instant.
    pub fn with_now(mut self, now: Timestamp) -> Self {
        self.now = Some(now);
        self
    }

    /// Include records whose expiry is not after the comparison instant.
    pub fn with_include_expired(mut self, include_expired: bool) -> Self {
        self.include_expired = include_expired;
        self
    }
}

/// Options for [`MemoryRepository::list_with_options`].
#[derive(Clone, Debug)]
pub struct MemoryListOptions {
    /// Optional normalized `source` filter.
    pub source: Option<String>,
    /// The instant to compare against `expires_at`; defaults to now.
    pub now: Option<Timestamp>,
    /// Whether expired records are still returned.
    pub include_expired: bool,
    /// Positive row limit.
    pub limit: i64,
}

impl Default for MemoryListOptions {
    fn default() -> Self {
        Self {
            source: None,
            now: None,
            include_expired: false,
            limit: DEFAULT_LIST_LIMIT,
        }
    }
}

impl MemoryListOptions {
    /// Filter by source identifier before querying.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Use a fixed comparison instant.
    pub fn with_now(mut self, now: Timestamp) -> Self {
        self.now = Some(now);
        self
    }

    /// Include records whose expiry is not after the comparison instant.
    pub fn with_include_expired(mut self, include_expired: bool) -> Self {
        self.include_expired = include_expired;
        self
    }

    /// Set the positive row limit.
    pub fn with_limit(mut self, limit: i64) -> Self {
        self.limit = limit;
        self
    }
}

/// Repository for durable memory entries.
#[derive(Clone, Copy, Debug)]
pub struct MemoryRepository<'a> {
    database: &'a Database,
}

impl<'a> MemoryRepository<'a> {
    /// Create a repository over the shared database.
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    /// Insert or replace one memory record.
    pub fn put(&self, record: &MemoryRecord) -> Result<(), RepositoryError> {
        let model_json = encode_model(record)?;
        let memory_id = record.memory_id().to_owned();
        let source = record.source().to_owned();
        let provenance_id = record.provenance_id().map(str::to_owned);
        let created_at = timestamp_column(record.created_at())?;
        let expires_at = record.expires_at().map(timestamp_column).transpose()?;
        with_transaction(self.database, |connection| {
            connection.execute(
                r#"
                INSERT INTO memory_entries(
                    memory_id, source, provenance_id, created_at, expires_at, model_json
                ) VALUES (?, ?, ?, ?, ?, ?)
                ON CONFLICT(memory_id) DO UPDATE SET
                    source = excluded.source,
                    provenance_id = excluded.provenance_id,
                    created_at = excluded.created_at,
                    expires_at = excluded.expires_at,
                    model_json = excluded.model_json
                "#,
                params![
                    memory_id,
                    source,
                    provenance_id,
                    created_at,
                    expires_at,
                    model_json
                ],
            )?;
            Ok(())
        })
    }

    /// Read one active memory record at the current instant.
    pub fn get(&self, memory_id: &str) -> Result<Option<MemoryRecord>, RepositoryError> {
        self.get_with_options(memory_id, &MemoryGetOptions::default())
    }

    /// Read one memory record with explicit expiry options.
    pub fn get_with_options(
        &self,
        memory_id: &str,
        options: &MemoryGetOptions,
    ) -> Result<Option<MemoryRecord>, RepositoryError> {
        let memory_id = normalized_identifier(memory_id, "memory_id")?;
        if options.include_expired {
            self.database
                .with_connection(|connection| {
                    query_model(
                        connection,
                        "SELECT model_json FROM memory_entries WHERE memory_id = ?",
                        params![memory_id],
                        MemoryRecord::NAME,
                    )
                })
                .map_err(RepositoryError::Persistence)?
        } else {
            let current = current_timestamp(options.now.as_ref())?;
            self.database
                .with_connection(|connection| {
                    query_model(
                        connection,
                        r#"
                        SELECT model_json
                        FROM memory_entries
                        WHERE memory_id = ?
                          AND (expires_at IS NULL OR expires_at > ?)
                        "#,
                        params![memory_id, current],
                        MemoryRecord::NAME,
                    )
                })
                .map_err(RepositoryError::Persistence)?
        }
    }

    /// List active memory records ordered by `created_at`, then `memory_id`,
    /// with the Python default limit.
    pub fn list(&self) -> Result<Vec<MemoryRecord>, RepositoryError> {
        self.list_with_options(&MemoryListOptions::default())
    }

    /// List memory records with explicit source, expiry and limit options.
    pub fn list_with_options(
        &self,
        options: &MemoryListOptions,
    ) -> Result<Vec<MemoryRecord>, RepositoryError> {
        let source = match &options.source {
            Some(source) => Some(normalized_identifier(source, "source")?),
            None => None,
        };
        let limit = validated_limit(options.limit)?;
        let current = current_timestamp(options.now.as_ref())?;
        if options.include_expired {
            self.database
                .with_connection(|connection| {
                    query_models(
                        connection,
                        r#"
                        SELECT model_json
                        FROM memory_entries
                        WHERE (? IS NULL OR source = ?)
                        ORDER BY created_at ASC, memory_id ASC
                        LIMIT ?
                        "#,
                        params![source.as_deref(), source.as_deref(), limit],
                        MemoryRecord::NAME,
                    )
                })
                .map_err(RepositoryError::Persistence)?
        } else {
            self.database
                .with_connection(|connection| {
                    query_models(
                        connection,
                        r#"
                        SELECT model_json
                        FROM memory_entries
                        WHERE (? IS NULL OR source = ?)
                          AND (expires_at IS NULL OR expires_at > ?)
                        ORDER BY created_at ASC, memory_id ASC
                        LIMIT ?
                        "#,
                        params![source.as_deref(), source.as_deref(), current, limit],
                        MemoryRecord::NAME,
                    )
                })
                .map_err(RepositoryError::Persistence)?
        }
    }

    /// Delete one memory record and report whether a row was removed.
    pub fn delete(&self, memory_id: &str) -> Result<bool, RepositoryError> {
        let memory_id = normalized_identifier(memory_id, "memory_id")?;
        with_transaction(self.database, |connection| {
            let changed = connection.execute(
                "DELETE FROM memory_entries WHERE memory_id = ?",
                params![memory_id],
            )?;
            Ok(changed == 1)
        })
    }
}
