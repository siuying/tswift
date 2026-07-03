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
