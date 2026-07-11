import XCTest

@testable import TSwiftCore

/// End-to-end coverage for `tswift.defaults.*` / `tswift.fs.*` running real
/// Swift scripts through the Foundation-backed host services (see
/// `TSwiftFoundationHostServices.swift`). Uses a private `UserDefaults` suite
/// and a temp directory so tests never touch real app state and are
/// order/parallel-safe.
final class TSwiftFoundationHostServicesTests: XCTestCase {
    private func makeSuite(_ name: String) -> UserDefaults {
        let suiteName = "tswift-tests-\(name)-\(UUID().uuidString)"
        let suite = UserDefaults(suiteName: suiteName)!
        addTeardownBlock { UserDefaults().removePersistentDomain(forName: suiteName) }
        return suite
    }

    private func makeTempDir(_ name: String) -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("tswift-tests-\(name)-\(UUID().uuidString)")
        try! FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        return dir
    }

    func testDeclareHostServiceAcceptsKnownNamespace() throws {
        let context = TSwiftContext()
        try context.declareHostService("tswift.defaults")
        try context.declareHostService("tswift.fs")
    }

    func testDeclareHostServiceRejectsUnknownNamespace() {
        let context = TSwiftContext()
        XCTAssertThrowsError(try context.declareHostService("tswift.nope"))
    }

    func testUserDefaultsSetGetRemoveRoundTrip() throws {
        let context = TSwiftContext()
        let suite = makeSuite(#function)
        try context.installFoundationHostServices(defaults: suite)

        let script = """
        import Foundation
        let d = UserDefaults.standard
        d.set("Swift", forKey: "language")
        d.set(6, forKey: "version")
        print(d.string(forKey: "language") ?? "nil")
        print(d.integer(forKey: "version"))
        d.removeObject(forKey: "language")
        print(d.string(forKey: "language") ?? "nil")
        """
        let result = TSwiftCore.run(script, in: context)
        XCTAssertTrue(result.ok, result.raw)
        XCTAssertEqual(result.stdout, "Swift\n6\nnil\n")

        // The real suite actually received the write (not just the runtime's
        // own bookkeeping) — this is the "delegates to real Foundation" claim.
        // Values cross the `tswift.defaults` wire as their JSON encoding (see
        // `crates/tswift-foundation/src/user_defaults.rs`), stored verbatim;
        // the suite's raw stored value for an `Int` is therefore the digit
        // string "6", which `UserDefaults.integer(forKey:)`'s NSString
        // bridging coerces back to `6`.
        XCTAssertEqual(suite.string(forKey: "version"), "6")
        XCTAssertEqual(suite.integer(forKey: "version"), 6)
    }

    func testFileManagerWriteReadRemoveRoundTrip() throws {
        let context = TSwiftContext()
        let fm = FileManager.default
        let dir = makeTempDir(#function)
        try context.installFoundationHostServices(fileManager: fm)

        let filePath = dir.appendingPathComponent("greeting.txt").path
        let script = """
        import Foundation
        let path = "\(filePath)"
        try! "hello from a script".write(toFile: path, atomically: true, encoding: .utf8)
        print(FileManager.default.fileExists(atPath: path))
        print(try! String(contentsOfFile: path))
        try! FileManager.default.removeItem(atPath: path)
        print(FileManager.default.fileExists(atPath: path))
        """
        let result = TSwiftCore.run(script, in: context)
        XCTAssertTrue(result.ok, result.raw)
        XCTAssertEqual(result.stdout, "true\nhello from a script\nfalse\n")

        // The file really existed on disk mid-script (not just simulated).
        XCTAssertFalse(fm.fileExists(atPath: filePath))
    }

    func testFileManagerRemoveMissingFileThrowsCatchableError() throws {
        let context = TSwiftContext()
        let dir = makeTempDir(#function)
        try context.installFoundationHostServices()

        let missing = dir.appendingPathComponent("nope.txt").path
        let script = """
        import Foundation
        struct AnyErr: Error {}
        do {
            try FileManager.default.removeItem(atPath: "\(missing)")
            print("unexpected success")
        } catch {
            print("caught")
        }
        """
        let result = TSwiftCore.run(script, in: context)
        XCTAssertTrue(result.ok, result.raw)
        XCTAssertEqual(result.stdout, "caught\n")
    }

    func testFileManagerListsDirectoryEntries() throws {
        let context = TSwiftContext()
        let dir = makeTempDir(#function)
        let fm = FileManager.default
        try context.installFoundationHostServices(fileManager: fm)

        try "a".write(toFile: dir.appendingPathComponent("a.txt").path, atomically: true, encoding: .utf8)
        try "b".write(toFile: dir.appendingPathComponent("b.txt").path, atomically: true, encoding: .utf8)

        let script = """
        import Foundation
        let names = try! FileManager.default.contentsOfDirectory(atPath: "\(dir.path)")
        print(names.sorted())
        """
        let result = TSwiftCore.run(script, in: context)
        XCTAssertTrue(result.ok, result.raw)
        XCTAssertEqual(result.stdout, "[\"a.txt\", \"b.txt\"]\n")
    }
}
