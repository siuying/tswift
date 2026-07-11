//! A minimal, hand-written FFI binding to the system `libsqlite3` (linked by
//! `build.rs`; see `docs/adr/0015-db-host-service-wire.md`).
//!
//! Deliberately tiny: open/close, prepare/bind/step/column/finalize, plus the
//! two post-execute queries (`changes`, `last_insert_rowid`) and error-message
//! access. That is exactly the surface `db.rs` needs to back
//! `tswift.db.execute`/`tswift.db.query`; no ORM-ish convenience, no covering
//! every SQLite C API entry point. This module is the *only* place in the
//! whole workspace that links or calls into SQLite — `tswift-core`,
//! `tswift-swiftdata`, and every other crate stay pure Rust.
//!
//! Everything below the `extern "C"` block is a safe wrapper: raw pointers
//! never escape this module, and every fallible call surfaces SQLite's own
//! result code + `sqlite3_errmsg` text as a `String` error.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr::{self, NonNull};

#[repr(C)]
struct sqlite3 {
    _private: [u8; 0],
}
#[repr(C)]
struct sqlite3_stmt {
    _private: [u8; 0],
}

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;

const SQLITE_INTEGER: c_int = 1;
const SQLITE_FLOAT: c_int = 2;
const SQLITE_TEXT: c_int = 3;
const SQLITE_BLOB: c_int = 4;
const SQLITE_NULL: c_int = 5;

/// `SQLITE_TRANSIENT`: tells SQLite to copy the bound bytes immediately,
/// rather than assume they outlive the call — required since our `&[u8]`/
/// `&str` bind arguments only live for the duration of the bind call.
const SQLITE_TRANSIENT: isize = -1;

extern "C" {
    fn sqlite3_open(filename: *const c_char, db: *mut *mut sqlite3) -> c_int;
    fn sqlite3_close(db: *mut sqlite3) -> c_int;
    fn sqlite3_errmsg(db: *mut sqlite3) -> *const c_char;
    fn sqlite3_extended_errcode(db: *mut sqlite3) -> c_int;

    fn sqlite3_prepare_v2(
        db: *mut sqlite3,
        sql: *const c_char,
        n_byte: c_int,
        stmt: *mut *mut sqlite3_stmt,
        tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_step(stmt: *mut sqlite3_stmt) -> c_int;
    fn sqlite3_finalize(stmt: *mut sqlite3_stmt) -> c_int;

    fn sqlite3_bind_null(stmt: *mut sqlite3_stmt, i: c_int) -> c_int;
    fn sqlite3_bind_int64(stmt: *mut sqlite3_stmt, i: c_int, v: i64) -> c_int;
    fn sqlite3_bind_double(stmt: *mut sqlite3_stmt, i: c_int, v: f64) -> c_int;
    fn sqlite3_bind_text(
        stmt: *mut sqlite3_stmt,
        i: c_int,
        text: *const c_char,
        n: c_int,
        destructor: isize,
    ) -> c_int;
    fn sqlite3_bind_blob(
        stmt: *mut sqlite3_stmt,
        i: c_int,
        data: *const c_void,
        n: c_int,
        destructor: isize,
    ) -> c_int;

    fn sqlite3_column_count(stmt: *mut sqlite3_stmt) -> c_int;
    fn sqlite3_column_name(stmt: *mut sqlite3_stmt, i: c_int) -> *const c_char;
    fn sqlite3_column_type(stmt: *mut sqlite3_stmt, i: c_int) -> c_int;
    fn sqlite3_column_int64(stmt: *mut sqlite3_stmt, i: c_int) -> i64;
    fn sqlite3_column_double(stmt: *mut sqlite3_stmt, i: c_int) -> f64;
    fn sqlite3_column_text(stmt: *mut sqlite3_stmt, i: c_int) -> *const u8;
    fn sqlite3_column_blob(stmt: *mut sqlite3_stmt, i: c_int) -> *const c_void;
    fn sqlite3_column_bytes(stmt: *mut sqlite3_stmt, i: c_int) -> c_int;

    fn sqlite3_changes(db: *mut sqlite3) -> c_int;
    fn sqlite3_last_insert_rowid(db: *mut sqlite3) -> i64;
}

/// A SQLite value read back from a result column, tagged by SQLite's own
/// storage-class enum (`sqlite3_column_type`) — the caller (`db.rs`) maps
/// this onto `tswift_swiftdata::db::DbValue`.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnValue {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// A structured SQLite failure: the result code (plain or extended, whatever
/// `sqlite3_extended_errcode` reports) plus `sqlite3_errmsg`'s text — the
/// "code + message" shape `db.rs` formats into the `$thrown` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteError {
    pub code: i32,
    pub message: String,
}

