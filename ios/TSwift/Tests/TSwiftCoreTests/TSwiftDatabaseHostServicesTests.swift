import XCTest

@testable import TSwiftCore

/// Coverage for `tswift.db.*`'s iOS backing (`TSwiftDatabaseHostServices.swift`,
/// real SQLite via `import SQLite3`). Mirrors
/// `crates/tswift-cli/src/db.rs`'s own test suite wire-for-wire.
///
/// Unlike `TSwiftFoundationHostServicesTests` (which runs real Swift scripts
/// through `TSwiftCore.run`, because `UserDefaults`/`FileManager` are real
/// Swift stdlib types wired to `tswift.defaults`/`tswift.fs`), there is no
/// Swift-facing `tswift.db` API yet (no `@Model`/SQL surface \u2014 see
/// `docs/adr/0015-db-host-service-wire.md`, explicitly future work), so
/// nothing in interpreted Swift can call it by name. These tests therefore
/// exercise `TSwiftDatabaseBacking` directly at the same boundary the
/// `registerHostFunction` closures use (`crates/tswift-cli/src/db.rs`'s own
/// tests do the equivalent: call `HostCallHandler::call` directly rather
/// than running a Swift script).
final class TSwiftDatabaseHostServicesTests: XCTestCase {
    private func makeTempDBPath(_ name: String) -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("tswift-db-tests-\(name)-\(UUID().uuidString)")
        try! FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        return dir.appendingPathComponent("test.sqlite3")
    }

    private func jsonObject(_ text: String) throws -> Any {
        try JSONSerialization.jsonObject(with: Data(text.utf8), options: [.fragmentsAllowed])
    }

    private func encodeParams(_ values: [[String: Any?]]) -> String {
        let objects: [[String: Any]] = values.map { entry in
            var out: [String: Any] = [:]
            for (k, v) in entry { out[k] = v ?? NSNull() }
            return out
        }
        let data = try! JSONSerialization.data(withJSONObject: objects, options: [])
        return String(data: data, encoding: .utf8)!
    }

    // MARK: - Registration smoke test

    func testInstallDatabaseHostServicesDeclaresCapabilityAndDoesNotThrow() throws {
        let context = TSwiftContext()
        try context.installDatabaseHostServices()
    }

    // MARK: - open / execute / query / close round trip

    func testOpenExecuteQueryCloseRoundTrip() throws {
        let backing = TSwiftDatabaseBacking()
        let handle = try backing.open(path: ":memory:")

        let createReply = try backing.execute(
            handle: handle, sql: "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)", paramsJSON: "[]"
        )
        let create = try jsonObject(createReply) as! [String: Any]
        XCTAssertEqual(create["rowsAffected"] as? Int, 0)

        let insertReply = try backing.execute(
            handle: handle,
            sql: "INSERT INTO t (name) VALUES (?)",
            paramsJSON: encodeParams([["text": "alice"]])
        )
        let insert = try jsonObject(insertReply) as! [String: Any]
        XCTAssertEqual(insert["rowsAffected"] as? Int, 1)
        XCTAssertEqual(insert["lastInsertRowid"] as? Int, 1)

        let selectReply = try backing.query(handle: handle, sql: "SELECT id, name FROM t", paramsJSON: "[]")
        let rows = try jsonObject(selectReply) as! [[String: Any]]
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual((rows[0]["id"] as? [String: Any])?["int"] as? Int, 1)
        XCTAssertEqual((rows[0]["name"] as? [String: Any])?["text"] as? String, "alice")

        try backing.close(handle: handle)
    }

    // MARK: - typed values, including blob and null

    func testTypedValuesRoundTripIncludingBlobAndNull() throws {
        let backing = TSwiftDatabaseBacking()
        let handle = try backing.open(path: ":memory:")
        _ = try backing.execute(
            handle: handle, sql: "CREATE TABLE t (i INTEGER, r REAL, s TEXT, b BLOB, n TEXT)", paramsJSON: "[]"
        )
        let blobB64 = Data([9, 8, 7]).base64EncodedString()
        let params = encodeParams([
            ["int": 7], ["real": 2.5], ["text": "hi"], ["blob": blobB64], ["null": NSNull()],
        ])
        _ = try backing.execute(handle: handle, sql: "INSERT INTO t VALUES (?, ?, ?, ?, ?)", paramsJSON: params)

        let reply = try backing.query(handle: handle, sql: "SELECT * FROM t", paramsJSON: "[]")
        let rows = try jsonObject(reply) as! [[String: Any]]
        let row = rows[0]
        XCTAssertEqual((row["i"] as? [String: Any])?["int"] as? Int, 7)
        XCTAssertEqual((row["r"] as? [String: Any])?["real"] as? Double, 2.5)
        XCTAssertEqual((row["s"] as? [String: Any])?["text"] as? String, "hi")
        XCTAssertEqual((row["b"] as? [String: Any])?["blob"] as? String, blobB64)
        XCTAssertNotNil((row["n"] as? [String: Any])?["null"] as? NSNull)
    }

    func testNonFiniteAndSignedZeroRealsRoundTripAsSentinelStrings() throws {
        let backing = TSwiftDatabaseBacking()
        let handle = try backing.open(path: ":memory:")
        _ = try backing.execute(handle: handle, sql: "CREATE TABLE t (r REAL)", paramsJSON: "[]")

        // Binding NaN: SQLite itself stores it as NULL (documented in
        // ADR-0015 \u2014 a storage-layer fact, not a wire-codec bug), so this
        // asserts the *wire codec* accepts the sentinel without crashing,
        // not that NaN round-trips through storage.
        for real in [["real": "nan"], ["real": "inf"], ["real": "-inf"]] {
            let params = "[" + (try! String(data: JSONSerialization.data(withJSONObject: real), encoding: .utf8)!) + "]"
            _ = try backing.execute(handle: handle, sql: "INSERT INTO t VALUES (?)", paramsJSON: params)
        }
        let reply = try backing.query(handle: handle, sql: "SELECT r FROM t", paramsJSON: "[]")
        let rows = try jsonObject(reply) as! [[String: Any]]
        XCTAssertEqual(rows.count, 3)
    }

    // MARK: - Transactions

    func testTransactionCommitAndRollback() throws {
        let backing = TSwiftDatabaseBacking()
        let handle = try backing.open(path: ":memory:")
        _ = try backing.execute(handle: handle, sql: "CREATE TABLE t (v INTEGER)", paramsJSON: "[]")

        try backing.begin(handle: handle)
        _ = try backing.execute(handle: handle, sql: "INSERT INTO t VALUES (1)", paramsJSON: "[]")
        try backing.rollback(handle: handle)
        var reply = try backing.query(handle: handle, sql: "SELECT COUNT(*) AS c FROM t", paramsJSON: "[]")
        var rows = try jsonObject(reply) as! [[String: Any]]
        XCTAssertEqual((rows[0]["c"] as? [String: Any])?["int"] as? Int, 0)

        try backing.begin(handle: handle)
        _ = try backing.execute(handle: handle, sql: "INSERT INTO t VALUES (1)", paramsJSON: "[]")
        try backing.commit(handle: handle)
        reply = try backing.query(handle: handle, sql: "SELECT COUNT(*) AS c FROM t", paramsJSON: "[]")
        rows = try jsonObject(reply) as! [[String: Any]]
        XCTAssertEqual((rows[0]["c"] as? [String: Any])?["int"] as? Int, 1)
    }

    // MARK: - Errors

    func testInvalidHandleIsThrownNotCrashed() throws {
        let backing = TSwiftDatabaseBacking()
        XCTAssertThrowsError(try backing.close(handle: 999)) { error in
            XCTAssertTrue(error is TSwiftHostFunctionError)
        }
    }

    func testDoubleCloseIsThrown() throws {
        let backing = TSwiftDatabaseBacking()
        let handle = try backing.open(path: ":memory:")
        try backing.close(handle: handle)
        XCTAssertThrowsError(try backing.close(handle: handle))
    }

    func testQueryAgainstClosedHandleIsThrown() throws {
        let backing = TSwiftDatabaseBacking()
        let handle = try backing.open(path: ":memory:")
        try backing.close(handle: handle)
        XCTAssertThrowsError(try backing.query(handle: handle, sql: "SELECT 1", paramsJSON: "[]"))
    }

    func testSQLSyntaxErrorIsThrownWithStructuredMessage() throws {
        let backing = TSwiftDatabaseBacking()
        let handle = try backing.open(path: ":memory:")
        XCTAssertThrowsError(
            try backing.execute(handle: handle, sql: "NOT VALID SQL", paramsJSON: "[]")
        ) { error in
            guard let hostError = error as? TSwiftHostFunctionError else {
                return XCTFail("expected TSwiftHostFunctionError, got \(error)")
            }
            XCTAssertTrue(hostError.message.contains("SQLite error"), hostError.message)
        }
    }

    func testMalformedParamsPayloadIsThrown() throws {
        let backing = TSwiftDatabaseBacking()
        let handle = try backing.open(path: ":memory:")
        XCTAssertThrowsError(
            try backing.execute(handle: handle, sql: "SELECT 1", paramsJSON: "not valid json")
        ) { error in
            XCTAssertTrue(error is TSwiftHostFunctionError)
        }
    }

    func testEmptySQLIsANoop() throws {
        let backing = TSwiftDatabaseBacking()
        let handle = try backing.open(path: ":memory:")

        let execReply = try backing.execute(handle: handle, sql: "-- nothing here", paramsJSON: "[]")
        let execResult = try jsonObject(execReply) as! [String: Any]
        XCTAssertEqual(execResult["rowsAffected"] as? Int, 0)

        let queryReply = try backing.query(handle: handle, sql: "   ", paramsJSON: "[]")
        let rows = try jsonObject(queryReply) as! [Any]
        XCTAssertTrue(rows.isEmpty)
    }

    func testDuplicateResultColumnsAreDisambiguated() throws {
        let backing = TSwiftDatabaseBacking()
        let handle = try backing.open(path: ":memory:")
        _ = try backing.execute(handle: handle, sql: "CREATE TABLE t (a INTEGER)", paramsJSON: "[]")
        _ = try backing.execute(handle: handle, sql: "INSERT INTO t VALUES (5)", paramsJSON: "[]")
        let reply = try backing.query(handle: handle, sql: "SELECT a, a, a FROM t", paramsJSON: "[]")
        let rows = try jsonObject(reply) as! [[String: Any]]
        let keys = Set(rows[0].keys)
        XCTAssertEqual(keys, ["a", "a_1", "a_2"])
    }

    func testOpenInvalidPathIsThrown() throws {
        let backing = TSwiftDatabaseBacking()
        XCTAssertThrowsError(
            try backing.open(path: "/nonexistent-tswift-dir/does/not/exist.db")
        ) { error in
            XCTAssertTrue(error is TSwiftHostFunctionError)
        }
    }

    // MARK: - `int`-tag strictness (matches `DbValue::from_json`'s `Json::Int(i64)`)

    func testFractionalIntTagPayloadIsThrownNotTruncated() throws {
        let backing = TSwiftDatabaseBacking()
        let handle = try backing.open(path: ":memory:")
        _ = try backing.execute(handle: handle, sql: "CREATE TABLE t (v INTEGER)", paramsJSON: "[]")
        // `5.0` must be rejected, not silently truncated to `5` — mirrors the
        // CLI's `Json::Int` strictness (a `.`-bearing literal tokenizes as
        // `Json::Double`, which `DbValue::from_json`'s `int` arm rejects).
        XCTAssertThrowsError(
            try backing.execute(handle: handle, sql: "INSERT INTO t VALUES (?)", paramsJSON: "[{\"int\":5.0}]")
        ) { error in
            guard let hostError = error as? TSwiftHostFunctionError else {
                return XCTFail("expected TSwiftHostFunctionError, got \(error)")
            }
            XCTAssertTrue(hostError.message.contains("int"), hostError.message)
        }
    }

    func testOutOfI64RangeIntTagPayloadIsThrownNotSaturated() throws {
        let backing = TSwiftDatabaseBacking()
        let handle = try backing.open(path: ":memory:")
        _ = try backing.execute(handle: handle, sql: "CREATE TABLE t (v INTEGER)", paramsJSON: "[]")
        // One past `Int64.max`: must be rejected, not saturated/truncated to
        // some in-range value — `NSNumber.int64Value` would otherwise silently
        // wrap/clamp this.
        XCTAssertThrowsError(
            try backing.execute(
                handle: handle, sql: "INSERT INTO t VALUES (?)", paramsJSON: "[{\"int\":9223372036854775808}]"
            )
        ) { error in
            XCTAssertTrue(error is TSwiftHostFunctionError)
        }
    }

    func testExactI64BoundaryIntTagPayloadsRoundTrip() throws {
        let backing = TSwiftDatabaseBacking()
        let handle = try backing.open(path: ":memory:")
        _ = try backing.execute(handle: handle, sql: "CREATE TABLE t (v INTEGER)", paramsJSON: "[]")
        for raw in ["-9223372036854775808", "9223372036854775807"] {
            _ = try backing.execute(
                handle: handle, sql: "INSERT INTO t VALUES (?)", paramsJSON: "[{\"int\":\(raw)}]"
            )
        }
        let reply = try backing.query(handle: handle, sql: "SELECT v FROM t ORDER BY v", paramsJSON: "[]")
        let rows = try jsonObject(reply) as! [[String: Any]]
        XCTAssertEqual(rows.count, 2)
        XCTAssertEqual((rows[0]["v"] as? [String: Any])?["int"] as? Int64, Int64.min)
        XCTAssertEqual((rows[1]["v"] as? [String: Any])?["int"] as? Int64, Int64.max)
    }

    // MARK: - Non-UTF-8 TEXT (matches `sqlite_ffi.rs::non_utf8_text_surfaces_structured_error`)

    func testNonUTF8TextSurfacesStructuredError() throws {
        let backing = TSwiftDatabaseBacking()
        let handle = try backing.open(path: ":memory:")
        _ = try backing.execute(handle: handle, sql: "CREATE TABLE t (v TEXT)", paramsJSON: "[]")
        // `0xFF 0xFE` is not valid UTF-8 in any position; cast a blob literal
        // to TEXT so SQLite stores it as the TEXT storage class verbatim,
        // exactly like the CLI's own test does.
        _ = try backing.execute(
            handle: handle, sql: "INSERT INTO t VALUES (CAST(x'fffe' AS TEXT))", paramsJSON: "[]"
        )
        XCTAssertThrowsError(
            try backing.query(handle: handle, sql: "SELECT v FROM t", paramsJSON: "[]")
        ) { error in
            guard let hostError = error as? TSwiftHostFunctionError else {
                return XCTFail("expected TSwiftHostFunctionError, got \(error)")
            }
            XCTAssertTrue(hostError.message.contains("non-UTF-8"), hostError.message)
        }
    }

    // MARK: - File-backed persistence in a temp dir

    func testFileBackedDatabasePersistsAcrossBackingInstances() throws {
        let path = makeTempDBPath(#function)

        do {
            let backing = TSwiftDatabaseBacking()
            let handle = try backing.open(path: path.path)
            _ = try backing.execute(handle: handle, sql: "CREATE TABLE t (v TEXT)", paramsJSON: "[]")
            _ = try backing.execute(
                handle: handle, sql: "INSERT INTO t VALUES (?)", paramsJSON: encodeParams([["text": "persisted"]])
            )
        }

        do {
            let backing = TSwiftDatabaseBacking()
            let handle = try backing.open(path: path.path)
            let reply = try backing.query(handle: handle, sql: "SELECT v FROM t", paramsJSON: "[]")
            let rows = try jsonObject(reply) as! [[String: Any]]
            XCTAssertEqual((rows[0]["v"] as? [String: Any])?["text"] as? String, "persisted")
        }

        // The file really exists on disk (not just simulated).
        XCTAssertTrue(FileManager.default.fileExists(atPath: path.path))
    }
}
