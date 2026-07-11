//! Native CLI backing for `tswift.db.*` (see
//! `tswift_swiftdata::db` for the wire schema/tagged-value codec, and
//! `docs/adr/0015-db-host-service-wire.md` for the full contract).
//!
//! Backed by the system SQLite through the tiny hand-written FFI in
//! `sqlite_ffi.rs` — real SQL, real file-backed databases (or `:memory:`),
//! exactly like Foundation's `FileManager` in `fs.rs` backs the real
//! filesystem.
//!
//! ## Handle lifecycle
//!
//! `open` mints an ascending `i64` handle and stores the `Connection` in a
//! process-wide table behind one `Mutex` (so this handler stays `Sync`
//! without needing per-connection interior mutability tricks — the
//! interpreter only ever calls it from one thread at a time anyway,
//! ADR-0005). `close` removes and drops the entry, finalizing the
//! connection. Any operation against a handle that was never opened or was
//! already closed is a structured `$thrown` error, not a panic or a silent
//! no-op — the same "double-close/invalid handle" guard `HostError`-style
//! APIs elsewhere in this crate use.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use tswift_core::json::{self, Json};
use tswift_core::HostCallHandler;
use tswift_swiftdata::db::{self, DbValue, ExecResult};

use crate::sqlite_ffi::{ColumnValue, Connection, SqliteError, Step};

pub struct DbHandler {
    next_handle: AtomicI64,
    conns: Mutex<HashMap<i64, Connection>>,
}

impl DbHandler {
    pub fn new() -> Self {
        Self {
            next_handle: AtomicI64::new(1),
            conns: Mutex::new(HashMap::new()),
        }
    }

    fn thrown(message: impl Into<String>) -> String {
        json::to_string(&Json::Object(vec![(
            "$thrown".to_string(),
            Json::Str(message.into()),
        )]))
    }

    fn thrown_sqlite(err: &SqliteError) -> String {
        Self::thrown(format!("SQLite error {}: {}", err.code, err.message))
    }
}

impl Default for DbHandler {
    fn default() -> Self {
        Self::new()
    }
}

fn to_column_value(v: &DbValue) -> ColumnValue {
    match v {
        DbValue::Null => ColumnValue::Null,
        DbValue::Int(i) => ColumnValue::Int(*i),
        DbValue::Real(d) => ColumnValue::Real(*d),
        DbValue::Text(s) => ColumnValue::Text(s.clone()),
        DbValue::Blob(b) => ColumnValue::Blob(b.clone()),
    }
}

fn to_db_value(v: ColumnValue) -> DbValue {
    match v {
        ColumnValue::Null => DbValue::Null,
        ColumnValue::Int(i) => DbValue::Int(i),
        ColumnValue::Real(d) => DbValue::Real(d),
        ColumnValue::Text(s) => DbValue::Text(s),
        ColumnValue::Blob(b) => DbValue::Blob(b),
    }
}

