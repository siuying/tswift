import Foundation
import TSwiftCore
import TSwiftFFI
import UiirRenderer

/// Drives a live SwiftUI preview: compiles a program into an initial UIIR tree,
/// then routes interaction events into the native render session and applies the
/// returned patch stream to a `RenderModel` in place (preserving focus, scroll,
/// in-flight drags). Owns a `TSwiftContext` for its lifetime.
///
/// Wire it to `RenderHostView` by passing `model` and installing
/// `eventSink` via `.uiirEventSink(_:)`.
@MainActor
public final class PreviewSession: ObservableObject {
    /// The current UIIR tree, mutated in place by dispatched patches.
    @Published public private(set) var model: RenderModel
    /// The root view type name from the last successful compile.
    @Published public private(set) var root: String?
    /// The error from the last failed compile/dispatch, if any.
    @Published public private(set) var lastError: String?
    /// Frontend diagnostics for the last linted source (empty when it's clean).
    /// Updated by `diagnose(_:)` independently of the live preview, so the
    /// editor can surface syntax/sema errors as the user types.
    @Published public private(set) var diagnostics: [Diagnostic] = []

    private let context: TSwiftContext

    public init(context: TSwiftContext = TSwiftContext()) {
        self.context = context
        self.model = RenderModel(root: UiirNode(id: "", kind: "Empty"))
    }

    /// An event sink that forwards rendered-control interactions into `dispatch`.
    public func makeEventSink() -> any UiirEventSink {
        ClosureEventSink { [weak self] event in
            self?.dispatch(event)
        }
    }

    // MARK: Compile

    private struct CompileEnvelope: Decodable {
        let ok: Bool
        let root: String?
        let tree: UiirNode?
        let error: String?
    }

    /// Compile `source`, mount its initial UIIR tree, and start a render session.
    public func compile(_ source: String) {
        let raw = source.withCString { cSource -> String in
            guard let ptr = tswift_swiftui_compile(context.handle, cSource) else { return "" }
            defer { tswift_string_free(ptr) }
            return String(cString: ptr)
        }
        let envelope: CompileEnvelope
        do {
            envelope = try JSONDecoder().decode(CompileEnvelope.self, from: Data(raw.utf8))
        } catch {
            lastError = raw.isEmpty
                ? "tswift_swiftui_compile returned null"
                : "failed to decode compile result: \(error) — raw: \(raw)"
            return
        }
        guard envelope.ok, let tree = envelope.tree else {
            // Keep the last good model/root for preview continuity; only the
            // error surfaces.
            lastError = envelope.error ?? "compile failed"
            return
        }
        root = envelope.root
        lastError = nil
        model = RenderModel(root: tree)
        // Fire `.task {}` closures — appear-time async work that runs inline
        // under the cooperative executor — and apply the resulting patches.
        runMountTasks()
    }

    /// Fire any pending `.task {}` closures on the live session and apply the
    /// returned patches. Mirrors `dispatch(...)`'s FFI-call pattern; a no-op
    /// (empty patch list) when the mounted view has no `.task` modifiers.
    public func runMountTasks() {
        guard let ptr = tswift_swiftui_run_mount_tasks(context.handle) else { return }
        defer { tswift_string_free(ptr) }
        let raw = String(cString: ptr)
        guard let envelope = try? JSONDecoder().decode(
            DispatchEnvelope.self, from: Data(raw.utf8)
        ), envelope.ok, let patches = envelope.patches else { return }
        model.apply(patches)
    }

    // MARK: Diagnostics

    /// One frontend diagnostic, with its position mapped back to the user's
    /// source (1-based line/column) and severity.
    public struct Diagnostic: Decodable, Identifiable, Equatable {
        public enum Severity: String, Decodable { case error, warning }
        public let line: Int
        public let col: Int
        public let message: String
        public let severity: Severity
        public var id: String { "\(line):\(col):\(severity.rawValue):\(message)" }
        public var isError: Bool { severity == .error }
    }

