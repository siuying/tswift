import Foundation
import TSwiftFFI

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

    private static func decode(_ raw: String) -> RunResult {
        guard let object = (try? JSONSerialization.jsonObject(with: Data(raw.utf8)))
            as? [String: Any]
        else {
            // A null return or unparseable envelope is an FFI-level failure;
            // surface the raw payload so it is not silently swallowed.
            let detail = raw.isEmpty ? "tswift_run returned null" : raw
            return RunResult(ok: false, stdout: "", diagnostics: detail, raw: raw)
        }
        let ok = object["ok"] as? Bool ?? false
        let run = object["run"] as? [String: Any]
        let stdout = run?["stdout"] as? String ?? ""
        let compile = object["compile"] as? [String: Any]
        let diagnostics = compile?["stderr"] as? String ?? ""
        return RunResult(ok: ok, stdout: stdout, diagnostics: diagnostics, raw: raw)
    }
}
