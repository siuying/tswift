import XCTest
@testable import TSwiftPlayground

/// Ordering tests for `StudioModel`'s debounced autosave racing structural
/// changes (rename/delete): a pending autosave scheduled before a rename or
/// delete must never resurrect the old file or recreate a deleted one once it
/// fires later.
@MainActor
final class StudioModelTests: XCTestCase {
    private var root: URL!
    private var store: ProjectStore!

    override func setUpWithError() throws {
        root = URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
            .appendingPathComponent("tswift-studiomodel-\(UUID().uuidString)", isDirectory: true)
        store = ProjectStore(root: root)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: root)
    }

    /// autosave scheduled -> delete -> autosave fires -> file must NOT reappear.
    func testAutosaveDoesNotResurrectDeletedFile() async throws {
        _ = try store.createProject(
            "Alpha",
            seed: [
                ProjectFile(name: "main.swift", contents: "print(1)"),
                ProjectFile(name: "Extra.swift", contents: "let a = 1"),
            ]
        )
        let project = try store.loadProject("Alpha")
        let model = StudioModel(store: store, project: project)

        // Edit Extra.swift to schedule a debounced autosave for it.
        model.select("Extra.swift")
        model.selectedText.wrappedValue = "let a = 2"

        // Delete it before the debounce window elapses. This must cancel/
        // invalidate the pending autosave.
        model.deleteFile("Extra.swift")

        // Let the (now-invalidated) autosave's debounce window fully elapse.
        try await Task.sleep(for: .milliseconds(500))

        let extraURL = root.appendingPathComponent("Alpha").appendingPathComponent("Extra.swift")
        XCTAssertFalse(
            FileManager.default.fileExists(atPath: extraURL.path),
            "a stale autosave recreated a file that was deleted"
        )
        let reloaded = try store.loadProject("Alpha")
        XCTAssertFalse(reloaded.files.contains { $0.name == "Extra.swift" })
    }

    /// autosave scheduled -> rename -> autosave fires -> old name must NOT
    /// reappear on disk (the write must land on the new name, or not at all,
    /// never resurrect the pre-rename file).
    func testAutosaveDoesNotResurrectOldNameAfterRename() async throws {
        _ = try store.createProject(
            "Alpha",
            seed: [
                ProjectFile(name: "main.swift", contents: "print(1)"),
                ProjectFile(name: "Old.swift", contents: "let a = 1"),
            ]
        )
        let project = try store.loadProject("Alpha")
        let model = StudioModel(store: store, project: project)

        model.select("Old.swift")
        model.selectedText.wrappedValue = "let a = 2"

        model.renameFile("Old.swift", to: "New.swift")

        try await Task.sleep(for: .milliseconds(500))

        let projectDir = root.appendingPathComponent("Alpha")
        XCTAssertFalse(
            FileManager.default.fileExists(atPath: projectDir.appendingPathComponent("Old.swift").path),
            "a stale autosave resurrected the pre-rename file"
        )
        XCTAssertTrue(FileManager.default.fileExists(atPath: projectDir.appendingPathComponent("New.swift").path))
    }

    /// Baseline: an autosave that is *not* raced by any structural change
    /// still persists normally (guards against over-invalidating).
    func testAutosavePersistsWithoutRace() async throws {
        _ = try store.createProject("Alpha")
        let project = try store.loadProject("Alpha")
        let model = StudioModel(store: store, project: project)

        model.select("main.swift")
        model.selectedText.wrappedValue = "print(99)"

        try await Task.sleep(for: .milliseconds(500))

        let reloaded = try store.loadProject("Alpha")
        XCTAssertEqual(reloaded.files.first { $0.name == "main.swift" }?.contents, "print(99)")
    }
}
