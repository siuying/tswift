import SnapshotTesting
import SwiftUI
import XCTest

@testable import UiirRenderer

/// Layer D iOS harness: render every UIIR fixture, replay its patch stream, and
/// snapshot at each step across a device × appearance matrix. See
/// `docs/plan/layer-d-ios-renderer.md`.
@MainActor
final class SnapshotTests: XCTestCase {
    /// Flip to `true` for the first run to record baselines, then set back.
    private let recording = false

    /// Device presets, paired with the web harness viewports (iPhone 13 @3x,
    /// iPad Pro 11" @2x). The fixture lays out in the full device area.
    private struct DeviceCase {
        let name: String
        let config: ViewImageConfig
    }
    private let devices: [DeviceCase] = [
        DeviceCase(name: "iphone", config: .iPhone13),
        DeviceCase(name: "ipad", config: .iPadPro11(.portrait)),
    ]

    /// Appearances, driven by the snapshot trait collection so semantic colors
    /// and the system background adapt.
    private struct SchemeCase {
        let name: String
        let style: UIUserInterfaceStyle
    }
    private let schemes: [SchemeCase] = [
        SchemeCase(name: "light", style: .light),
        SchemeCase(name: "dark", style: .dark),
    ]

    /// Snapshot `view` once per device × appearance, naming each
    /// `<step>-<device>-<scheme>` to align with the web baselines.
    private func snapshot(_ view: some View, named step: String) {
        for device in devices {
            for scheme in schemes {
                assertSnapshot(
                    of: UIHostingController(rootView: view),
                    as: .image(
                        on: device.config,
                        precision: 0.98,
                        traits: UITraitCollection(userInterfaceStyle: scheme.style)
                    ),
                    named: "\(step)-\(device.name)-\(scheme.name)",
                    record: recording
                )
            }
        }
    }

    /// Render a fixture's initial tree and each post-patch step.
    private func runFixture(_ name: String) throws {
        let root = try FixtureLoader.loadUiir(name)
        let model = RenderModel(root: root)
        let host = RenderHostView(model: model)

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
    // C1/C2/C3 breadth fixtures (previously only covered by the Rust goldens).
    func testTextStyling() throws { try runFixture("text-styling") }
    func testLayout() throws { try runFixture("layout") }
    func testLayoutTyped() throws { try runFixture("layout-typed") }
    func testStackAlignment() throws { try runFixture("stack-alignment") }
    func testLazyGrids() throws { try runFixture("lazy-grids") }
    func testContainers() throws { try runFixture("containers") }
    func testScrollHorizontal() throws { try runFixture("scroll-horizontal") }
    func testDecoration() throws { try runFixture("decoration") }
    func testContent() throws { try runFixture("content") }
    func testGrids() throws { try runFixture("grids") }
    func testStyling() throws { try runFixture("styling") }
}
