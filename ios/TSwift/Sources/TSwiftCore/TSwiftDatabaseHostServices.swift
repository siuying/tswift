import Foundation
import SQLite3
import TSwiftFFI

/// Real-SQLite host backing for `tswift.db.*` (see
/// `crates/tswift-swiftdata/src/db.rs` for the op names / tagged `DbValue`
/// wire codec this file implements, `crates/tswift-cli/src/db.rs` for the
/// reference native (CLI) backing this one mirrors wire-for-wire, and
/// `docs/adr/0015-db-host-service-wire.md` for the full wire contract).
///
/// ## Platform tier: iOS (native — same class as the CLI's backing, not a
/// degraded one)
///
/// `import SQLite3` links the system `libsqlite3` that ships as part of
/// every Apple platform SDK (no vendored/bundled SQLite, no third-party
/// dependency) — the same "real SQL, real file-backed databases" tier the
/// CLI ships (`crates/tswift-cli/src/sqlite_ffi.rs`), just reached through
/// Swift's own `SQLite3` system module instead of a hand-written Rust FFI.
///
/// ## Usage
///
/// ```swift
/// let context = TSwiftContext()
/// try context.installDatabaseHostServices()
/// let result = TSwiftCore.run(script, in: context)
/// ```
///
/// There is no Swift-facing SQL/`@Model` API in this slice (see the ADR —
/// that is future work layered on top of this wire), so nothing in the
/// standard library calls `tswift.db.*` yet; this file exists to make the
/// host capability real ahead of that layer, exactly like the CLI shipped
/// its backing before any Swift-facing SwiftData surface existed.
extension TSwiftContext {
    /// Declare `tswift.db` and register its seven host functions, backed by
    /// `TSwiftDatabaseBacking` (real SQLite via `import SQLite3`). Call once
    /// per context, before the first `run`/SwiftUI compile that needs
    /// `tswift.db`.
    @discardableResult
    public func installDatabaseHostServices() throws -> Self {
        try declareHostService("tswift.db")
        let backing = TSwiftDatabaseBacking()

        try registerHostFunction(
            .init(
                name: "tswift.db.open",
                parameters: [.init(label: "path", type: .string)],
                returns: .int,
                throwing: true
            )
        ) { args in
            try backing.open(path: args[0] as? String ?? "")
        }

        try registerHostFunction(
            .init(
                name: "tswift.db.close",
                parameters: [.init(label: "handle", type: .int)],
                throwing: true
            )
        ) { args in
            try backing.close(handle: Self.intArg(args, 0))
            return nil
        }

        try registerHostFunction(
            .init(
                name: "tswift.db.execute",
                parameters: [
                    .init(label: "handle", type: .int),
                    .init(label: "sql", type: .string),
                    .init(label: "params", type: .string),
                ],
                returns: .string,
                throwing: true
            )
        ) { args in
            try backing.execute(
                handle: Self.intArg(args, 0),
                sql: args[1] as? String ?? "",
                paramsJSON: args[2] as? String ?? "[]"
            )
        }

        try registerHostFunction(
            .init(
                name: "tswift.db.query",
                parameters: [
                    .init(label: "handle", type: .int),
                    .init(label: "sql", type: .string),
                    .init(label: "params", type: .string),
                ],
                returns: .string,
                throwing: true
            )
        ) { args in
            try backing.query(
                handle: Self.intArg(args, 0),
                sql: args[1] as? String ?? "",
                paramsJSON: args[2] as? String ?? "[]"
            )
        }

        try registerHostFunction(
            .init(
                name: "tswift.db.begin",
                parameters: [.init(label: "handle", type: .int)],
                throwing: true
            )
        ) { args in
            try backing.begin(handle: Self.intArg(args, 0))
            return nil
        }

        try registerHostFunction(
            .init(
                name: "tswift.db.commit",
                parameters: [.init(label: "handle", type: .int)],
                throwing: true
            )
        ) { args in
            try backing.commit(handle: Self.intArg(args, 0))
            return nil
        }

        try registerHostFunction(
            .init(
                name: "tswift.db.rollback",
                parameters: [.init(label: "handle", type: .int)],
                throwing: true
            )
        ) { args in
            try backing.rollback(handle: Self.intArg(args, 0))
            return nil
        }

        return self
    }