impl std::fmt::Display for SqliteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SQLite error {}: {}", self.code, self.message)
    }
}

/// A **non-null** prepared-statement pointer. The only constructor,
/// [`Stmt::new`], rejects NULL, so holding a `Stmt` is proof the pointer is
/// non-null: every `Statement` method can call into SQLite without a
/// per-callsite null guard. The "no statement" case (empty / whitespace /
/// comment-only SQL, where `sqlite3_prepare_v2` yields `SQLITE_OK` + a NULL
/// out-pointer) is represented structurally by `Statement.stmt == None`,
/// never by a live `Stmt` — so a NULL deref is unrepresentable by
/// construction rather than avoided by convention.
#[derive(Debug, Clone, Copy)]
struct Stmt(NonNull<sqlite3_stmt>);

impl Stmt {
    /// `Some` iff `raw` is non-null.
    fn new(raw: *mut sqlite3_stmt) -> Option<Stmt> {
        NonNull::new(raw).map(Stmt)
    }

    fn as_ptr(self) -> *mut sqlite3_stmt {
        self.0.as_ptr()
    }
}

/// Convert a byte length into SQLite's `c_int` bind-length argument,
/// rejecting any value that would wrap a signed 32-bit int. SQLite's
/// non-`*64` `sqlite3_bind_text`/`sqlite3_bind_blob` cap at `i32::MAX`
/// bytes; feeding a larger length (possible on 64-bit builds) would silently
/// truncate/wrap to a negative or bogus count. We surface it as a structured
/// (catchable) error instead. Factored out so the boundary is unit-testable
/// without allocating a multi-gigabyte buffer.
fn checked_bind_len(len: usize) -> Result<c_int, SqliteError> {
    i32::try_from(len).map_err(|_| SqliteError {
        code: -1,
        message: format!(
            "value too large: {len} bytes exceeds SQLite's {}-byte bind limit",
            i32::MAX
        ),
    })
}

/// An open SQLite connection. `Send` (never `Sync`-shared concurrently by
/// this crate — see `db.rs`'s locking) so it can sit behind a `Mutex`.
pub struct Connection {
    handle: *mut sqlite3,
}

// SAFETY: a `sqlite3*` may be handed to another thread as long as it is not
// used concurrently from two threads at once (SQLite's default build is
// "serialized" thread-safe, but this crate additionally never lets two
// threads touch one `Connection` at the same time — `db.rs` holds the whole
// connection table behind one `Mutex`, so calls are already serialized).
unsafe impl Send for Connection {}

impl Connection {
    /// Open (creating if absent) the database file at `path`.
    pub fn open(path: &str) -> Result<Connection, SqliteError> {
        let c_path = CString::new(path).map_err(|_| SqliteError {
            code: -1,
            message: "path contains an interior NUL byte".to_string(),
        })?;
        let mut handle: *mut sqlite3 = ptr::null_mut();
        // SAFETY: `c_path` is a valid, NUL-terminated C string for the
        // duration of the call; `handle` is a valid out-pointer.
        let rc = unsafe { sqlite3_open(c_path.as_ptr(), &mut handle) };
        if rc != SQLITE_OK {
            let err = last_error(handle, rc);
            // `sqlite3_open` still allocates a handle on failure (needed to
            // read the error message off it); it must be closed either way.
            if !handle.is_null() {
                unsafe { sqlite3_close(handle) };
            }
            return Err(err);
        }
        Ok(Connection { handle })
    }

