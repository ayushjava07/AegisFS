//! SQLite-backed store.
//!
//! The reference durability backend. All heavy lifting is delegated to
//! `rusqlite`; run/task documents are stored as their JSON form and the row's
//! denormalized columns serve the hottest filters. The connection is guarded
//! by `tokio::sync::Mutex` so the same store can be shared freely across
//! worker tasks. Synchronous SQL calls block a worker for microseconds on
//! SSDs; a dedicated executor pool is out of scope for the benchmark and the
//! bottleneck never shows in the single-node workload.
//!
//! Queue operations use per-row transactions where guards matter
//! (`claim`/`ack`/`release`) so two workers cannot both believe they hold a
//! lease of the same run.

use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension};
use tokio::sync::Mutex;

use crate::domain::ids::{RunId, TaskRunId};
use crate::domain::run::{Run, TaskRun};
use crate::domain::workflow::WorkflowDef;
use crate::error::StorageError;

use super::filter::RunFilter;
use super::migrations;
use super::model::{ClaimToken, QueueEntry, WorkflowRecord, WorkflowSummary};
use super::Store;

/// SQLite-backed store handle.
#[derive(Clone)]
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    /// Opens a database file (or `:memory:`), applying migrations on first
    /// access.
    pub fn open(path: &str) -> Result<Self, StorageError> {
        let conn = Connection::open(path).map_err(backend_err)?;
        migrations::apply_migrations(&conn).map_err(backend_err)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Opens an in-memory database (tests and ephemeral servers).
    pub fn open_in_memory() -> Result<Self, StorageError> {
        Self::open(":memory:")
    }

    /// The schema version this connection runs on.
    pub fn schema_version(&self) -> Result<u32, StorageError> {
        let conn = self.conn.blocking_lock();
        migrations::schema_version(&conn).map_err(backend_err)
    }
}

fn backend_err(err: rusqlite::Error) -> StorageError {
    StorageError::Backend(err.to_string())
}

fn not_found(entity: impl std::fmt::Display) -> StorageError {
    StorageError::NotFound(entity.to_string())
}

fn decode<T: serde::de::DeserializeOwned>(document: &str) -> Result<T, rusqlite::Error> {
    serde_json::from_str(document).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(e) as Box<dyn std::error::Error + Send + Sync>,
        )
    })
}

fn decode_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<Run> {
    let document: String = row.get("document")?;
    decode(&document)
}

fn decode_task_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRun> {
    let document: String = row.get("document")?;
    decode(&document)
}