impl HostCallHandler for DbHandler {
    fn call(&self, name: &str, args_json: &str) -> Result<String, String> {
        let Json::Array(args) = json::parse(args_json).map_err(|e| e.to_string())? else {
            return Err(format!("{name}: expected an args array"));
        };
        let str_arg = |i: usize| -> Result<String, String> {
            match args.get(i) {
                Some(Json::Str(s)) => Ok(s.clone()),
                _ => Err(format!(
                    "{name}: expected a String argument at position {i}"
                )),
            }
        };
        let handle_arg = |i: usize| -> Result<i64, String> {
            match args.get(i) {
                Some(Json::Int(n)) => Ok(*n),
                _ => Err(format!(
                    "{name}: expected an Int handle argument at position {i}"
                )),
            }
        };

        match name {
            db::OP_OPEN => {
                let path = str_arg(0)?;
                match Connection::open(&path) {
                    Ok(conn) => {
                        let handle = self.next_handle.fetch_add(1, Ordering::SeqCst);
                        self.conns
                            .lock()
                            .map_err(|_| "db handle table poisoned".to_string())?
                            .insert(handle, conn);
                        Ok(json::to_string(&Json::Int(handle)))
                    }
                    Err(e) => Ok(Self::thrown_sqlite(&e)),
                }
            }
            db::OP_CLOSE => {
                let handle = handle_arg(0)?;
                let mut conns = self
                    .conns
                    .lock()
                    .map_err(|_| "db handle table poisoned".to_string())?;
                match conns.remove(&handle) {
                    Some(_conn) => Ok("null".to_string()), // dropped here: connection closes.
                    None => Ok(Self::thrown(format!(
                        "tswift.db.close: handle {handle} is not open (already closed, or never opened)"
                    ))),
                }
            }
            db::OP_EXECUTE => {
                let handle = handle_arg(0)?;
                let sql = str_arg(1)?;
                let params_text = str_arg(2)?;
                let params = match db::decode_params(&params_text) {
                    Ok(p) => p,
                    // A malformed params payload is a data error the Swift
                    // caller can `catch`, not a bridge-level `Err`.
                    Err(e) => return Ok(Self::thrown(format!("{name}: {e}"))),
                };
                let conns = self
                    .conns
                    .lock()
                    .map_err(|_| "db handle table poisoned".to_string())?;
                let Some(conn) = conns.get(&handle) else {
                    return Ok(Self::thrown(format!(
                        "tswift.db.execute: handle {handle} is not open"
                    )));
                };
                let stmt = match conn.prepare(&sql) {
                    Ok(s) => s,
                    Err(e) => return Ok(Self::thrown_sqlite(&e)),
                };
                for (i, value) in params.iter().enumerate() {
                    if let Err(e) = stmt.bind((i + 1) as i32, &to_column_value(value)) {
                        return Ok(Self::thrown_sqlite(&e));
                    }
                }
                if let Err(e) = stmt.step_to_completion() {
                    return Ok(Self::thrown_sqlite(&e));
                }
                let result = ExecResult {
                    rows_affected: conn.changes(),
                    last_insert_rowid: conn.last_insert_rowid(),
                };
                Ok(json::to_string(&Json::Str(result.encode())))
            }
            db::OP_QUERY => {
                let handle = handle_arg(0)?;
                let sql = str_arg(1)?;
                let params_text = str_arg(2)?;
                let params = match db::decode_params(&params_text) {
                    Ok(p) => p,
                    // A malformed params payload is a data error the Swift
                    // caller can `catch`, not a bridge-level `Err`.
                    Err(e) => return Ok(Self::thrown(format!("{name}: {e}"))),
                };
                let conns = self
                    .conns
                    .lock()
                    .map_err(|_| "db handle table poisoned".to_string())?;
                let Some(conn) = conns.get(&handle) else {
                    return Ok(Self::thrown(format!(
                        "tswift.db.query: handle {handle} is not open"
                    )));
                };
                let stmt = match conn.prepare(&sql) {
                    Ok(s) => s,
                    Err(e) => return Ok(Self::thrown_sqlite(&e)),
                };
                for (i, value) in params.iter().enumerate() {
                    if let Err(e) = stmt.bind((i + 1) as i32, &to_column_value(value)) {
                        return Ok(Self::thrown_sqlite(&e));
                    }
                }
                let col_count = stmt.column_count();
                let col_names: Vec<String> = (0..col_count).map(|i| stmt.column_name(i)).collect();
                let mut rows = Vec::new();
                loop {
                    match stmt.step() {
                        Ok(Step::Row) => {
                            let mut row = Vec::with_capacity(col_count);
                            for (i, col_name) in col_names.iter().enumerate() {
                                match stmt.column_value(i) {
                                    Ok(v) => row.push((col_name.clone(), to_db_value(v))),
                                    Err(e) => return Ok(Self::thrown_sqlite(&e)),
                                }
                            }
                            rows.push(row);
                        }
                        Ok(Step::Done) => break,
                        Err(e) => return Ok(Self::thrown_sqlite(&e)),
                    }
                }
                Ok(json::to_string(&Json::Str(db::encode_rows(&rows))))
            }
            db::OP_BEGIN => run_control(self, handle_arg(0)?, "BEGIN"),
            db::OP_COMMIT => run_control(self, handle_arg(0)?, "COMMIT"),
            db::OP_ROLLBACK => run_control(self, handle_arg(0)?, "ROLLBACK"),
            other => Err(format!("unknown host fn `{other}`")),
        }
    }
}