    /// Prepare `sql` (a single statement — `sqlite3_prepare_v2` silently
    /// ignores anything after the first `;`, matching this module's
    /// single-statement-per-call scope).
    ///
    /// `sqlite3_prepare_v2` returns `SQLITE_OK` with a **null** statement
    /// out-pointer when `sql` compiles to no statement at all (empty,
    /// all-whitespace, or comment-only input). We surface that as a valid
    /// `Statement` whose `stmt` is `None`; every method below treats the
    /// no-statement case as a completed no-op (zero rows, zero columns) —
    /// matching the `sqlite3` CLI's behavior of quietly accepting an empty
    /// statement.
    pub fn prepare(&self, sql: &str) -> Result<Statement<'_>, SqliteError> {
        let c_sql = CString::new(sql).map_err(|_| SqliteError {
            code: -1,
            message: "sql contains an interior NUL byte".to_string(),
        })?;
        let mut stmt: *mut sqlite3_stmt = ptr::null_mut();
        // SAFETY: `self.handle` is a live connection; `c_sql` is valid for
        // the call; `stmt`/`tail` are valid out-pointers (tail unused).
        let rc = unsafe {
            sqlite3_prepare_v2(self.handle, c_sql.as_ptr(), -1, &mut stmt, ptr::null_mut())
        };
        if rc != SQLITE_OK {
            return Err(self.last_error(rc));
        }
        // `stmt` may legitimately be null here (no-op statement). `Stmt::new`
        // maps that to `None`, so the no-op case is represented structurally
        // and every method below is null-safe by construction.
        Ok(Statement {
            conn: self,
            stmt: Stmt::new(stmt),
        })
    }

    /// Rows changed by the most recently completed `INSERT`/`UPDATE`/`DELETE`.
    pub fn changes(&self) -> i64 {
        // SAFETY: `self.handle` is a live connection.
        unsafe { sqlite3_changes(self.handle) as i64 }
    }

    /// `rowid` of the most recent successful `INSERT`.
    pub fn last_insert_rowid(&self) -> i64 {
        // SAFETY: `self.handle` is a live connection.
        unsafe { sqlite3_last_insert_rowid(self.handle) }
    }

    /// Execute a bare statement with no bind parameters and discard its rows
    /// — used for `BEGIN`/`COMMIT`/`ROLLBACK`.
    pub fn exec_simple(&self, sql: &str) -> Result<(), SqliteError> {
        let stmt = self.prepare(sql)?;
        stmt.step_to_completion()?;
        Ok(())
    }

    fn last_error(&self, rc: c_int) -> SqliteError {
        last_error(self.handle, rc)
    }
}

fn last_error(handle: *mut sqlite3, rc: c_int) -> SqliteError {
    if handle.is_null() {
        return SqliteError {
            code: rc,
            message: "sqlite3 handle is null".to_string(),
        };
    }
    // SAFETY: `handle` is non-null; both calls just read connection state.
    let code = unsafe { sqlite3_extended_errcode(handle) };
    let message = unsafe {
        let ptr = sqlite3_errmsg(handle);
        if ptr.is_null() {
            String::new()
        } else {
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    };
    SqliteError {
        code: code as i32,
        message,
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: `self.handle` was returned by `sqlite3_open` and is
            // dropped exactly once here; no `Statement` outlives its
            // `Connection` (borrowed lifetime `'_` enforces that at compile
            // time), so every prepared statement is already finalized.
            unsafe {
                sqlite3_close(self.handle);
            }
        }
    }
}

/// A prepared statement, borrowed from its owning [`Connection`] — it cannot
/// outlive the connection (enforced by the `'a` lifetime), which guarantees
/// `sqlite3_close` never runs while a statement is still live.
#[derive(Debug)]
pub struct Statement<'a> {
    conn: &'a Connection,
    /// `None` for a no-op statement (empty/whitespace/comment-only SQL);
    /// `Some` wraps a guaranteed-non-null pointer — see [`Stmt`].
    stmt: Option<Stmt>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection").finish_non_exhaustive()
    }
}

/// The outcome of one [`Statement::step`] call.
pub enum Step {
    /// A result row is available; read it with `Statement::column_*`.
    Row,
    /// The statement is finished; no more rows.
    Done,
}

