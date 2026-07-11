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

    func testRunModuleResolvesCrossFile() {
        let module = TSwiftModule(files: [
            TSwiftSourceFile(path: "models.swift", contents: "struct Point { let x: Int }\n"),
            TSwiftSourceFile(path: "main.swift", contents: "let p = Point(x: 8)\nprint(p.x)\n"),
        ])
        let result = TSwiftCore.run(module: module)
        XCTAssertTrue(result.ok, result.raw)
        XCTAssertEqual(result.stdout, "8\n")
    }

    func testRunModuleDiagnosticNamesSecondFile() {
        let module = TSwiftModule(files: [
            TSwiftSourceFile(path: "a.swift", contents: "struct A {}\nstruct B {}\n"),
            TSwiftSourceFile(path: "main.swift", contents: "let x = 1\n#error(\"boom\")\n"),
        ])
        let result = TSwiftCore.run(module: module)
        XCTAssertFalse(result.ok, result.raw)
        XCTAssertTrue(result.diagnostics.contains("main.swift:2:"), result.diagnostics)
        XCTAssertTrue(result.diagnostics.contains("boom"), result.diagnostics)
    }

    func testRunModuleRejectsTopLevelOutsideMain() {
        let module = TSwiftModule(files: [
            TSwiftSourceFile(path: "helpers.swift", contents: "func f() {}\nprint(\"nope\")\n"),
            TSwiftSourceFile(path: "main.swift", contents: "f()\n"),
        ])
        let result = TSwiftCore.run(module: module)
        XCTAssertFalse(result.ok, result.raw)
        XCTAssertTrue(result.diagnostics.contains("helpers.swift"), result.diagnostics)
    }

    func testListSymbolsAcrossFiles() {
        let module = TSwiftModule(files: [
            TSwiftSourceFile(
                path: "models.swift",
                contents: "struct Point {\n    let x: Int\n    func moved() -> Point { self }\n}\n"
            ),
            TSwiftSourceFile(path: "main.swift", contents: "func greet() {}\n"),
        ])
        let result = TSwiftCore.listSymbols(module: module)
        XCTAssertTrue(result.ok, result.error ?? "")
        let byName = Dictionary(grouping: result.symbols, by: \.name)
        XCTAssertNotNil(byName["Point"], "expected the Point struct")
        XCTAssertEqual(byName["Point"]?.first?.kind, "struct")
        XCTAssertEqual(byName["Point"]?.first?.file, "models.swift")
        // A nested member reports its container.
        XCTAssertEqual(byName["x"]?.first?.container, "Point")
        // A symbol from the second file is listed too.
        XCTAssertNotNil(byName["greet"], "expected greet() from main.swift")
        XCTAssertEqual(byName["greet"]?.first?.file, "main.swift")
    }

    func testListSymbolsMalformedModuleIsNotOk() {
        // A file with a syntax error must not abort the whole listing; a
        // well-formed sibling still contributes its symbols.
        let module = TSwiftModule(files: [
            TSwiftSourceFile(path: "broken.swift", contents: "struct {{{ "),
            TSwiftSourceFile(path: "ok.swift", contents: "struct Good {}\n"),
        ])
        let result = TSwiftCore.listSymbols(module: module)
        XCTAssertTrue(result.ok, result.error ?? "")
        XCTAssertTrue(result.symbols.contains { $0.name == "Good" }, "expected Good")
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

    // MARK: - Host-native functions (Epic #246)

    func testHostFunctionCallableFromScript() throws {
        let context = TSwiftContext()
        try context.registerHostFunction(
            .init(name: "hostDeviceName", returns: .string)
        ) { _ in "iPhone 42" }
        let result = TSwiftCore.run("print(hostDeviceName())", in: context)
        XCTAssertTrue(result.ok, result.raw)
        XCTAssertEqual(result.stdout, "iPhone 42\n")
    }

    func testHostFunctionReceivesLabeledArgument() throws {
        let context = TSwiftContext()
        try context.registerHostFunction(
            .init(
                name: "hostHaptic",
                parameters: [.init(label: "style", type: .string)],
                returns: .string
            )
        ) { args in
            let style = args.first as? String ?? "none"
            return "did \(style)"
        }
        let result = TSwiftCore.run(#"print(hostHaptic(style: "tap"))"#, in: context)
        XCTAssertTrue(result.ok, result.raw)
        XCTAssertEqual(result.stdout, "did tap\n")
    }

    func testHostFunctionVoidReturn() throws {
        let context = TSwiftContext()
        try context.registerHostFunction(
            .init(name: "hostLog", parameters: [.init(label: "message", type: .string)])
        ) { _ in nil }
        let result = TSwiftCore.run(#"hostLog(message: "hi"); print("ok")"#, in: context)
        XCTAssertTrue(result.ok, result.raw)
        XCTAssertEqual(result.stdout, "ok\n")
    }

    func testHostFunctionThrowsCatchableError() throws {
        let context = TSwiftContext()
        try context.registerHostFunction(
            .init(name: "hostRisky", returns: .int, throwing: true)
        ) { _ in throw TSwiftHostFunctionError("boom") }
        let script = """
        do {
            _ = try hostRisky()
            print("unexpected")
        } catch {
            print("caught")
        }
        """
        let result = TSwiftCore.run(script, in: context)
        XCTAssertTrue(result.ok, result.raw)
        XCTAssertEqual(result.stdout, "caught\n")
    }

    func testRemovedHostFunctionIsNotCallable() throws {
        let context = TSwiftContext()
        try context.registerHostFunction(
            .init(name: "hostDeviceName", returns: .string)
        ) { _ in "iPhone" }
        context.removeHostFunction(named: "hostDeviceName")
        let result = TSwiftCore.run("print(hostDeviceName())", in: context)
        XCTAssertFalse(result.ok, result.raw)
    }

    func testReplacedHostFunctionUsesLatestClosure() throws {
        let context = TSwiftContext()
        try context.registerHostFunction(
            .init(name: "hostDeviceName", returns: .string)
        ) { _ in "first" }
        try context.registerHostFunction(
            .init(name: "hostDeviceName", returns: .string)
        ) { _ in "second" }
        let result = TSwiftCore.run("print(hostDeviceName())", in: context)
        XCTAssertTrue(result.ok, result.raw)
        XCTAssertEqual(result.stdout, "second\n")
    }

    func testHostFunctionMalformedSignatureThrows() {
        let context = TSwiftContext()
        XCTAssertThrowsError(
            try context.registerHostFunction(.init(name: "")) { _ in nil }
        )
    }
}
