//! SQLite database builtins — connection pooling, prepared statements,
//! transactions/savepoints, migrations, and maintenance.
//!
//! Every `db_*` builtin resolves its connection through the interpreter's
//! per-path pool (`Interpreter::db_conn`, in `mod.rs`) rather than opening a
//! fresh connection per call — a fresh connection per call was the previous
//! behavior, and meant no PRAGMA configuration ever took effect (each
//! connection lived for exactly one query) and no statement caching was
//! possible. The pool is what makes WAL mode, `busy_timeout`, and prepared-
//! statement reuse actually work.

use super::builtins_base64::base64_encode;
use super::Signal;
use crate::value::Value;
use std::collections::HashMap;

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn gx_to_sqlite(val: &Value) -> rusqlite::types::ToSqlOutput<'static> {
    use rusqlite::types::{ToSqlOutput, Value as SqlValue};
    match val {
        Value::Null => ToSqlOutput::Owned(SqlValue::Null),
        Value::Bool(b) => ToSqlOutput::Owned(SqlValue::Integer(*b as i64)),
        Value::Number(n) => {
            if n.fract() == 0.0 {
                ToSqlOutput::Owned(SqlValue::Integer(*n as i64))
            } else {
                ToSqlOutput::Owned(SqlValue::Real(*n))
            }
        }
        Value::Str(s) => ToSqlOutput::Owned(SqlValue::Text(s.clone())),
        other => ToSqlOutput::Owned(SqlValue::Text(other.to_string())),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn sqlite_to_gx(val: rusqlite::types::ValueRef<'_>) -> Value {
    use rusqlite::types::ValueRef;
    match val {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Number(i as f64),
        ValueRef::Real(f) => Value::Number(f),
        ValueRef::Text(s) => Value::Str(String::from_utf8_lossy(s).to_string()),
        // GX has no native binary value type. Round-tripping a blob through
        // base64 TEXT (db_exec(db, "...", [base64_decode-free — see docs])
        // is the documented, supported pattern for binary data — adding a
        // new Value::Blob variant would ripple through every other match
        // over Value in the interpreter for a pattern no real GX
        // application (including GClaw, whose binary payloads all flow
        // through base64-encoded HTTP fields, never a blob column) actually
        // needs today.
        ValueRef::Blob(b) => Value::Str(base64_encode(b)),
    }
}

/// Apply production defaults to a freshly opened connection. Called exactly
/// once, right after `Connection::open`, before the connection enters the
/// pool — every subsequent use (query, exec, transaction) shares these
/// settings for the life of the pooled connection.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn configure_connection(conn: &rusqlite::Connection) -> Result<(), Signal> {
    // WAL: readers no longer block behind a writer (and vice versa) the way
    // the default rollback-journal mode does — the single biggest lever
    // against "database is locked" under the concurrent access pattern
    // GX's own HTTP server worker pool (see bridge_impl.rs) now produces
    // whenever multiple routes touch the same database file.
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| Signal::Error(format!("failed to enable WAL mode: {}", e)))?;
    // Without this, a connection that finds the database locked returns
    // SQLITE_BUSY immediately instead of waiting — with WAL mode this
    // mostly matters for writer-vs-writer contention, which WAL doesn't
    // eliminate on its own.
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| Signal::Error(format!("failed to set busy_timeout: {}", e)))?;
    // SQLite defaults this OFF for backward compatibility with pre-3.6.19
    // behavior; ON is the correct default for a new application unless it
    // deliberately wants to allow orphaned rows.
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| Signal::Error(format!("failed to enable foreign_keys: {}", e)))?;
    Ok(())
}

/// Execute an INSERT/UPDATE/DELETE on a pooled connection, using a cached
/// prepared statement — free now that the connection itself is reused
/// across calls (previously moot: a fresh connection per call meant a
/// fresh statement cache per call too).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn db_exec_on_conn(
    conn: &rusqlite::Connection,
    sql: &str,
    params: Vec<Value>,
) -> Result<Value, Signal> {
    let sql_params: Vec<rusqlite::types::ToSqlOutput<'_>> =
        params.iter().map(gx_to_sqlite).collect();
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|e| Signal::Error(format!("db_exec: SQL error: {}", e)))?;
    let affected = stmt
        .execute(rusqlite::params_from_iter(sql_params.iter()))
        .map_err(|e| Signal::Error(format!("db_exec: SQL error: {}", e)))?;
    Ok(Value::Number(affected as f64))
}