impl<'a> Statement<'a> {
    /// Bind positional parameter `index` (1-based, matching SQLite's own
    /// `?`/`?NNN` numbering) to `value`.
    pub fn bind(&self, index: c_int, value: &ColumnValue) -> Result<(), SqliteError> {
        let Some(stmt) = self.stmt else {
            // A no-op statement (empty/whitespace/comment-only SQL) has no
            // bind parameters; binding one is a caller error, surfaced as a
            // structured (catchable) failure rather than a NULL deref.
            return Err(SqliteError {
                code: -1,
                message: "cannot bind a parameter to an empty statement".to_string(),
            });
        };
        let ptr = stmt.as_ptr();
        // Length checks happen before entering `unsafe` so an over-large
        // text/blob returns a structured error instead of wrapping to a
        // bogus `c_int` count.
        let rc = match value {
            // SAFETY: `ptr` is a live, non-finalized prepared statement
            // (non-null by `Stmt` construction); `index` is caller-supplied
            // per SQLite's own 1-based convention.
            ColumnValue::Null => unsafe { sqlite3_bind_null(ptr, index) },
            ColumnValue::Int(i) => unsafe { sqlite3_bind_int64(ptr, index, *i) },
            ColumnValue::Real(d) => unsafe { sqlite3_bind_double(ptr, index, *d) },
            ColumnValue::Text(s) => {
                let bytes = s.as_bytes();
                let n = checked_bind_len(bytes.len())?;
                // SAFETY: as above; `bytes` is valid for `n` bytes and
                // SQLITE_TRANSIENT tells SQLite to copy before returning.
                unsafe {
                    sqlite3_bind_text(
                        ptr,
                        index,
                        bytes.as_ptr() as *const c_char,
                        n,
                        SQLITE_TRANSIENT,
                    )
                }
            }
            ColumnValue::Blob(bytes) => {
                let n = checked_bind_len(bytes.len())?;
                // SAFETY: as above; `bytes` is valid for `n` bytes.
                unsafe {
                    sqlite3_bind_blob(
                        ptr,
                        index,
                        bytes.as_ptr() as *const c_void,
                        n,
                        SQLITE_TRANSIENT,
                    )
                }
            }
        };
        if rc != SQLITE_OK {
            return Err(self.conn.last_error(rc));
        }
        Ok(())
    }

    /// Advance to the next row (or completion). A no-op statement (null
    /// `stmt`) is already complete: it yields no rows.
    pub fn step(&self) -> Result<Step, SqliteError> {
        let Some(stmt) = self.stmt else {
            return Ok(Step::Done);
        };
        // SAFETY: `stmt` is a live prepared statement (non-null by `Stmt`).
        let rc = unsafe { sqlite3_step(stmt.as_ptr()) };
        match rc {
            SQLITE_ROW => Ok(Step::Row),
            SQLITE_DONE => Ok(Step::Done),
            _ => Err(self.conn.last_error(rc)),
        }
    }

    /// Step until the statement completes, discarding any rows (bare
    /// DDL/`BEGIN`/`COMMIT` statements never produce rows, but tolerate one
    /// anyway rather than erroring).
    pub fn step_to_completion(&self) -> Result<(), SqliteError> {
        loop {
            match self.step()? {
                Step::Row => continue,
                Step::Done => return Ok(()),
            }
        }
    }

    pub fn column_count(&self) -> usize {
        let Some(stmt) = self.stmt else {
            return 0;
        };
        // SAFETY: `stmt` is a live prepared statement (non-null by `Stmt`).
        unsafe { sqlite3_column_count(stmt.as_ptr()) as usize }
    }

