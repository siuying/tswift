import XCTest

@testable import TSwiftCore

final class TSwiftCoreTests: XCTestCase {
    func testRunPrintsToStdout() {
        let result = TSwiftCore.run(#"print("hi")"#)
        XCTAssertTrue(result.ok, result.raw)
        XCTAssertEqual(result.stdout, "hi\n")
    }

    func testCompileErrorIsNotOk() {
        let result = TSwiftCore.run(#"#error("boom")"#)
        XCTAssertFalse(result.ok, result.raw)
        XCTAssertTrue(result.diagnostics.contains("boom"), result.diagnostics)
    }

    func testContextReuseAcrossRuns() {
        let context = TSwiftContext()
        let first = TSwiftCore.run(#"print("one")"#, in: context)
        let second = TSwiftCore.run(#"print("two")"#, in: context)
        XCTAssertEqual(first.stdout, "one\n")
        XCTAssertEqual(second.stdout, "two\n")
    }
}