/// Execute a SELECT on a pooled connection, using a cached prepared
/// statement.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn db_query_on_conn(
    conn: &rusqlite::Connection,
    sql: &str,
    params: Vec<Value>,
) -> Result<Value, Signal> {
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|e| Signal::Error(format!("db_query: SQL error: {}", e)))?;
    let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let sql_params: Vec<rusqlite::types::ToSqlOutput<'_>> =
        params.iter().map(gx_to_sqlite).collect();
    let mut rows_iter = stmt
        .query(rusqlite::params_from_iter(sql_params.iter()))
        .map_err(|e| Signal::Error(format!("db_query: query error: {}", e)))?;
    let mut rows: Vec<Value> = Vec::new();
    loop {
        match rows_iter.next() {
            Ok(Some(row)) => {
                let mut map = HashMap::new();
                for (i, col) in col_names.iter().enumerate() {
                    let v = sqlite_to_gx(row.get_ref(i).unwrap_or(rusqlite::types::ValueRef::Null));
                    map.insert(col.clone(), v);
                }
                rows.push(Value::Object(map));
            }
            Ok(None) => break,
            Err(e) => return Err(Signal::Error(format!("db_query: row error: {}", e))),
        }
    }
    Ok(Value::Array(rows))
}

/// Apply pending migrations, tracked via SQLite's own `PRAGMA user_version`
/// — an integer built into every database file for exactly this purpose,
/// so there's no separate bookkeeping table to create or get out of sync.
/// `migrations[i]` is applied when `user_version <= i`; each migration runs
/// in its own transaction, so a failure partway through leaves the database
/// at the last successfully applied version, not partially migrated.
/// Idempotent: re-running with the same (or a prefix of the same) list is a
/// no-op once `user_version >= migrations.len()`.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn db_migrate_on_conn(
    conn: &rusqlite::Connection,
    migrations: &[String],
) -> Result<Value, Signal> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| Signal::Error(format!("db_migrate: cannot read user_version: {}", e)))?;
    let from_version = current;
    let mut applied = 0i64;
    let mut version = current;
    for (i, migration_sql) in migrations.iter().enumerate() {
        let target = (i as i64) + 1;
        if target <= current {
            continue;
        }
        conn.execute_batch("BEGIN")
            .map_err(|e| Signal::Error(format!("db_migrate: BEGIN failed: {}", e)))?;
        let result = conn.execute_batch(migration_sql);
        match result {
            Ok(()) => {
                // PRAGMA user_version can't be parameter-bound; `target` is
                // this function's own loop counter, never user/script input.
                let set = conn.execute_batch(&format!("PRAGMA user_version = {}", target));
                match set {
                    Ok(()) => match conn.execute_batch("COMMIT") {
                        Ok(()) => {
                            version = target;
                            applied += 1;
                        }
                        Err(e) => {
                            let _ = conn.execute_batch("ROLLBACK");
                            return Err(Signal::Error(format!(
                                "db_migrate: migration {} committed data but failed to record \
                                 the new version ({}); database left at version {}",
                                target, e, current
                            )));
                        }
                    },
                    Err(e) => {
                        let _ = conn.execute_batch("ROLLBACK");
                        return Err(Signal::Error(format!(
                            "db_migrate: migration {} failed to record its version ({}); rolled back",
                            target, e
                        )));
                    }
                }
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(Signal::Error(format!(
                    "db_migrate: migration {} failed and was rolled back: {}. Database left at version {}.",
                    target, e, version
                )));
            }
        }
    }
    let mut result = HashMap::new();
    result.insert("ok".into(), Value::Bool(true));
    result.insert("from_version".into(), Value::Number(from_version as f64));
    result.insert("to_version".into(), Value::Number(version as f64));
    result.insert("applied".into(), Value::Number(applied as f64));
    Ok(Value::Object(result))
}