    /// Decode a positional `Int` argument (`NSNumber`, per
    /// `HostFunctionBox`'s JSON-decode shape — see `TSwiftHostFunction.swift`).
    fileprivate static func intArg(_ args: [Any], _ index: Int) -> Int {
        (args[index] as? NSNumber)?.intValue ?? (args[index] as? Int) ?? 0
    }
}

/// The real SQLite backing for `tswift.db.*`: an ascending `Int` handle
/// counter plus a handle→connection table (mirrors
/// `crates/tswift-cli/src/db.rs::DbHandler` — the interpreter is
/// single-threaded, ADR-0005, so no lock is needed here either, same as the
/// Rust side's own reasoning for using a plain `Mutex` rather than anything
/// fancier).
///
/// Every method's `throws` surfaces as `TSwiftHostFunctionError` directly
/// (its `message` shaped `"SQLite error <extended-code>: <sqlite3_errmsg
/// text>"`, matching the native CLI backing's `db.rs::thrown_sqlite`, or
/// `"tswift.db.<op>: handle <n> is not open"` for a bad handle, matching
/// `db.rs`'s handle-lifecycle errors) so it crosses the host-function bridge
/// as a catchable `$thrown` Swift error — same structured "code + message in
/// text" shape as every other platform, with no extra error-type conversion
/// needed in the `installDatabaseHostServices` registration closures, since
/// `HostFunctionBox.respond` (`TSwiftHostFunction.swift`) already special-
/// cases `TSwiftHostFunctionError` to encode `message` verbatim as `$thrown`.
public final class TSwiftDatabaseBacking {
    private var nextHandle: Int = 1
    private var connections: [Int: OpaquePointer] = [:]

    public init() {}

    deinit {
        for (_, db) in connections { sqlite3_close(db) }
    }

    // MARK: - tswift.db.open / .close

    public func open(path: String) throws -> Int {
        var db: OpaquePointer?
        let rc = sqlite3_open(path, &db)
        if rc != SQLITE_OK {
            let error = Self.lastError(db, rc: rc)
            if let db { sqlite3_close(db) }
            throw TSwiftHostFunctionError(error)
        }
        let handle = nextHandle
        nextHandle += 1
        connections[handle] = db
        return handle
    }

    public func close(handle: Int) throws {
        guard let db = connections.removeValue(forKey: handle) else {
            throw TSwiftHostFunctionError(
                "tswift.db.close: handle \(handle) is not open (already closed, or never opened)"
            )
        }
        sqlite3_close(db)
    }

    // MARK: - tswift.db.execute / .query

    public func execute(handle: Int, sql: String, paramsJSON: String) throws -> String {
        let db = try connection(handle, op: "tswift.db.execute")
        let params = try Self.decodeParams(paramsJSON, op: "tswift.db.execute")
        let stmt = try prepare(db, sql: sql)
        defer { sqlite3_finalize(stmt) }
        if let stmt {
            try bind(stmt, params: params, db: db)
            try stepToCompletion(stmt, db: db)
        }
        let rowsAffected = Int(sqlite3_changes(db))
        let lastInsertRowid = sqlite3_last_insert_rowid(db)
        let reply = "{\"rowsAffected\":\(rowsAffected),\"lastInsertRowid\":\(lastInsertRowid)}"
        return reply
    }

    public func query(handle: Int, sql: String, paramsJSON: String) throws -> String {
        let db = try connection(handle, op: "tswift.db.query")
        let params = try Self.decodeParams(paramsJSON, op: "tswift.db.query")
        let stmt = try prepare(db, sql: sql)
        defer { sqlite3_finalize(stmt) }
        guard let stmt else { return "[]" }
        try bind(stmt, params: params, db: db)
        let colCount = Int(sqlite3_column_count(stmt))
        var colNames: [String] = []
        colNames.reserveCapacity(colCount)
        for i in 0..<colCount {
            colNames.append(String(cString: sqlite3_column_name(stmt, Int32(i))))
        }
        var rows: [[(String, String)]] = []
        while true {
            let rc = sqlite3_step(stmt)
            if rc == SQLITE_ROW {
                var row: [(String, String)] = []
                row.reserveCapacity(colCount)
                for i in 0..<colCount {
                    row.append((colNames[i], try Self.columnValueJSON(stmt, Int32(i))))
                }
                rows.append(row)
            } else if rc == SQLITE_DONE {
                break
            } else {
                throw TSwiftHostFunctionError(Self.lastError(db, rc: rc))
            }
        }
        return Self.encodeRows(rows)
    }

