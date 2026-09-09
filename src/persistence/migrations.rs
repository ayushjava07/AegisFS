//! SQLite schema, maintained as committed, forward-only migrations.
//!
//! Unlike a build-time migration runner, the platform executes migrations
//! lazily at connection open and stamps the resulting schema version in
//! `meta`. Adding a migration means appending to [`MIGRATIONS`] and bumping
//! the reader/writer code in [super::sqlite] in the same change — there is
//! never a "migrate everything at server start" step, so a fresh checkout
//! which never opens a database file does not need one.

use rusqlite::Connection;

/// One schema migration.
pub struct Migration {
    /// Migration number, strictly increasing.
    pub version: u32,
    /// Short description for `meta.schema_version` and logs.
    pub name: &'static str,
    /// Ordered DDL statements executed in the migration.
    pub statements: &'static [&'static str],
}

/// The complete migration chain, in order. Never edit statements of a
/// migration that has shipped; append a new one instead.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial schema",
        statements: &[
            // Definition store (natural key tenant+name).
            "CREATE TABLE IF NOT EXISTS workflows (
                tenant        TEXT NOT NULL,
                name          TEXT NOT NULL,
                version       INTEGER NOT NULL,
                spec_version  INTEGER NOT NULL,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                document      TEXT NOT NULL,
                PRIMARY KEY (tenant, name)
            )",
            // Run records, with denormalized filter columns.
            "CREATE TABLE IF NOT EXISTS runs (
                id                TEXT PRIMARY KEY,
                tenant            TEXT NOT NULL,
                def_name          TEXT NOT NULL,
                def_version       INTEGER NOT NULL,
                status            TEXT NOT NULL,
                attempts          INTEGER NOT NULL,
                next_attempt_at_ms INTEGER,
                created_at_ms     INTEGER NOT NULL,
                run_number        INTEGER NOT NULL,
                document          TEXT NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS idx_runs_tenant ON runs (tenant)",
            "CREATE INDEX IF NOT EXISTS idx_runs_status ON runs (status)",
            "CREATE INDEX IF NOT EXISTS idx_runs_def ON runs (tenant, def_name)",
            "CREATE INDEX IF NOT EXISTS idx_runs_created ON runs (created_at_ms)",
            // Task-run records.
            "CREATE TABLE IF NOT EXISTS task_runs (
                id         TEXT PRIMARY KEY,
                run_id     TEXT NOT NULL,
                task_name  TEXT NOT NULL,
                status     TEXT NOT NULL,
                document   TEXT NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS idx_task_runs_run ON task_runs (run_id)",
            // Run queue.
            "CREATE TABLE IF NOT EXISTS queue_entries (
                run_id         TEXT PRIMARY KEY,
                token          TEXT NOT NULL,
                due_at_ms      INTEGER NOT NULL,
                lease_until_ms INTEGER,
                claimed_by     TEXT
            )",
            "CREATE INDEX IF NOT EXISTS idx_queue_due ON queue_entries (due_at_ms)",
            "CREATE INDEX IF NOT EXISTS idx_queue_lease ON queue_entries (lease_until_ms)",
            // Schema bookkeeping.
            "CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
        ],
    },
    Migration {
        version: 2,
        name: "per-definition run counters",
        statements: &[
            // Monotonic submission counters, incremented transactionally.
            "CREATE TABLE IF NOT EXISTS run_counters (
                def_name TEXT PRIMARY KEY,
                n        INTEGER NOT NULL
            )",
        ],
    },
];

/// Highest migration number in the chain.
pub const fn latest_version() -> u32 {
    if MIGRATIONS.is_empty() {
        0
    } else {
        MIGRATIONS[MIGRATIONS.len() - 1].version
    }
}

/// Applies all pending migrations to `conn`, stamping the version in `meta`.
///
/// Idempotent: re-running on an already-migrated database is a no-op for any
/// entity DDL (all statements are `IF NOT EXISTS`) and only refreshes `meta`.
pub fn apply_migrations(conn: &Connection) -> rusqlite::Result<()> {
    for migration in MIGRATIONS {
        for statement in migration.statements {
            conn.execute_batch(statement)?;
        }
    }
    let version = latest_version();
    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [version.to_string()],
    )?;
    Ok(())
}

/// Reads the schema version a connection last migrated to.
pub fn schema_version(conn: &Connection) -> rusqlite::Result<u32> {
    let stored: String = conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
    )?;
    Ok(stored.parse().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn migrations_are_idempotent() {
        let c = conn();
        apply_migrations(&c).unwrap();
        apply_migrations(&c).unwrap();
        assert_eq!(schema_version(&c).unwrap(), latest_version());
    }

    #[test]
    fn migration_versions_strictly_increase() {
        let mut last = 0u32;
        for m in MIGRATIONS {
            assert!(m.version > last && m.version == last + 1, "version {} is gapless", m.version);
            assert!(!m.name.is_empty());
            last = m.version;
        }
        assert_eq!(latest_version(), last);
    }

    #[test]
    fn queue_and_runs_tables_exist() {
        let c = conn();
        apply_migrations(&c).unwrap();
        let names: Vec<String> = c
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for table in ["workflows", "runs", "task_runs", "queue_entries", "meta"] {
            assert!(names.iter().any(|n| n == table), "missing table {table}");
        }
    }
}