/// `PRAGMA integrity_check` — returns `{ok: true}` when SQLite reports "ok",
/// or `{ok: false, errors: [...]}` with every reported problem otherwise.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn db_integrity_check_on_conn(conn: &rusqlite::Connection) -> Result<Value, Signal> {
    let mut stmt = conn
        .prepare("PRAGMA integrity_check")
        .map_err(|e| Signal::Error(format!("db_integrity_check: {}", e)))?;
    let rows: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| Signal::Error(format!("db_integrity_check: {}", e)))?
        .filter_map(|r| r.ok())
        .collect();
    let mut result = HashMap::new();
    let ok = rows.len() == 1 && rows[0] == "ok";
    result.insert("ok".into(), Value::Bool(ok));
    result.insert(
        "errors".into(),
        Value::Array(if ok {
            Vec::new()
        } else {
            rows.into_iter().map(Value::Str).collect()
        }),
    );
    Ok(Value::Object(result))
}

/// `VACUUM` — rebuilds the database file, reclaiming space from deleted
/// rows. Requires no other transaction to be active on this connection;
/// callers (see mod.rs) reject this when called from inside `db_transaction`.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn db_vacuum_on_conn(conn: &rusqlite::Connection) -> Result<Value, Signal> {
    conn.execute_batch("VACUUM")
        .map_err(|e| Signal::Error(format!("db_vacuum: {}", e)))?;
    let mut result = HashMap::new();
    result.insert("ok".into(), Value::Bool(true));
    Ok(Value::Object(result))
}