fn run_control(handler: &DbHandler, handle: i64, sql: &str) -> Result<String, String> {
    let conns = handler
        .conns
        .lock()
        .map_err(|_| "db handle table poisoned".to_string())?;
    let Some(conn) = conns.get(&handle) else {
        return Ok(DbHandler::thrown(format!(
            "tswift.db.{}: handle {handle} is not open",
            sql.to_ascii_lowercase()
        )));
    };
    match conn.exec_simple(sql) {
        Ok(()) => Ok("null".to_string()),
        Err(e) => Ok(DbHandler::thrown_sqlite(&e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_memory(handler: &DbHandler) -> i64 {
        let args = json::to_string(&Json::Array(vec![Json::Str(":memory:".to_string())]));
        let reply = handler.call(db::OP_OPEN, &args).unwrap();
        let Json::Int(handle) = json::parse(&reply).unwrap() else {
            panic!("expected Int handle, got {reply}");
        };
        handle
    }

    fn execute(handler: &DbHandler, handle: i64, sql: &str, params: &[DbValue]) -> String {
        let args = json::to_string(&Json::Array(vec![
            Json::Int(handle),
            Json::Str(sql.to_string()),
            Json::Str(db::encode_params(params)),
        ]));
        handler.call(db::OP_EXECUTE, &args).unwrap()
    }

    fn query(handler: &DbHandler, handle: i64, sql: &str, params: &[DbValue]) -> String {
        let args = json::to_string(&Json::Array(vec![
            Json::Int(handle),
            Json::Str(sql.to_string()),
            Json::Str(db::encode_params(params)),
        ]));
        handler.call(db::OP_QUERY, &args).unwrap()
    }

    fn unwrap_json_string(reply: &str) -> String {
        match json::parse(reply).unwrap() {
            Json::Str(s) => s,
            other => panic!("expected JSON string reply, got {other:?}"),
        }
    }

    #[test]
    fn open_execute_query_close_round_trip() {
        let handler = DbHandler::new();
        let handle = open_memory(&handler);
        let create = execute(
            &handler,
            handle,
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
            &[],
        );
        let result = ExecResult::decode(&unwrap_json_string(&create)).unwrap();
        assert_eq!(result.rows_affected, 0);

        let insert = execute(
            &handler,
            handle,
            "INSERT INTO t (name) VALUES (?)",
            &[DbValue::Text("alice".to_string())],
        );
        let result = ExecResult::decode(&unwrap_json_string(&insert)).unwrap();
        assert_eq!(result.rows_affected, 1);
        assert_eq!(result.last_insert_rowid, 1);

        let selected = query(&handler, handle, "SELECT id, name FROM t", &[]);
        let rows = db::decode_rows(&unwrap_json_string(&selected)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], ("id".to_string(), DbValue::Int(1)));
        assert_eq!(
            rows[0][1],
            ("name".to_string(), DbValue::Text("alice".to_string()))
        );

        let close_args = json::to_string(&Json::Array(vec![Json::Int(handle)]));
        assert_eq!(handler.call(db::OP_CLOSE, &close_args).unwrap(), "null");
    }

    #[test]
    fn typed_values_round_trip_including_blob_and_null() {
        let handler = DbHandler::new();
        let handle = open_memory(&handler);
        execute(
            &handler,
            handle,
            "CREATE TABLE t (i INTEGER, r REAL, s TEXT, b BLOB, n TEXT)",
            &[],
        );
        execute(
            &handler,
            handle,
            "INSERT INTO t VALUES (?, ?, ?, ?, ?)",
            &[
                DbValue::Int(7),
                DbValue::Real(2.5),
                DbValue::Text("hi".to_string()),
                DbValue::Blob(vec![9, 8, 7]),
                DbValue::Null,
            ],
        );
        let selected = query(&handler, handle, "SELECT * FROM t", &[]);
        let rows = db::decode_rows(&unwrap_json_string(&selected)).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row[0].1, DbValue::Int(7));
        assert_eq!(row[1].1, DbValue::Real(2.5));
        assert_eq!(row[2].1, DbValue::Text("hi".to_string()));
        assert_eq!(row[3].1, DbValue::Blob(vec![9, 8, 7]));
        assert_eq!(row[4].1, DbValue::Null);
    }

    #[test]
    fn transaction_commit_and_rollback() {
        let handler = DbHandler::new();
        let handle = open_memory(&handler);
        execute(&handler, handle, "CREATE TABLE t (v INTEGER)", &[]);

        let begin_args = json::to_string(&Json::Array(vec![Json::Int(handle)]));
        assert_eq!(handler.call(db::OP_BEGIN, &begin_args).unwrap(), "null");
        execute(&handler, handle, "INSERT INTO t VALUES (1)", &[]);
        assert_eq!(handler.call(db::OP_ROLLBACK, &begin_args).unwrap(), "null");
        let after_rollback = query(&handler, handle, "SELECT COUNT(*) AS c FROM t", &[]);
        let rows = db::decode_rows(&unwrap_json_string(&after_rollback)).unwrap();
        assert_eq!(rows[0][0].1, DbValue::Int(0));

        assert_eq!(handler.call(db::OP_BEGIN, &begin_args).unwrap(), "null");
        execute(&handler, handle, "INSERT INTO t VALUES (1)", &[]);
        assert_eq!(handler.call(db::OP_COMMIT, &begin_args).unwrap(), "null");
        let after_commit = query(&handler, handle, "SELECT COUNT(*) AS c FROM t", &[]);
        let rows = db::decode_rows(&unwrap_json_string(&after_commit)).unwrap();
        assert_eq!(rows[0][0].1, DbValue::Int(1));
    }

    #[test]
    fn invalid_handle_is_thrown_not_panicked() {
        let handler = DbHandler::new();
        let args = json::to_string(&Json::Array(vec![Json::Int(999)]));
        let reply = handler.call(db::OP_CLOSE, &args).unwrap();
        let parsed = json::parse(&reply).unwrap();
        assert!(parsed.get("$thrown").is_some(), "{reply}");
    }

    #[test]
    fn double_close_is_thrown() {
        let handler = DbHandler::new();
        let handle = open_memory(&handler);
        let args = json::to_string(&Json::Array(vec![Json::Int(handle)]));
        assert_eq!(handler.call(db::OP_CLOSE, &args).unwrap(), "null");
        let reply = handler.call(db::OP_CLOSE, &args).unwrap();
        let parsed = json::parse(&reply).unwrap();
        assert!(parsed.get("$thrown").is_some(), "{reply}");
    }

    #[test]
    fn query_against_closed_handle_is_thrown() {
        let handler = DbHandler::new();
        let handle = open_memory(&handler);
        let args = json::to_string(&Json::Array(vec![Json::Int(handle)]));
        handler.call(db::OP_CLOSE, &args).unwrap();
        let reply = query(&handler, handle, "SELECT 1", &[]);
        let parsed = json::parse(&reply).unwrap();
        assert!(parsed.get("$thrown").is_some(), "{reply}");
    }

    #[test]
    fn sql_syntax_error_is_thrown_with_structured_message() {
        let handler = DbHandler::new();
        let handle = open_memory(&handler);
        let reply = execute(&handler, handle, "NOT VALID SQL", &[]);
        let parsed = json::parse(&reply).unwrap();
        let Some(Json::Str(message)) = parsed.get("$thrown") else {
            panic!("expected $thrown string, got {reply}");
        };
        assert!(message.contains("SQLite error"), "{message}");
    }

    #[test]
    fn malformed_params_payload_is_thrown_not_bridge_err() {
        let handler = DbHandler::new();
        let handle = open_memory(&handler);
        // `params` must be a JSON array of tagged values; hand it garbage.
        let args = json::to_string(&Json::Array(vec![
            Json::Int(handle),
            Json::Str("SELECT 1".to_string()),
            Json::Str("not valid json".to_string()),
        ]));
        let reply = handler
            .call(db::OP_EXECUTE, &args)
            .expect("malformed params must be a catchable $thrown, not a bridge Err");
        let parsed = json::parse(&reply).unwrap();
        assert!(parsed.get("$thrown").is_some(), "{reply}");

        let reply = handler
            .call(db::OP_QUERY, &args)
            .expect("malformed params must be a catchable $thrown, not a bridge Err");
        let parsed = json::parse(&reply).unwrap();
        assert!(parsed.get("$thrown").is_some(), "{reply}");
    }

    #[test]
    fn empty_sql_is_a_noop() {
        let handler = DbHandler::new();
        let handle = open_memory(&handler);
        // execute of comment-only SQL: a clean no-op, 0 rows affected.
        let reply = execute(&handler, handle, "-- nothing here", &[]);
        let result = ExecResult::decode(&unwrap_json_string(&reply)).unwrap();
        assert_eq!(result.rows_affected, 0);
        // query of empty SQL: an empty result set, not a panic.
        let selected = query(&handler, handle, "   ", &[]);
        let rows = db::decode_rows(&unwrap_json_string(&selected)).unwrap();
        assert!(rows.is_empty(), "{selected}");
    }

    #[test]
    fn duplicate_result_columns_are_disambiguated() {
        let handler = DbHandler::new();
        let handle = open_memory(&handler);
        execute(&handler, handle, "CREATE TABLE t (a INTEGER)", &[]);
        execute(&handler, handle, "INSERT INTO t VALUES (5)", &[]);
        let selected = query(&handler, handle, "SELECT a, a, a FROM t", &[]);
        let rows = db::decode_rows(&unwrap_json_string(&selected)).unwrap();
        let keys: Vec<&str> = rows[0].iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["a", "a_1", "a_2"]);
    }

    #[test]
    fn open_invalid_path_is_thrown() {
        let handler = DbHandler::new();
        // A directory that does not exist, several levels deep, with no
        // `mkdir -p` — SQLite reports `SQLITE_CANTOPEN` rather than creating
        // intermediate directories.
        let args = json::to_string(&Json::Array(vec![Json::Str(
            "/nonexistent-tswift-dir/does/not/exist.db".to_string(),
        )]));
        let reply = handler.call(db::OP_OPEN, &args).unwrap();
        let parsed = json::parse(&reply).unwrap();
        assert!(parsed.get("$thrown").is_some(), "{reply}");
    }

    #[test]
    fn file_backed_database_persists_across_handlers() {
        let dir = std::env::temp_dir().join(format!(
            "tswift-db-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("test.sqlite3");
        let _ = std::fs::remove_file(&file);
        {
            let handler = DbHandler::new();
            let args = json::to_string(&Json::Array(vec![Json::Str(
                file.to_string_lossy().into_owned(),
            )]));
            let reply = handler.call(db::OP_OPEN, &args).unwrap();
            let Json::Int(handle) = json::parse(&reply).unwrap() else {
                panic!("expected handle");
            };
            execute(&handler, handle, "CREATE TABLE t (v TEXT)", &[]);
            execute(
                &handler,
                handle,
                "INSERT INTO t VALUES (?)",
                &[DbValue::Text("persisted".to_string())],
            );
        }
        {
            let handler = DbHandler::new();
            let args = json::to_string(&Json::Array(vec![Json::Str(
                file.to_string_lossy().into_owned(),
            )]));
            let reply = handler.call(db::OP_OPEN, &args).unwrap();
            let Json::Int(handle) = json::parse(&reply).unwrap() else {
                panic!("expected handle");
            };
            let selected = query(&handler, handle, "SELECT v FROM t", &[]);
            let rows = db::decode_rows(&unwrap_json_string(&selected)).unwrap();
            assert_eq!(
                rows[0][0],
                ("v".to_string(), DbValue::Text("persisted".to_string()))
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
