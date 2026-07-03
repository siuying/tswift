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

    func testHTTPHandlerServesScriptURLSession() {
        let context = TSwiftContext()
        context.setHTTPHandler { request in
            XCTAssertEqual(request.method, "GET")
            XCTAssertEqual(request.url, "https://api.example.com/greeting")
            return .response(
                status: 200,
                headers: [("Content-Type", "text/plain")],
                body: Data("hello from host".utf8)
            )
        }
        let script = """
        import Foundation
        let (data, resp) = try await URLSession.shared.data(
            from: URL(string: "https://api.example.com/greeting")!)
        print((resp as! HTTPURLResponse).statusCode)
        print(String(data: data, encoding: .utf8) ?? "nil")
        """
        let result = TSwiftCore.run(script, in: context)
        XCTAssertTrue(result.ok, result.raw)
        XCTAssertEqual(result.stdout, "200\nhello from host\n")
    }

    func testHTTPFailureThrowsURLErrorInScript() {
        let context = TSwiftContext()
        context.setHTTPHandler { _ in
            .failure(code: "timedOut", message: "scripted timeout")
        }
        let script = """
        import Foundation
        do {
            _ = try await URLSession.shared.data(
                from: URL(string: "https://slow.example.com/")!)
            print("unexpected success")
        } catch let error as URLError {
            print("caught \\(error.errorCode)")
        }
        """
        let result = TSwiftCore.run(script, in: context)
        XCTAssertTrue(result.ok, result.raw)
        XCTAssertEqual(result.stdout, "caught -1001\n")
    }

    func testRemovedHandlerMakesURLSessionUnavailable() {
        let context = TSwiftContext()
        context.setHTTPHandler { _ in
            .response(status: 200, headers: [], body: Data())
        }
        context.removeHTTPHandler()
        let script = """
        import Foundation
        _ = try await URLSession.shared.data(from: URL(string: "https://x.example/")!)
        """
        let result = TSwiftCore.run(script, in: context)
        XCTAssertFalse(result.ok, result.raw)
    }
}
