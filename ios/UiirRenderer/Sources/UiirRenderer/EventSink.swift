import SwiftUI

/// A user interaction emitted by a rendered interactive control, addressed to
/// the node that produced it. Mirrors the `(id, event, value)` triple that
/// `tswift swiftui dispatch` consumes: `event` is `"tap"` or `"set"`, and
/// `value` is a raw JSON scalar (`"true"`, `"42"`, `"\"hi\""`) or `""` for a
/// payload-less tap.
public struct UiirEvent: Equatable, Sendable {
    public let id: String
    public let event: String
    public let value: String

    public init(id: String, event: String, value: String) {
        self.id = id
        self.event = event
        self.value = value
    }

    /// A payload-less tap on `id`.
    public static func tap(_ id: String) -> UiirEvent {
        UiirEvent(id: id, event: "tap", value: "")
    }

    /// A control `set` writing the raw JSON scalar `value` to `id`.
    public static func set(_ id: String, _ value: String) -> UiirEvent {
        UiirEvent(id: id, event: "set", value: value)
    }

    /// A payload-less gesture/lifecycle/submit event on `id` (ADR-0013 §3):
    /// `"longPress"`, `"appear"`, `"disappear"`, or `"submit"`. The runtime
    /// routes these into the node's handler map by name.
    public static func named(_ id: String, _ event: String) -> UiirEvent {
        UiirEvent(id: id, event: event, value: "")
    }

    /// A `TabView` tab selection (ADR-0013 §2): `value` is the chosen tab's
    /// tag-or-index as a raw JSON scalar (`"\"home\""`, `"1"`). The runtime
    /// writes it through the `selection:` binding, or keeps per-node state.
    public static func select(_ id: String, _ value: String) -> UiirEvent {
        UiirEvent(id: id, event: "select", value: value)
    }

    /// A `NavigationStack` back navigation (ADR-0013 §1): the system pop
    /// gesture / back button on the stack `id`. The runtime pops the topmost
    /// pushed screen.
    public static func back(_ id: String) -> UiirEvent {
        UiirEvent(id: id, event: "back", value: "")
    }
}

/// Receives interaction events from rendered controls. The default sink
/// ([`NoopEventSink`]) discards them, so static snapshots stay inert; a live
/// preview injects a sink that forwards to a render session.
@MainActor
public protocol UiirEventSink {
    func send(_ event: UiirEvent)
}

/// The default sink: a no-op, keeping `ViewFactory` output identical to the
/// pre-seam static renderer.
public struct NoopEventSink: UiirEventSink {
    public init() {}
    public func send(_ event: UiirEvent) {}
}

/// A sink backed by a closure — convenient for wiring a live render session.
public struct ClosureEventSink: UiirEventSink {
    private let handler: @MainActor (UiirEvent) -> Void
    public init(_ handler: @escaping @MainActor (UiirEvent) -> Void) {
        self.handler = handler
    }
    public func send(_ event: UiirEvent) {
        handler(event)
    }
}

private struct UiirEventSinkKey: @preconcurrency EnvironmentKey {
    @MainActor static let defaultValue: any UiirEventSink = NoopEventSink()
}

extension EnvironmentValues {
    /// The sink that rendered controls forward interactions to. Defaults to a
    /// no-op; inject a live one with `.uiirEventSink(_:)`.
    public var uiirEventSink: any UiirEventSink {
        get { self[UiirEventSinkKey.self] }
        set { self[UiirEventSinkKey.self] = newValue }
    }
}

extension View {
    /// Install the `sink` that this subtree's controls forward interactions to.
    public func uiirEventSink(_ sink: any UiirEventSink) -> some View {
        environment(\.uiirEventSink, sink)
    }
}
