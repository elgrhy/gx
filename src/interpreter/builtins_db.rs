//! SQLite database builtins.

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
        ValueRef::Blob(b) => Value::Str(base64_encode(b)),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn db_query_impl(path: &str, sql: &str, params: Vec<Value>) -> Result<Value, Signal> {
    let conn = rusqlite::Connection::open(path)
        .map_err(|e| Signal::Error(format!("db_query: cannot open '{}': {}", path, e)))?;
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| Signal::Error(format!("db_query: SQL error: {}", e)))?;
    let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let sql_params: Vec<rusqlite::types::ToSqlOutput<'_>> =
        params.iter().map(gx_to_sqlite).collect();
    let rows_iter = stmt
        .query(rusqlite::params_from_iter(sql_params.iter()))
        .map_err(|e| Signal::Error(format!("db_query: query error: {}", e)))?;
    let mut rows_iter = rows_iter;
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

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn db_exec_impl(path: &str, sql: &str, params: Vec<Value>) -> Result<Value, Signal> {
    let conn = rusqlite::Connection::open(path)
        .map_err(|e| Signal::Error(format!("db_exec: cannot open '{}': {}", path, e)))?;
    db_exec_on_conn(&conn, sql, params)
}

/// Execute an INSERT/UPDATE/DELETE on an already-open connection (used inside db_transaction).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn db_exec_on_conn(
    conn: &rusqlite::Connection,
    sql: &str,
    params: Vec<Value>,
) -> Result<Value, Signal> {
    let sql_params: Vec<rusqlite::types::ToSqlOutput<'_>> =
        params.iter().map(gx_to_sqlite).collect();
    let affected = conn
        .execute(sql, rusqlite::params_from_iter(sql_params.iter()))
        .map_err(|e| Signal::Error(format!("db_exec: SQL error: {}", e)))?;
    Ok(Value::Number(affected as f64))
}

/// Execute a SELECT on an already-open connection (used inside db_transaction).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn db_query_on_conn(
    conn: &rusqlite::Connection,
    sql: &str,
    params: Vec<Value>,
) -> Result<Value, Signal> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| Signal::Error(format!("db_query: SQL error: {}", e)))?;
    let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let sql_params: Vec<rusqlite::types::ToSqlOutput<'_>> =
        params.iter().map(gx_to_sqlite).collect();
    let rows_iter = stmt
        .query(rusqlite::params_from_iter(sql_params.iter()))
        .map_err(|e| Signal::Error(format!("db_query: query error: {}", e)))?;
    let mut rows_iter = rows_iter;
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
