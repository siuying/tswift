import XCTest

@testable import UiirRenderer

@MainActor
final class EventSinkTests: XCTestCase {
    /// Records every event it receives, for assertions.
    final class Recorder: UiirEventSink {
        private(set) var events: [UiirEvent] = []
        func send(_ event: UiirEvent) { events.append(event) }
    }

    private func node(kind: String, id: String, args: [String: UiirValue] = [:]) -> UiirNode {
        UiirNode(id: id, kind: kind, args: args, modifiers: [], children: [])
    }

    func testButtonTapForwardsTapEvent() {
        let recorder = Recorder()
        let button = node(kind: "Button", id: "0.1", args: ["title": .string("Increment")])
        ViewFactory.tapAction(button, recorder)()
        XCTAssertEqual(recorder.events, [UiirEvent(id: "0.1", event: "tap", value: "")])
    }

    func testControlBindingSetForwardsSetEvent() {
        let recorder = Recorder()
        let toggle = node(kind: "Toggle", id: "0.2", args: ["isOn": .bool(false)])
        let binding = ViewFactory.controlBinding(
            toggle, recorder, value: false, encode: { $0 ? "true" : "false" }
        )
        binding.wrappedValue = true  // simulate the user flipping the toggle
        XCTAssertEqual(recorder.events, [UiirEvent(id: "0.2", event: "set", value: "true")])
    }

    func testNoopSinkRecordsNothing() {
        let button = node(kind: "Button", id: "0.0")
        // The default sink must silently discard — keeps snapshots inert.
        ViewFactory.tapAction(button, NoopEventSink())()
    }
}
