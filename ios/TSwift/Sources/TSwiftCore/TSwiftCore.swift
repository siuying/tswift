import Foundation
import TSwiftFFI

// MARK: - Module types

/// A single named Swift source file in a multi-file module.
public struct TSwiftSourceFile: Sendable {
    /// The file's name (e.g. `"main.swift"`, `"helpers.swift"`). Diagnostics
    /// are attributed per file: an error reports its own `path:line:col`.
    public let path: String
    /// The file's Swift source text.
    public let contents: String

    public init(path: String, contents: String) {
        self.path = path
        self.contents = contents
    }

    /// JSON representation: `{"path":"…","contents":"…"}` (values JSON-encoded).
    public var jsonObject: Any {
        ["path": path, "contents": contents]
    }
}

/// An ordered collection of named Swift source files.
public struct TSwiftModule: Sendable {
    public let files: [TSwiftSourceFile]

    public init(files: [TSwiftSourceFile]) {
        self.files = files
    }

    /// Encode to the FFI wire payload: `{"files":[{"path":…,"contents":…},…]}`.
    public func toJSON() -> String {
        let arr = files.map(\.jsonObject)
        let dict: [String: Any] = ["files": arr]
        guard let data = try? JSONSerialization.data(withJSONObject: dict),
              let s = String(data: data, encoding: .utf8)
        else { return "{\"files\":[]}" }
        return s
    }
}

// MARK: - Symbols

/// One declaration symbol discovered by the frontend across a module — the
/// unit an outline/jump-to-symbol view is built from.
public struct TSwiftSymbol: Decodable, Identifiable, Sendable, Equatable {
    /// The declared name (e.g. `CounterView`, `body`, `increment`).
    public let name: String
    /// The lowercase Swift keyword for the declaration (`struct`, `func`,
    /// `var`, `let`, `enum`, `class`, `case`, `init`, `subscript`, …).
    public let kind: String
    /// The source file the declaration lives in (a module file `path`).
    public let file: String
    /// The 1-based line the declaration starts on within `file`.
    public let line: Int
    /// The nearest enclosing container declaration's name, if nested.
    public let container: String?
    /// A cheap one-line signature preview, if the frontend produced one.
    public let signature: String?

    /// Stable identity for SwiftUI lists: file + line + name uniquely locate a
    /// declaration (two symbols can't start at the same line in one file).
    public var id: String { "\(file):\(line):\(name)" }
}

extension TSwiftCore {
    /// The decoded result of a `listSymbols` call.
    public struct SymbolsResult: Sendable {
        /// Whether the module JSON itself parsed (individual files with syntax
        /// errors are skipped, not fatal — they just contribute no symbols).
        public let ok: Bool
        /// Every declaration symbol found across the module's files, in source
        /// order per file.
        public let symbols: [TSwiftSymbol]
        /// The error message when `ok` is false (malformed module JSON).
        public let error: String?
    }

    /// List every declaration symbol across a multi-file `module`.
    ///
    /// Stateless (needs no `TSwiftContext`): each file is analyzed
    /// independently, so a syntax error in one file does not block symbols
    /// from the others. Backed by the `tswift_list_symbols` C ABI.
    public static func listSymbols(module: TSwiftModule) -> SymbolsResult {
        let moduleJSON = module.toJSON()
        let raw = moduleJSON.withCString { cJSON -> String in
            guard let ptr = tswift_list_symbols(cJSON) else { return "" }
            defer { tswift_string_free(ptr) }
            return String(cString: ptr)
        }
        return decodeSymbols(raw)
    }

    private struct SymbolsEnvelope: Decodable {
        let ok: Bool
        let symbols: [TSwiftSymbol]
        let error: String?
    }

    private static func decodeSymbols(_ raw: String) -> SymbolsResult {
        guard let envelope = try? JSONDecoder().decode(
            SymbolsEnvelope.self, from: Data(raw.utf8)
        ) else {
            let detail = raw.isEmpty ? "tswift_list_symbols returned null" : raw
            return SymbolsResult(ok: false, symbols: [], error: detail)
        }
        return SymbolsResult(ok: envelope.ok, symbols: envelope.symbols, error: envelope.error)
    }
}

// MARK: - TSwiftCore

/// One-shot "compile a Swift program and run it for its stdout" façade.
public enum TSwiftCore {
    /// The decoded result of a `run`.
    public struct RunResult: Sendable {
        /// Whether compilation and execution both succeeded.
        public let ok: Bool
        /// Captured standard output.
        public let stdout: String
        /// Compiler diagnostics (warnings, or the error on failure).
        public let diagnostics: String
        /// The raw result-JSON envelope, for callers that want everything.
        public let raw: String
    }

    /// Compile and run a multi-file `module`, returning its decoded result.
    ///
    /// The ordered files form one compilation unit; each diagnostic reports its
    /// own file (`path:line:col`). Only `main.swift` (or a single-file program)
    /// may contain top-level executable statements. Creates a fresh context per
    /// call by default; pass an existing `context` to reuse interpreter state.
    public static func run(
        module: TSwiftModule,
        in context: TSwiftContext = TSwiftContext()
    ) -> RunResult {
        let moduleJSON = module.toJSON()
        let raw = moduleJSON.withCString { cJSON -> String in
            guard let ptr = tswift_run_module(context.handle, cJSON) else { return "" }
            defer { tswift_string_free(ptr) }
            return String(cString: ptr)
        }
        return decode(raw)
    }

    /// Compile and run `source`, returning its decoded result.
    ///
    /// Creates a fresh context per call by default; pass an existing `context`
    /// to reuse interpreter state (e.g. the fragment cache) across runs.
    public static func run(
        _ source: String,
        in context: TSwiftContext = TSwiftContext()
    ) -> RunResult {
        let raw = source.withCString { cSource -> String in
            guard let ptr = tswift_run(context.handle, cSource) else { return "" }
            defer { tswift_string_free(ptr) }
            return String(cString: ptr)
        }
        return decode(raw)
    }

    /// The result-JSON envelope returned by `tswift_run`.
    private struct RunEnvelope: Decodable {
        struct Compile: Decodable { let ok: Bool; let stderr: String }
        struct Run: Decodable { let ok: Bool; let stdout: String }
        let ok: Bool
        let compile: Compile?
        let run: Run?
    }

    private static func decode(_ raw: String) -> RunResult {
        let envelope: RunEnvelope
        do {
            envelope = try JSONDecoder().decode(RunEnvelope.self, from: Data(raw.utf8))
        } catch {
            // A null return or unparseable envelope is an FFI-level failure;
            // surface the raw payload so it is not silently swallowed.
            let detail = raw.isEmpty ? "tswift_run returned null" : raw
            return RunResult(ok: false, stdout: "", diagnostics: detail, raw: raw)
        }
        return RunResult(
            ok: envelope.ok,
            stdout: envelope.run?.stdout ?? "",
            diagnostics: envelope.compile?.stderr ?? "",
            raw: raw
        )
    }
}
