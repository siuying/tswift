import UiirRenderer
import XCTest

@testable import TSwiftUI

@MainActor
final class PreviewSessionTests: XCTestCase {
    private let counter = """
    struct CounterView: View {
        @State private var count = 0
        var body: some View {
            VStack {
                Text("\\(count)")
                Button("Increment") { count += 1 }
            }
        }
    }
    """

    /// Whether any node in the tree is a `Text` whose verbatim arg equals `text`.
    private func containsText(_ node: UiirNode, _ text: String) -> Bool {
        if node.kind == "Text", node.args["verbatim"]?.stringValue == text {
            return true
        }
        return node.children.contains { containsText($0, text) }
    }

    func testCompileMountsInitialTree() {
        let session = PreviewSession()
        session.compile(counter)
        XCTAssertNil(session.lastError, session.lastError ?? "")
        XCTAssertEqual(session.root, "CounterView")
        XCTAssertEqual(session.model.root.kind, "VStack")
        XCTAssertTrue(containsText(session.model.root, "0"))
    }

    func testDispatchTapUpdatesLabel() {
        let session = PreviewSession()
        session.compile(counter)
        // The button is the second child of the root VStack: id "0.1".
        session.dispatch(id: "0.1", event: "tap", value: "")
        XCTAssertNil(session.lastError, session.lastError ?? "")
        XCTAssertTrue(containsText(session.model.root, "1"))
        XCTAssertFalse(containsText(session.model.root, "0"))
    }

    func testDispatchRejectsNonScalarValue() {
        let session = PreviewSession()
        session.compile(counter)
        session.dispatch(id: "0.1", event: "set", value: "{\"x\":1}")
        XCTAssertNotNil(session.lastError)
        XCTAssertTrue(session.lastError?.contains("JSON scalar") ?? false)
        // The tree is unchanged (no patch applied).
        XCTAssertTrue(containsText(session.model.root, "0"))
    }

    func testCompileErrorSurfaced() {
        let session = PreviewSession()
        session.compile("let x = 1")
        XCTAssertNotNil(session.lastError)
    }

    func testEventJSONOmitsEmptyValue() {
        XCTAssertEqual(
            PreviewSession.eventJSON(id: "0.1", event: "tap", value: ""),
            #"{"id":"0.1","event":"tap"}"#
        )
        XCTAssertEqual(
            PreviewSession.eventJSON(id: "0.2", event: "set", value: "true"),
            #"{"id":"0.2","event":"set","value":true}"#
        )
    }
}