    pub fn column_name(&self, i: usize) -> String {
        let Some(stmt) = self.stmt else {
            return String::new();
        };
        // SAFETY: `stmt` is live (non-null by `Stmt`); `i` is caller-checked
        // against `column_count`. SQLite guarantees a NUL-terminated UTF-8
        // name (or null on OOM, guarded below).
        unsafe {
            let ptr = sqlite3_column_name(stmt.as_ptr(), i as c_int);
            if ptr.is_null() {
                String::new()
            } else {
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        }
    }

    /// Read column `i` of the current row, tagged by SQLite's own storage
    /// class. Must only be called right after `step()` returned
    /// [`Step::Row`].
    ///
    /// A `TEXT` column that does not hold valid UTF-8 is surfaced as a
    /// structured (catchable) error rather than being silently lossy-decoded
    /// (which would corrupt the value with replacement characters). A
    /// lossless blob-style representation of malformed text is intentionally
    /// out of scope for v1 — well-formed UTF-8 is the norm and a clean error
    /// beats silent corruption.
    pub fn column_value(&self, i: usize) -> Result<ColumnValue, SqliteError> {
        let Some(stmt) = self.stmt else {
            // A no-op statement never yields a row, so this method's
            // precondition (called after `step()` returned `Row`) cannot
            // hold; return `Null` rather than dereferencing a null pointer.
            return Ok(ColumnValue::Null);
        };
        let handle = stmt.as_ptr();
        let idx = i as c_int;
        // SAFETY: `handle` is live (non-null by `Stmt`) and currently
        // positioned on a row (the documented precondition of this method);
        // `idx` is caller-checked against `column_count`.
        unsafe {
            match sqlite3_column_type(handle, idx) {
                SQLITE_INTEGER => Ok(ColumnValue::Int(sqlite3_column_int64(handle, idx))),
                SQLITE_FLOAT => Ok(ColumnValue::Real(sqlite3_column_double(handle, idx))),
                SQLITE_TEXT => {
                    let ptr = sqlite3_column_text(handle, idx);
                    let len = sqlite3_column_bytes(handle, idx) as usize;
                    if ptr.is_null() || len == 0 {
                        Ok(ColumnValue::Text(String::new()))
                    } else {
                        let slice = std::slice::from_raw_parts(ptr, len);
                        match std::str::from_utf8(slice) {
                            Ok(s) => Ok(ColumnValue::Text(s.to_string())),
                            Err(_) => Err(SqliteError {
                                code: -1,
                                message: format!(
                                    "column {i} contains non-UTF-8 TEXT (not representable; \
                                     lossless blob decoding is out of scope for v1)"
                                ),
                            }),
                        }
                    }
                }
                SQLITE_BLOB => {
                    let ptr = sqlite3_column_blob(handle, idx);
                    let len = sqlite3_column_bytes(handle, idx) as usize;
                    if ptr.is_null() || len == 0 {
                        Ok(ColumnValue::Blob(Vec::new()))
                    } else {
                        let slice = std::slice::from_raw_parts(ptr as *const u8, len);
                        Ok(ColumnValue::Blob(slice.to_vec()))
                    }
                }
                SQLITE_NULL => Ok(ColumnValue::Null),
                _ => Ok(ColumnValue::Null),
            }
        }
    }
}

impl<'a> Drop for Statement<'a> {
    fn drop(&mut self) {
        if let Some(stmt) = self.stmt {
            // SAFETY: `stmt` was returned by `sqlite3_prepare_v2` (non-null by
            // `Stmt` construction) and is finalized exactly once here.
            unsafe {
                sqlite3_finalize(stmt.as_ptr());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_memory() -> Connection {
        Connection::open(":memory:").unwrap()
    }

    #[test]
    fn open_memory_db_and_create_table() {
        let conn = open_memory();
        conn.exec_simple("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
            .unwrap();
    }

    #[test]
    fn insert_and_query_round_trips_every_type() {
        let conn = open_memory();
        conn.exec_simple("CREATE TABLE t (i INTEGER, r REAL, s TEXT, b BLOB, n INTEGER)")
            .unwrap();
        let stmt = conn
            .prepare("INSERT INTO t VALUES (?, ?, ?, ?, ?)")
            .unwrap();
        stmt.bind(1, &ColumnValue::Int(42)).unwrap();
        stmt.bind(2, &ColumnValue::Real(3.5)).unwrap();
        stmt.bind(3, &ColumnValue::Text("hi".to_string())).unwrap();
        stmt.bind(4, &ColumnValue::Blob(vec![1, 2, 3])).unwrap();
        stmt.bind(5, &ColumnValue::Null).unwrap();
        stmt.step_to_completion().unwrap();
        assert_eq!(conn.changes(), 1);

        let query = conn.prepare("SELECT i, r, s, b, n FROM t").unwrap();
        match query.step().unwrap() {
            Step::Row => {}
            Step::Done => panic!("expected a row"),
        }
        assert_eq!(query.column_count(), 5);
        assert_eq!(query.column_name(0), "i");
        assert_eq!(query.column_value(0).unwrap(), ColumnValue::Int(42));
        assert_eq!(query.column_value(1).unwrap(), ColumnValue::Real(3.5));
        assert_eq!(
            query.column_value(2).unwrap(),
            ColumnValue::Text("hi".to_string())
        );
        assert_eq!(
            query.column_value(3).unwrap(),
            ColumnValue::Blob(vec![1, 2, 3])
        );
        assert_eq!(query.column_value(4).unwrap(), ColumnValue::Null);
        assert!(matches!(query.step().unwrap(), Step::Done));
    }

    #[test]
    fn last_insert_rowid_tracks_autoincrement() {
        let conn = open_memory();
        conn.exec_simple("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
            .unwrap();
        let stmt = conn.prepare("INSERT INTO t (v) VALUES (?)").unwrap();
        stmt.bind(1, &ColumnValue::Text("a".to_string())).unwrap();
        stmt.step_to_completion().unwrap();
        assert_eq!(conn.last_insert_rowid(), 1);
        let stmt2 = conn.prepare("INSERT INTO t (v) VALUES (?)").unwrap();
        stmt2.bind(1, &ColumnValue::Text("b".to_string())).unwrap();
        stmt2.step_to_completion().unwrap();
        assert_eq!(conn.last_insert_rowid(), 2);
    }

    #[test]
    fn syntax_error_surfaces_structured_error() {
        let conn = open_memory();
        let err = conn.prepare("NOT VALID SQL").unwrap_err();
        assert_ne!(err.code, 0);
        assert!(!err.message.is_empty());
    }

    #[test]
    fn transactions_commit_and_rollback() {
        let conn = open_memory();
        conn.exec_simple("CREATE TABLE t (v INTEGER)").unwrap();

        conn.exec_simple("BEGIN").unwrap();
        conn.prepare("INSERT INTO t VALUES (1)")
            .unwrap()
            .step_to_completion()
            .unwrap();
        conn.exec_simple("ROLLBACK").unwrap();
        let q = conn.prepare("SELECT COUNT(*) FROM t").unwrap();
        assert!(matches!(q.step().unwrap(), Step::Row));
        assert_eq!(q.column_value(0).unwrap(), ColumnValue::Int(0));

        conn.exec_simple("BEGIN").unwrap();
        conn.prepare("INSERT INTO t VALUES (1)")
            .unwrap()
            .step_to_completion()
            .unwrap();
        conn.exec_simple("COMMIT").unwrap();
        let q2 = conn.prepare("SELECT COUNT(*) FROM t").unwrap();
        assert!(matches!(q2.step().unwrap(), Step::Row));
        assert_eq!(q2.column_value(0).unwrap(), ColumnValue::Int(1));
    }

    #[test]
    fn empty_and_comment_only_sql_is_a_noop_statement() {
        let conn = open_memory();
        for sql in ["", "   ", "-- just a comment", "/* block */"] {
            let stmt = conn.prepare(sql).unwrap();
            assert_eq!(stmt.column_count(), 0);
            // A no-op statement completes immediately with no rows.
            assert!(matches!(stmt.step().unwrap(), Step::Done));
            stmt.step_to_completion().unwrap();
            // Binding into an empty statement is a structured error, not UB.
            assert!(stmt.bind(1, &ColumnValue::Int(1)).is_err());
        }
    }

    #[test]
    fn checked_bind_len_rejects_oversize_without_allocating() {
        // Boundary: i32::MAX fits, i32::MAX + 1 must be rejected. No buffer
        // is allocated \u2014 only the length integer is checked.
        assert_eq!(checked_bind_len(0).unwrap(), 0);
        assert_eq!(checked_bind_len(i32::MAX as usize).unwrap(), i32::MAX);
        let err = checked_bind_len(i32::MAX as usize + 1).unwrap_err();
        assert_eq!(err.code, -1);
        assert!(err.message.contains("value too large"), "{}", err.message);
        // usize::MAX (as would arise from a huge Vec on 64-bit) is rejected.
        assert!(checked_bind_len(usize::MAX).is_err());
    }

    #[test]
    fn non_utf8_text_surfaces_structured_error() {
        let conn = open_memory();
        conn.exec_simple("CREATE TABLE t (v TEXT)").unwrap();
        // Insert an invalid UTF-8 byte sequence (0xff 0xfe) as TEXT via a
        // blob-to-text cast trick: bind a blob, then read the column as text.
        conn.exec_simple("INSERT INTO t VALUES (CAST(x'fffe' AS TEXT))")
            .unwrap();
        let q = conn.prepare("SELECT v FROM t").unwrap();
        assert!(matches!(q.step().unwrap(), Step::Row));
        let err = q.column_value(0).unwrap_err();
        assert!(err.message.contains("non-UTF-8"), "{}", err.message);
    }
}