    // MARK: - tswift.db.begin / .commit / .rollback

    public func begin(handle: Int) throws { try runControl(handle, sql: "BEGIN") }
    public func commit(handle: Int) throws { try runControl(handle, sql: "COMMIT") }
    public func rollback(handle: Int) throws { try runControl(handle, sql: "ROLLBACK") }

    private func runControl(_ handle: Int, sql: String) throws {
        let db = try connection(handle, op: "tswift.db.\(sql.lowercased())")
        let stmt = try prepare(db, sql: sql)
        defer { sqlite3_finalize(stmt) }
        if let stmt {
            try stepToCompletion(stmt, db: db)
        }
    }

    // MARK: - Handle lookup

    private func connection(_ handle: Int, op: String) throws -> OpaquePointer {
        guard let db = connections[handle] else {
            throw TSwiftHostFunctionError("\(op): handle \(handle) is not open")
        }
        return db
    }

    // MARK: - Statement helpers

    /// `sqlite3_prepare_v2` returns `SQLITE_OK` with a **null** statement
    /// out-pointer for SQL that compiles to no statement at all (empty,
    /// all-whitespace, or comment-only) — mirrored here exactly like
    /// `crates/tswift-cli/src/sqlite_ffi.rs::Connection::prepare`'s own doc:
    /// treated as a valid, no-op statement (`nil`), not an error.
    private func prepare(_ db: OpaquePointer, sql: String) throws -> OpaquePointer? {
        var stmt: OpaquePointer?
        let rc = sqlite3_prepare_v2(db, sql, -1, &stmt, nil)
        if rc != SQLITE_OK {
            throw TSwiftHostFunctionError(Self.lastError(db, rc: rc))
        }
        return stmt
    }

    private func stepToCompletion(_ stmt: OpaquePointer, db: OpaquePointer) throws {
        while true {
            let rc = sqlite3_step(stmt)
            if rc == SQLITE_DONE { return }
            if rc != SQLITE_ROW {
                throw TSwiftHostFunctionError(Self.lastError(db, rc: rc))
            }
            // A bare DDL/BEGIN/COMMIT statement never produces rows, but
            // tolerate one anyway rather than erroring (matches
            // `sqlite_ffi.rs::step_to_completion`).
        }
    }

    private func bind(_ stmt: OpaquePointer, params: [DbValue], db: OpaquePointer) throws {
        for (i, value) in params.enumerated() {
            let idx = Int32(i + 1) // 1-based, matching SQLite's own `?` numbering.
            let rc: Int32
            switch value {
            case .null:
                rc = sqlite3_bind_null(stmt, idx)
            case let .int(n):
                rc = sqlite3_bind_int64(stmt, idx, n)
            case let .real(d):
                rc = sqlite3_bind_double(stmt, idx, d)
            case let .text(s):
                rc = sqlite3_bind_text(stmt, idx, s, -1, Self.sqliteTransient)
            case let .blob(bytes):
                rc = bytes.withUnsafeBytes { raw -> Int32 in
                    sqlite3_bind_blob(stmt, idx, raw.baseAddress, Int32(bytes.count), Self.sqliteTransient)
                }
            }
            if rc != SQLITE_OK {
                throw TSwiftHostFunctionError(Self.lastError(db, rc: rc))
            }
        }
    }