/// Online backup via SQLite's own backup API (`rusqlite`'s `backup`
/// feature) — safe to run against a database that's concurrently being
/// written by another connection, unlike a plain file copy (which can copy
/// a torn/inconsistent snapshot, or miss data still sitting in a WAL file
/// that hasn't been checkpointed into the main database file yet).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn db_backup_impl(
    source: &rusqlite::Connection,
    dest_path: &str,
) -> Result<Value, Signal> {
    let mut dest = rusqlite::Connection::open(dest_path).map_err(|e| {
        Signal::Error(format!(
            "db_backup: cannot open destination '{}': {}",
            dest_path, e
        ))
    })?;
    let backup = rusqlite::backup::Backup::new(source, &mut dest)
        .map_err(|e| Signal::Error(format!("db_backup: {}", e)))?;
    backup
        .run_to_completion(5, std::time::Duration::from_millis(250), None)
        .map_err(|e| Signal::Error(format!("db_backup: {}", e)))?;
    let mut result = HashMap::new();
    result.insert("ok".into(), Value::Bool(true));
    result.insert("path".into(), Value::Str(dest_path.to_string()));
    Ok(Value::Object(result))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A fresh, uniquely-named temp db path per test — cargo test runs
    /// tests in parallel, so a shared fixed path would race.
    fn temp_db_path(label: &str) -> String {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!(
                "gx_builtins_db_test_{}_{}_{}.db",
                label,
                std::process::id(),
                n
            ))
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn configure_connection_enables_wal_busy_timeout_and_foreign_keys() {
        let path = temp_db_path("configure");
        let conn = rusqlite::Connection::open(&path).unwrap();
        configure_connection(&conn).unwrap();

        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let foreign_keys: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path));
        let _ = std::fs::remove_file(format!("{}-shm", path));
    }

    #[test]
    fn db_exec_on_conn_and_db_query_on_conn_round_trip_params() {
        let path = temp_db_path("roundtrip");
        let conn = rusqlite::Connection::open(&path).unwrap();
        configure_connection(&conn).unwrap();

        db_exec_on_conn(
            &conn,
            "CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT, active INTEGER)",
            vec![],
        )
        .unwrap();
        db_exec_on_conn(
            &conn,
            "INSERT INTO t(name, active) VALUES (?, ?)",
            vec![Value::Str("alice".into()), Value::Bool(true)],
        )
        .unwrap();

        let rows = db_query_on_conn(&conn, "SELECT name, active FROM t", vec![]).unwrap();
        match rows {
            Value::Array(items) => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    Value::Object(m) => {
                        assert_eq!(m.get("name"), Some(&Value::Str("alice".into())));
                        assert_eq!(m.get("active"), Some(&Value::Number(1.0)));
                    }
                    other => panic!("expected object row, got {:?}", other),
                }
            }
            other => panic!("expected array, got {:?}", other),
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn db_migrate_is_idempotent_and_applies_only_new_migrations() {
        let path = temp_db_path("migrate");
        let conn = rusqlite::Connection::open(&path).unwrap();
        configure_connection(&conn).unwrap();

        let m1 = vec!["CREATE TABLE a(id INTEGER PRIMARY KEY)".to_string()];
        let r1 = db_migrate_on_conn(&conn, &m1).unwrap();
        let Value::Object(o1) = r1 else { panic!() };
        assert_eq!(o1.get("applied"), Some(&Value::Number(1.0)));
        assert_eq!(o1.get("to_version"), Some(&Value::Number(1.0)));

        // Re-running the exact same list is a no-op.
        let r2 = db_migrate_on_conn(&conn, &m1).unwrap();
        let Value::Object(o2) = r2 else { panic!() };
        assert_eq!(o2.get("applied"), Some(&Value::Number(0.0)));

        // Extending the list applies only the new entry.
        let m2 = vec![
            "CREATE TABLE a(id INTEGER PRIMARY KEY)".to_string(),
            "CREATE TABLE b(id INTEGER PRIMARY KEY)".to_string(),
        ];
        let r3 = db_migrate_on_conn(&conn, &m2).unwrap();
        let Value::Object(o3) = r3 else { panic!() };
        assert_eq!(o3.get("applied"), Some(&Value::Number(1.0)));
        assert_eq!(o3.get("to_version"), Some(&Value::Number(2.0)));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn db_migrate_leaves_the_database_at_the_last_successful_version_on_failure() {
        let path = temp_db_path("migrate_fail");
        let conn = rusqlite::Connection::open(&path).unwrap();
        configure_connection(&conn).unwrap();

        let migrations = vec![
            "CREATE TABLE a(id INTEGER PRIMARY KEY)".to_string(),
            "THIS IS NOT VALID SQL".to_string(),
        ];
        let result = db_migrate_on_conn(&conn, &migrations);
        assert!(result.is_err());

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            version, 1,
            "the first migration must still be recorded as applied"
        );

        // table 'a' from migration 1 must exist even though migration 2 failed.
        let exists: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn db_integrity_check_reports_ok_for_a_healthy_database() {
        let path = temp_db_path("integrity");
        let conn = rusqlite::Connection::open(&path).unwrap();
        configure_connection(&conn).unwrap();
        db_exec_on_conn(&conn, "CREATE TABLE t(id INTEGER PRIMARY KEY)", vec![]).unwrap();

        let result = db_integrity_check_on_conn(&conn).unwrap();
        let Value::Object(o) = result else { panic!() };
        assert_eq!(o.get("ok"), Some(&Value::Bool(true)));
        assert_eq!(o.get("errors"), Some(&Value::Array(vec![])));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn db_vacuum_succeeds_on_a_healthy_database() {
        let path = temp_db_path("vacuum");
        let conn = rusqlite::Connection::open(&path).unwrap();
        configure_connection(&conn).unwrap();
        db_exec_on_conn(&conn, "CREATE TABLE t(id INTEGER PRIMARY KEY)", vec![]).unwrap();
        db_exec_on_conn(&conn, "DELETE FROM t", vec![]).unwrap();

        let result = db_vacuum_on_conn(&conn).unwrap();
        let Value::Object(o) = result else { panic!() };
        assert_eq!(o.get("ok"), Some(&Value::Bool(true)));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn db_backup_copies_schema_and_data_to_a_fresh_file() {
        let src_path = temp_db_path("backup_src");
        let dest_path = temp_db_path("backup_dest");
        let conn = rusqlite::Connection::open(&src_path).unwrap();
        configure_connection(&conn).unwrap();
        db_exec_on_conn(
            &conn,
            "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT)",
            vec![],
        )
        .unwrap();
        db_exec_on_conn(
            &conn,
            "INSERT INTO t(v) VALUES (?)",
            vec![Value::Str("hello".into())],
        )
        .unwrap();

        let result = db_backup_impl(&conn, &dest_path).unwrap();
        let Value::Object(o) = result else { panic!() };
        assert_eq!(o.get("ok"), Some(&Value::Bool(true)));

        let dest_conn = rusqlite::Connection::open(&dest_path).unwrap();
        let rows = db_query_on_conn(&dest_conn, "SELECT v FROM t", vec![]).unwrap();
        match rows {
            Value::Array(items) => assert_eq!(items.len(), 1),
            other => panic!("expected array, got {:?}", other),
        }

        let _ = std::fs::remove_file(&src_path);
        let _ = std::fs::remove_file(&dest_path);
    }
}