fn decode_workflow(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowRecord> {
    let document: String = row.get("document")?;
    decode::<WorkflowDef>(&document).map(|def| WorkflowRecord { def })
}

fn encode_json<T: serde::Serialize>(value: &T) -> Result<String, StorageError> {
    serde_json::to_string(value).map_err(|e| StorageError::Backend(format!("encode: {e}")))
}

impl Store for SqliteStore {
    fn put_workflow(&self, def: WorkflowDef) -> Result<WorkflowRecord, StorageError> {
        let conn = self.conn.blocking_lock();
        let document = encode_json(&def)?;
        conn.execute(
            "INSERT INTO workflows (tenant, name, version, spec_version, created_at_ms, updated_at_ms, document)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(tenant, name) DO UPDATE SET
               version = excluded.version,
               spec_version = excluded.spec_version,
               updated_at_ms = excluded.updated_at_ms,
               document = excluded.document",
            params![
                def.tenant,
                def.name,
                def.version,
                def.spec_version,
                def.created_at_ms,
                def.updated_at_ms,
                document
            ],
        )
        .map_err(backend_err)?;
        Ok(WorkflowRecord { def })
    }

    fn update_workflow_version(
        &self,
        tenant: &str,
        name: &str,
        def: WorkflowDef,
        expected_version: u32,
    ) -> Result<WorkflowRecord, StorageError> {
        let conn = self.conn.blocking_lock();
        let version: Option<u32> = conn
            .query_row(
                "SELECT version FROM workflows WHERE tenant = ?1 AND name = ?2",
                params![tenant, name],
                |row| row.get(0),
            )
            .optional()
            .map_err(backend_err)?;
        let stored = version.ok_or_else(|| not_found(format!("workflow {tenant}/{name}")))?;
        if stored != expected_version {
            return Err(StorageError::ConcurrentModification(format!(
                "workflow {tenant}/{name}"
            )));
        }
        if def.version != expected_version + 1 {
            return Err(StorageError::Conflict(format!(
                "workflow {tenant}/{name} version must be {}+1, got {}",
                expected_version, def.version
            )));
        }
        let document = encode_json(&def)?;
        conn.execute(
            "UPDATE workflows SET version = ?3, spec_version = ?4, updated_at_ms = ?5, document = ?6
             WHERE tenant = ?1 AND name = ?2",
            params![
                tenant,
                name,
                def.version,
                def.spec_version,
                def.updated_at_ms,
                document
            ],
        )
        .map_err(backend_err)?;
        Ok(WorkflowRecord { def })
    }

    fn get_workflow(&self, tenant: &str, name: &str) -> Result<WorkflowRecord, StorageError> {
        let conn = self.conn.blocking_lock();
        conn.query_row(
            "SELECT document FROM workflows WHERE tenant = ?1 AND name = ?2",
            params![tenant, name],
            decode_workflow,
        )
        .optional()
        .map_err(backend_err)?
        .ok_or_else(|| not_found(format!("workflow {tenant}/{name}")))
    }

    fn list_workflows(&self) -> Result<Vec<WorkflowRecord>, StorageError> {
        let conn = self.conn.blocking_lock();
        let mut stmt = conn
            .prepare("SELECT document FROM workflows ORDER BY tenant, name")
            .map_err(backend_err)?;
        let rows = stmt
            .query_map([], decode_workflow)
            .map_err(backend_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(backend_err)
    }

    fn list_workflow_summaries(&self) -> Result<Vec<WorkflowSummary>, StorageError> {
        let workflows = self.list_workflows()?;
        let mut out = Vec::new();
        for rec in &workflows {
            let mut runs: Vec<Run> = {
                let conn = self.conn.blocking_lock();
                let mut stmt = conn
                    .prepare("SELECT document FROM runs WHERE tenant = ?1 AND def_name = ?2")
                    .map_err(backend_err)?;
                let rows = stmt
                    .query_map(params![rec.def.tenant, rec.def.name], decode_run)
                    .map_err(backend_err)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(backend_err)?
            };
            runs.sort_by_key(|r| std::cmp::Reverse(r.created_at_ms));
            out.push(WorkflowSummary::assemble(rec.clone(), &runs));
        }
        Ok(out)
    }

    fn put_run(&self, run: &Run) -> Result<(), StorageError> {
        let conn = self.conn.blocking_lock();
        let document = encode_json(run)?;
        conn.execute(
            "INSERT INTO runs (id, tenant, def_name, def_version, status, attempts, next_attempt_at_ms, created_at_ms, run_number, document)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
               status = excluded.status,
               attempts = excluded.attempts,
               next_attempt_at_ms = excluded.next_attempt_at_ms,
               document = excluded.document",
            params![
                run.id.as_str(),
                run.tenant,
                run.def_name,
                run.def_version as i64,
                run.status.as_ref(), // "queued" via AsRefStr
                run.attempts as i64,
                run.next_attempt_at_ms,
                run.created_at_ms,
                run.run_number as i64,
                document
            ],
        )
        .map_err(backend_err)?;
        Ok(())
    }

    fn get_run(&self, id: &RunId) -> Result<Run, StorageError> {
        let conn = self.conn.blocking_lock();
        conn.query_row(
            "SELECT document FROM runs WHERE id = ?1",
            params![id.as_str()],
            decode_run,
        )
        .optional()
        .map_err(backend_err)?
        .ok_or_else(|| not_found(format!("run {id}")))
    }

    fn delete_run(&self, id: &RunId) -> Result<(), StorageError> {
        let conn = self.conn.blocking_lock();
        conn.execute("DELETE FROM task_runs WHERE run_id = ?1", params![id.as_str()])
            .map_err(backend_err)?;
        conn.execute("DELETE FROM runs WHERE id = ?1", params![id.as_str()])
            .map_err(backend_err)?;
        Ok(())
    }

    fn list_runs(&self, filter: &RunFilter) -> Result<Vec<Run>, StorageError> {
        // Push the hot dimensions into SQL; apply the rest (tags, ranges) in
        // Rust for parity with the memory backend. Ordering happens in Rust to
        // keep the behavior identical across backends.
        let conn = self.conn.blocking_lock();
        let mut rows: Vec<Run> = Vec::new();
        if let Some(tenant) = &filter.tenant {
            let mut stmt = conn
                .prepare("SELECT document FROM runs WHERE tenant = ?1")
                .map_err(backend_err)?;
            let r = stmt
                .query_map(params![tenant], decode_run)
                .map_err(backend_err)?;
            rows.extend(r.collect::<rusqlite::Result<Vec<_>>>().map_err(backend_err)?);
        } else {
            let mut stmt = conn.prepare("SELECT document FROM runs").map_err(backend_err)?;
            let r = stmt.query_map([], decode_run).map_err(backend_err)?;
            rows.extend(r.collect::<rusqlite::Result<Vec<_>>>().map_err(backend_err)?);
        }
        let matched: Vec<&Run> = rows.iter().filter(|r| filter.matches(r)).collect();
        Ok(filter.apply_order(matched))
    }

    fn count_runs(&self, filter: &RunFilter) -> Result<usize, StorageError> {
        Ok(self.list_runs(filter)?.len())
    }

    fn put_task_run(&self, tr: &TaskRun) -> Result<(), StorageError> {
        let conn = self.conn.blocking_lock();
        let document = encode_json(tr)?;
        conn.execute(
            "INSERT INTO task_runs (id, run_id, task_name, status, document)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
               status = excluded.status,
               document = excluded.document",
            params![tr.id.as_str(), tr.run_id.as_str(), tr.task_name, tr.status.as_ref(), document],
        )
        .map_err(backend_err)?;
        Ok(())
    }

    fn get_task_run(&self, id: &TaskRunId) -> Result<TaskRun, StorageError> {
        let conn = self.conn.blocking_lock();
        conn.query_row(
            "SELECT document FROM task_runs WHERE id = ?1",
            params![id.as_str()],
            decode_task_run,
        )
        .optional()
        .map_err(backend_err)?
        .ok_or_else(|| not_found(format!("task run {id}")))
    }

    fn list_task_runs_for_run(&self, run_id: &RunId) -> Result<Vec<TaskRun>, StorageError> {
        let conn = self.conn.blocking_lock();
        let mut stmt = conn
            .prepare("SELECT document FROM task_runs WHERE run_id = ?1 ORDER BY task_name")
            .map_err(backend_err)?;
        let rows = stmt
            .query_map(params![run_id.as_str()], decode_task_run)
            .map_err(backend_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(backend_err)
    }

    fn next_run_number(&self, def_name: &str) -> Result<u64, StorageError> {
        let conn = self.conn.blocking_lock();
        conn.execute(
            "INSERT INTO run_counters (def_name, n) VALUES (?1, 1)
             ON CONFLICT(def_name) DO UPDATE SET n = n + 1",
            params![def_name],
        )
        .map_err(backend_err)?;
        let n: i64 = conn
            .query_row(
                "SELECT n FROM run_counters WHERE def_name = ?1",
                params![def_name],
                |row| row.get(0),
            )
            .map_err(backend_err)?;
        Ok(n as u64)
    }

    fn enqueue(&self, entry: QueueEntry) -> Result<(), StorageError> {
        let conn = self.conn.blocking_lock();
        conn.execute(
            "INSERT INTO queue_entries (run_id, token, due_at_ms, lease_until_ms, claimed_by)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                entry.run_id.as_str(),
                entry.token.0,
                entry.due_at_ms,
                entry.lease_until_ms,
                entry.claimed_by
            ],
        )
        .map_err(backend_err)?;
        Ok(())
    }

    fn scan_ready(&self, now_ms: i64, limit: usize) -> Result<Vec<QueueEntry>, StorageError> {
        let conn = self.conn.blocking_lock();
        let mut stmt = conn
            .prepare(
                "SELECT run_id, token, due_at_ms, lease_until_ms, claimed_by
                 FROM queue_entries
                 WHERE due_at_ms <= ?1 AND (lease_until_ms IS NULL OR lease_until_ms <= ?1)
                 ORDER BY due_at_ms ASC LIMIT ?2",
            )
            .map_err(backend_err)?;
        let rows = stmt
            .query_map(params![now_ms, limit as i64], |row| {
                Ok(QueueEntry {
                    run_id: RunId::from_validated(row.get::<_, String>(0)?),
                    token: ClaimToken(row.get(1)?),
                    due_at_ms: row.get(2)?,
                    lease_until_ms: row.get(3)?,
                    claimed_by: row.get(4)?,
                })
            })
            .map_err(backend_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(backend_err)
    }

    fn claim(
        &self,
        run_id: &RunId,
        token: &ClaimToken,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<(), StorageError> {
        let conn = self.conn.blocking_lock();
        // The UPDATE is the guard: it only commits when the current row is
        // unclaimed (lease is `NULL`) or already owned by `token`. Expired
        // leases are reclaimed by `recover_expired_leases`, never stolen here —
        // otherwise short leases would be stealable by a slower worker.
        let updated = conn.execute(
            "UPDATE queue_entries SET token = ?2, claimed_by = ?3, lease_until_ms = ?4
             WHERE run_id = ?1
               AND (lease_until_ms IS NULL OR token = ?2)",
            params![run_id.as_str(), token.0, "dispatcher", now_ms + lease_ms],
        )
        .map_err(backend_err)?;
        if updated == 0 {
            return Err(StorageError::ClaimLost(format!("run {run_id}")));
        }
        Ok(())
    }

    fn ack(&self, run_id: &RunId, token: &ClaimToken) -> Result<(), StorageError> {
        self.claim_guard_op(run_id, token, "DELETE FROM queue_entries WHERE run_id = ?1 AND token = ?2")
    }

    fn release(
        &self,
        run_id: &RunId,
        token: &ClaimToken,
        retry_at_ms: i64,
    ) -> Result<(), StorageError> {
        let conn = self.conn.blocking_lock();
        let updated = conn.execute(
            "UPDATE queue_entries SET token = '', claimed_by = NULL, lease_until_ms = NULL, due_at_ms = ?3
             WHERE run_id = ?1 AND token = ?2",
            params![run_id.as_str(), token.0, retry_at_ms],
        )
        .map_err(backend_err)?;
        if updated == 0 {
            return Err(StorageError::ClaimLost(format!("run {run_id}")));
        }
        Ok(())
    }

    fn failclaim(&self, run_id: &RunId, token: &ClaimToken) -> Result<(), StorageError> {
        let conn = self.conn.blocking_lock();
        let updated = conn.execute(
            "UPDATE queue_entries SET token = '', claimed_by = NULL, lease_until_ms = NULL
             WHERE run_id = ?1 AND token = ?2",
            params![run_id.as_str(), token.0],
        )
        .map_err(backend_err)?;
        if updated == 0 {
            return Err(StorageError::ClaimLost(format!("run {run_id}")));
        }
        Ok(())
    }

    fn recover_expired_leases(&self, now_ms: i64) -> Result<usize, StorageError> {
        let conn = self.conn.blocking_lock();
        let changed = conn
            .execute(
                "UPDATE queue_entries SET token = '', claimed_by = NULL, lease_until_ms = NULL
                 WHERE lease_until_ms IS NOT NULL AND lease_until_ms <= ?1",
                params![now_ms],
            )
            .map_err(backend_err)?;
        Ok(changed)
    }

    fn len_queue(&self) -> usize {
        // Best-effort gauge: unused unless the connection responds quickly.
        let conn = match self.conn.try_lock() {
            Ok(conn) => conn,
            Err(_) => return 0,
        };
        conn.query_row("SELECT COUNT(*) FROM queue_entries", [], |row| row.get(0))
            .unwrap_or(0)
    }
}

impl SqliteStore {
    fn claim_guard_op(
        &self,
        run_id: &RunId,
        token: &ClaimToken,
        sql: &str,
    ) -> Result<(), StorageError> {
        let conn = self.conn.blocking_lock();
        let updated = conn
            .execute(sql, params![run_id.as_str(), token.0])
            .map_err(backend_err)?;
        if updated == 0 {
            return Err(StorageError::ClaimLost(format!("run {run_id}")));
        }
        Ok(())
    }
}