    /// `SQLITE_TRANSIENT`: tells SQLite to copy the bound bytes immediately
    /// rather than assume they outlive the call (the Swift `String`/`Data`
    /// bind arguments here only live for the duration of the bind call) —
    /// same reasoning as `sqlite_ffi.rs`'s own `SQLITE_TRANSIENT` constant.
    private static let sqliteTransient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)

    private static func lastError(_ db: OpaquePointer?, rc: Int32) -> String {
        guard let db else { return "SQLite error \(rc): sqlite3 handle is null" }
        let code = sqlite3_extended_errcode(db)
        let message = String(cString: sqlite3_errmsg(db))
        return "SQLite error \(code): \(message)"
    }

    // MARK: - Column reading

    /// Read result column `i` of the statement's current row, tagged by
    /// SQLite's own storage class (`sqlite3_column_type`), and return the
    /// wire-shape tagged-JSON text for it (mirrors
    /// `crates/tswift-cli/src/sqlite_ffi.rs::Statement::column_value` +
    /// `tswift_swiftdata::db::DbValue::to_json`).
    ///
    /// A `TEXT` column that does not hold valid UTF-8 is surfaced as a
    /// structured (catchable) `TSwiftHostFunctionError`, matching the native
    /// CLI backing exactly (`sqlite_ffi.rs::column_value`'s `SQLITE_TEXT`
    /// arm / `non_utf8_text_surfaces_structured_error`), rather than
    /// silently lossy-decoding through `String(cString:)` (which would
    /// replace invalid sequences with U+FFFD and corrupt the value with no
    /// signal to the caller). Reads the column's raw bytes directly
    /// (`sqlite3_column_text`/`sqlite3_column_bytes`, not `String(cString:)`)
    /// and validates them strictly via `String(bytes:encoding:.utf8)`, which
    /// returns `nil` — rather than lossy-substituting — on malformed input.
    private static func columnValueJSON(_ stmt: OpaquePointer, _ i: Int32) throws -> String {
        switch sqlite3_column_type(stmt, i) {
        case SQLITE_INTEGER:
            return "{\"int\":\(sqlite3_column_int64(stmt, i))}"
        case SQLITE_FLOAT:
            return "{\"real\":\(realLiteral(sqlite3_column_double(stmt, i)))}"
        case SQLITE_TEXT:
            let len = Int(sqlite3_column_bytes(stmt, i))
            let text: String
            if len == 0 {
                text = ""
            } else if let raw = sqlite3_column_text(stmt, i) {
                let bytes = raw.withMemoryRebound(to: UInt8.self, capacity: len) { ptr in
                    Array(UnsafeBufferPointer(start: ptr, count: len))
                }
                guard let decoded = String(bytes: bytes, encoding: .utf8) else {
                    throw TSwiftHostFunctionError(
                        "SQLite error -1: column \(i) contains non-UTF-8 TEXT (not representable; "
                            + "lossless blob decoding is out of scope for v1)"
                    )
                }
                text = decoded
            } else {
                text = ""
            }
            return "{\"text\":\(jsonStringLiteral(text))}"
        case SQLITE_BLOB:
            let count = Int(sqlite3_column_bytes(stmt, i))
            let bytes: [UInt8]
            if count > 0, let raw = sqlite3_column_blob(stmt, i) {
                bytes = Array(UnsafeRawBufferPointer(start: raw, count: count))
            } else {
                bytes = []
            }
            return "{\"blob\":\(jsonStringLiteral(Data(bytes).base64EncodedString()))}"
        default: // SQLITE_NULL, or any future storage class.
            return "{\"null\":null}"
        }
    }

    /// The `real` payload: a bare JSON number literal for a finite,
    /// non-negative-zero value; a tagged sentinel string for
    /// `NaN`/`±Infinity`/`-0` (mirrors
    /// `tswift_swiftdata::db::real_payload` — plain JSON has no `NaN`/
    /// `Infinity` literal and `-0` re-parses as `0`).
    private static func realLiteral(_ d: Double) -> String {
        if d.isNaN { return "\"nan\"" }
        if d.isInfinite { return d > 0 ? "\"inf\"" : "\"-inf\"" }
        if d == 0, d.sign == .minus { return "\"-0\"" }
        return "\(d)"
    }

    private static func jsonStringLiteral(_ s: String) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: [s], options: []),
              let text = String(data: data, encoding: .utf8)
        else {
            return "\"\""
        }
        // `JSONSerialization` wraps `s` in a one-element array; strip the
        // brackets to get just the encoded string literal.
        return String(text.dropFirst().dropLast())
    }

    /// Encode `tswift.db.query`'s reply: a JSON array of column-name-keyed
    /// objects, one per row, with duplicate column names disambiguated
    /// exactly like `tswift_swiftdata::db::disambiguate_columns` (`a`,
    /// `a_1`, `a_2`, \u2026, advancing past any real collision).
    private static func encodeRows(_ rows: [[(String, String)]]) -> String {
        let encodedRows = rows.map { row -> String in
            var used: Set<String> = []
            var next: [String: Int] = [:]
            var pairs: [String] = []
            pairs.reserveCapacity(row.count)
            for (name, valueJSON) in row {
                var key = name
                if used.contains(key) {
                    var counter = next[name] ?? 0
                    var candidate: String
                    repeat {
                        counter += 1
                        candidate = "\(name)_\(counter)"
                    } while used.contains(candidate)
                    next[name] = counter
                    key = candidate
                }
                used.insert(key)
                pairs.append("\(jsonStringLiteral(key)):\(valueJSON)")
            }
            return "{\(pairs.joined(separator: ","))}"
        }
        return "[\(encodedRows.joined(separator: ","))]"
    }

    // MARK: - `params` decoding

    /// A SQL value tagged with its SQLite storage class (mirrors
    /// `tswift_swiftdata::db::DbValue` — see that module's doc for the full
    /// wire-encoding rationale).
    fileprivate enum DbValue {
        case null
        case int(Int64)
        case real(Double)
        case text(String)
        case blob([UInt8])
    }

    /// Decode `params` (a JSON-array-of-tagged-values `String`) into
    /// `[DbValue]`. Throws `TSwiftHostFunctionError` on malformed input — an
    /// ordinary catchable `$thrown` (see `db.rs`'s "malformed params is
    /// `$thrown`, not a bridge `Err`" rule), never a protocol-level failure.
    fileprivate static func decodeParams(_ text: String, op: String) throws -> [DbValue] {
        guard let data = text.data(using: .utf8),
              let parsed = try? JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed]),
              let array = parsed as? [Any]
        else {
            throw TSwiftHostFunctionError("\(op): params must be a JSON array")
        }
        return try array.map { try dbValue(from: $0, op: op) }
    }

    private static func dbValue(from node: Any, op: String) throws -> DbValue {
        guard let object = node as? [String: Any], object.count == 1, let (tag, payload) = object.first
        else {
            throw TSwiftHostFunctionError("\(op): db value must be a single-key tagged object")
        }
        switch tag {
        case "null":
            return .null
        case "int":
            guard let n = payload as? NSNumber else {
                throw TSwiftHostFunctionError("\(op): db value tagged `int` must carry a JSON number")
            }
            // `JSONSerialization` tags the `NSNumber` it produces with the
            // C type that matches how the literal actually parsed: `"q"`
            // (`long long`) only for an integer literal that fits exactly in
            // a *signed* 64-bit range. A fractional/exponent literal (`5.0`,
            // `1e10`), a `bool` literal (`true`/`false`, also bridges to
            // `NSNumber`), or an integer literal outside the `i64` range
            // (magnitude too large → `"Q"` unsigned 64-bit or, past that,
            // `NSDecimalNumber` → `"d"`) all get a *different* objCType —
            // never `"q"`. This mirrors the CLI's `Json::Int(i64)` strictness
            // (`text.parse::<i64>()`, which fails outright on a fractional or
            // out-of-range literal) without truncating/saturating a value
            // that doesn't actually fit.
            guard String(cString: n.objCType) == "q" else {
                throw TSwiftHostFunctionError(
                    "\(op): db value tagged `int` must carry a JSON number that is a whole "
                        + "number within the 64-bit signed integer range, got `\(n)`"
                )
            }
            return .int(n.int64Value)
        case "real":
            if let s = payload as? String {
                switch s {
                case "nan": return .real(.nan)
                case "inf": return .real(.infinity)
                case "-inf": return .real(-.infinity)
                case "-0", "-0.0": return .real(-0.0)
                default:
                    throw TSwiftHostFunctionError(
                        "\(op): db value tagged `real` has unknown sentinel string `\(s)`"
                    )
                }
            }
            guard let n = payload as? NSNumber else {
                throw TSwiftHostFunctionError(
                    "\(op): db value tagged `real` must carry a JSON number or sentinel string"
                )
            }
            return .real(n.doubleValue)
        case "text":
            guard let s = payload as? String else {
                throw TSwiftHostFunctionError("\(op): db value tagged `text` must carry a JSON string")
            }
            return .text(s)
        case "blob":
            guard let s = payload as? String, let data = Data(base64Encoded: s) else {
                throw TSwiftHostFunctionError("\(op): db value tagged `blob` has invalid base64")
            }
            return .blob([UInt8](data))
        default:
            throw TSwiftHostFunctionError("\(op): unknown db value tag `\(tag)`")
        }
    }
}