    private struct DiagnosticsEnvelope: Decodable {
        let ok: Bool
        let diagnostics: [Diagnostic]
    }

    /// Lint `source` through the frontend and publish its diagnostics, **without**
    /// rendering — the editor's live error-feedback channel. Decode failures are
    /// surfaced as a single synthetic error so the UI never silently drops them.
    public func diagnose(_ source: String) {
        let raw = source.withCString { cSource -> String in
            guard let ptr = tswift_diagnostics(cSource) else { return "" }
            defer { tswift_string_free(ptr) }
            return String(cString: ptr)
        }
        guard let envelope = try? JSONDecoder().decode(
            DiagnosticsEnvelope.self, from: Data(raw.utf8)
        ) else {
            diagnostics = [Diagnostic(
                line: 1, col: 1,
                message: raw.isEmpty ? "tswift_diagnostics returned null"
                                     : "failed to decode diagnostics",
                severity: .error
            )]
            return
        }
        diagnostics = envelope.diagnostics
    }

    // MARK: Dispatch

    private struct DispatchEnvelope: Decodable {
        let ok: Bool
        let patches: [Patch]?
        let error: String?
    }

    /// Route a rendered-control event into the live session.
    public func dispatch(_ event: UiirEvent) {
        dispatch(id: event.id, event: event.event, value: event.value)
    }

    /// Route an event by its parts. `value` is a raw JSON scalar (`"true"`,
    /// `"42"`, `"\"hi\""`) or `""` for a payload-less tap.
    public func dispatch(id: String, event: String, value: String) {
        guard value.isEmpty || Self.isJSONScalar(value) else {
            lastError = "dispatch value must be a JSON scalar (got: \(value))"
            return
        }
        let eventJSON = Self.eventJSON(id: id, event: event, value: value)
        let raw = eventJSON.withCString { cEvent -> String in
            guard let ptr = tswift_swiftui_dispatch(context.handle, cEvent) else { return "" }
            defer { tswift_string_free(ptr) }
            return String(cString: ptr)
        }
        let envelope: DispatchEnvelope
        do {
            envelope = try JSONDecoder().decode(DispatchEnvelope.self, from: Data(raw.utf8))
        } catch {
            lastError = raw.isEmpty
                ? "tswift_swiftui_dispatch returned null"
                : "failed to decode dispatch result: \(error) — raw: \(raw)"
            return
        }
        guard envelope.ok, let patches = envelope.patches else {
            lastError = envelope.error ?? "dispatch failed"
            return
        }
        lastError = nil
        model.apply(patches)
    }

    /// Build the `{"id","event","value"?}` envelope. `value` is injected raw
    /// (it is already a JSON scalar); `id`/`event` are JSON-encoded.
    static func eventJSON(id: String, event: String, value: String) -> String {
        let idJSON = jsonScalar(id)
        let eventJSON = jsonScalar(event)
        if value.isEmpty {
            return "{\"id\":\(idJSON),\"event\":\(eventJSON)}"
        }
        return "{\"id\":\(idJSON),\"event\":\(eventJSON),\"value\":\(value)}"
    }

    private static func jsonScalar(_ s: String) -> String {
        if let data = try? JSONSerialization.data(withJSONObject: s, options: [.fragmentsAllowed]),
           let encoded = String(data: data, encoding: .utf8) {
            return encoded
        }
        return "\"\""
    }

    /// Whether `value` is a single JSON scalar (string/number/bool/null) rather
    /// than a compound array/object — the dispatch contract for `value`.
    static func isJSONScalar(_ value: String) -> Bool {
        guard let parsed = try? JSONSerialization.jsonObject(
            with: Data(value.utf8), options: [.fragmentsAllowed]
        ) else {
            return false
        }
        return !(parsed is [Any]) && !(parsed is [String: Any])
    }
}
