import Foundation
import TSwiftFFI

/// The lifespan-owning VM handle (the QuickJS `JSContext` analogue). Owns the
/// native interpreter bundle; a host may reuse one across runs so the fragment
/// cache and installed stdlib persist. Freed automatically on `deinit`.
///
/// Not thread-safe: confine a context to a single thread/actor.
public final class TSwiftContext {
    /// Opaque pointer to the Rust `Context`. Never null while this object lives.
    /// `package`-visible so the sibling `TSwiftUI` module can drive the same
    /// context, without exposing the raw pointer to external consumers.
    package let handle: OpaquePointer

    /// Retains the registered HTTP handler box while the native side holds a
    /// borrowed pointer to it (see `TSwiftHTTP.swift`). Internal on purpose.
    var httpHandlerBox: AnyObject?

    /// Retains the registered host-function handler boxes (keyed by name) while
    /// the native side holds borrowed pointers to them (see
    /// `TSwiftHostFunction.swift`). Released when the box is removed/replaced or
    /// when the context deinits — so nothing leaks across runs. Internal on
    /// purpose.
    var hostFunctionBoxes: [String: AnyObject] = [:]

    public init() {
        guard let handle = tswift_context_new() else {
            fatalError("tswift_context_new returned null")
        }
        self.handle = handle
    }

    deinit {
        tswift_context_free(handle)
    }
}

/// A failure declaring a host-service capability (e.g. an unrecognised
/// namespace).
public struct TSwiftHostServiceError: Error, Sendable {
    public let message: String
    public init(_ message: String) { self.message = message }
}

extension TSwiftContext {
    /// Declare that this host backs the host-service identified by
    /// `namespace` (`"tswift.defaults"`, `"tswift.fs"`, `"tswift.db"`),
    /// enabling the framework APIs layered on it (e.g. `UserDefaults`,
    /// `FileManager`) for scripts run through this context.
    ///
    /// This declares *capability only* — it does not, by itself, make any
    /// calls succeed. The host must also register the concrete
    /// `tswift.<namespace>.<op>` functions the framework calls via
    /// `registerHostFunction` (see `TSwiftFoundationHostServices.swift` for
    /// ready-made Foundation-backed implementations of `tswift.defaults`/
    /// `tswift.fs`). A service left undeclared here degrades every API layered
    /// on it to a capability diagnostic, matching a page/host that never opts
    /// in.
    public func declareHostService(_ namespace: String) throws {
        let resultJSON = namespace.withCString { cNamespace -> String in
            guard let ptr = tswift_declare_host_service(handle, cNamespace) else { return "" }
            defer { tswift_string_free(ptr) }
            return String(cString: ptr)
        }
        struct Envelope: Decodable { let ok: Bool; let namespace: String?; let error: String? }
        let envelope = try? JSONDecoder().decode(Envelope.self, from: Data(resultJSON.utf8))
        guard let envelope, envelope.ok else {
            let detail = resultJSON.isEmpty
                ? "tswift_declare_host_service returned null"
                : (envelope?.error ?? resultJSON)
            throw TSwiftHostServiceError(detail)
        }
    }
}
