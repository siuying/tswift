import SnapshotTesting
import SwiftUI
import XCTest

@testable import UiirRenderer

/// Layer D iOS harness: render every UIIR fixture, replay its patch stream, and
/// snapshot at each step. See `docs/plan/layer-d-ios-renderer.md`.
@MainActor
final class SnapshotTests: XCTestCase {
    /// Flip to `true` for the first run to record baselines, then set back.
    private let recording = false

    private let hostSize = CGSize(width: 320, height: 480)

    private func snapshot(_ view: some View, named name: String) {
        let host = view.frame(width: hostSize.width, height: hostSize.height)
        assertSnapshot(
            of: UIHostingController(rootView: host),
            as: .image(on: .iPhone13, precision: 0.98),
            named: name,
            record: recording
        )
    }

    /// Render a fixture's initial tree and each post-patch step.
    private func runFixture(_ name: String) throws {
        let root = try FixtureLoader.loadUiir(name)
        let model = RenderModel(root: root)
        let host = RenderHostView(model: model, size: hostSize)

        snapshot(host, named: "\(name)-0-initial")

        let steps = try FixtureLoader.loadPatches(name)
        for (i, batch) in steps.enumerated() {
            model.apply(batch)
            snapshot(host, named: "\(name)-\(i + 1)")
        }
    }

    func testCounter() throws { try runFixture("counter") }
    func testGreeting() throws { try runFixture("greeting") }
    func testStack() throws { try runFixture("stack") }
    func testProfile() throws { try runFixture("profile") }
    func testList() throws { try runFixture("list") }
    func testObservable() throws { try runFixture("observable") }
    func testControls() throws { try runFixture("controls") }
    func testForm() throws { try runFixture("form") }
    func testPicker() throws { try runFixture("picker") }
    func testEnvironment() throws { try runFixture("environment") }
    func testSections() throws { try runFixture("sections") }
}
