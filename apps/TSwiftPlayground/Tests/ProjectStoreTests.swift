import XCTest
@testable import TSwiftPlayground

/// Unit tests for the file-backed `ProjectStore`: project + file CRUD and
/// persistence, all against a fresh temp-directory root (never the app
/// sandbox), so they run hermetically and in parallel.
final class ProjectStoreTests: XCTestCase {
    private var root: URL!
    private var store: ProjectStore!

    override func setUpWithError() throws {
        root = URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
            .appendingPathComponent("tswift-projectstore-\(UUID().uuidString)", isDirectory: true)
        store = ProjectStore(root: root)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: root)
        try? FileManager.default.removeItem(at: escapeTarget)
    }

    // MARK: Projects

    func testCreateSeedsMainSwift() throws {
        let project = try store.createProject("Alpha")
        XCTAssertEqual(project.name, "Alpha")
        XCTAssertEqual(project.files.map(\.name), ["main.swift"])
        XCTAssertEqual(store.projectNames(), ["Alpha"])
    }

    func testCreateWithSeedFiles() throws {
        let seed = [
            ProjectFile(name: "main.swift", contents: "print(1)"),
            ProjectFile(name: "Helper.swift", contents: "func h() {}"),
        ]
        let project = try store.createProject("Beta", seed: seed)
        XCTAssertEqual(Set(project.files.map(\.name)), ["main.swift", "Helper.swift"])
    }

    func testDuplicateProjectRejected() throws {
        _ = try store.createProject("Alpha")
        XCTAssertThrowsError(try store.createProject("Alpha")) { error in
            XCTAssertEqual(error as? ProjectStoreError, .alreadyExists("Alpha"))
        }
    }

    func testProjectNamesSortedAndFiltersFiles() throws {
        _ = try store.createProject("Zeta")
        _ = try store.createProject("Alpha")
        // A stray loose file at root must not show up as a project.
        try "x".write(to: root.appendingPathComponent("loose.txt"), atomically: true, encoding: .utf8)
        XCTAssertEqual(store.projectNames(), ["Alpha", "Zeta"])
    }

    func testDeleteProject() throws {
        _ = try store.createProject("Alpha")
        try store.deleteProject("Alpha")
        XCTAssertEqual(store.projectNames(), [])
        XCTAssertThrowsError(try store.deleteProject("Alpha"))
    }

    func testRenameProjectPreservesFiles() throws {
        _ = try store.createProject("Alpha", seed: [ProjectFile(name: "main.swift", contents: "let a = 1")])
        try store.renameProject("Alpha", to: "Gamma")
        XCTAssertEqual(store.projectNames(), ["Gamma"])
        let loaded = try store.loadProject("Gamma")
        XCTAssertEqual(loaded.files.first?.contents, "let a = 1")
    }

    func testInvalidProjectNameRejected() {
        XCTAssertThrowsError(try store.createProject("bad/name"))
        XCTAssertThrowsError(try store.createProject("  "))
        XCTAssertThrowsError(try store.createProject(".hidden"))
    }

    // MARK: Files

    func testCreateFileAddsSwiftExtension() throws {
        _ = try store.createProject("Alpha")
        let file = try store.createFile("Model", in: "Alpha")
        XCTAssertEqual(file.name, "Model.swift")
        let loaded = try store.loadProject("Alpha")
        XCTAssertEqual(Set(loaded.files.map(\.name)), ["main.swift", "Model.swift"])
    }

    func testDuplicateFileRejected() throws {
        _ = try store.createProject("Alpha")
        XCTAssertThrowsError(try store.createFile("main.swift", in: "Alpha")) { error in
            XCTAssertEqual(error as? ProjectStoreError, .alreadyExists("main.swift"))
        }
    }

    func testSaveFilePersistsAcrossReload() throws {
        _ = try store.createProject("Alpha")
        try store.saveFile(ProjectFile(name: "main.swift", contents: "print(42)"), in: "Alpha")
        // A fresh store over the same root sees the persisted contents.
        let reopened = ProjectStore(root: root)
        let loaded = try reopened.loadProject("Alpha")
        XCTAssertEqual(loaded.files.first { $0.name == "main.swift" }?.contents, "print(42)")
    }

    func testRenameFile() throws {
        _ = try store.createProject("Alpha")
        _ = try store.createFile("Old.swift", in: "Alpha")
        try store.renameFile("Old.swift", to: "New.swift", in: "Alpha")
        let loaded = try store.loadProject("Alpha")
        XCTAssertTrue(loaded.files.contains { $0.name == "New.swift" })
        XCTAssertFalse(loaded.files.contains { $0.name == "Old.swift" })
    }

    func testDeleteFile() throws {
        _ = try store.createProject("Alpha")
        _ = try store.createFile("Extra.swift", in: "Alpha")
        try store.deleteFile("Extra.swift", in: "Alpha")
        let loaded = try store.loadProject("Alpha")
        XCTAssertEqual(loaded.files.map(\.name), ["main.swift"])
    }

    func testInvalidFileNameRejected() throws {
        _ = try store.createProject("Alpha")
        XCTAssertThrowsError(try store.createFile("a/b.swift", in: "Alpha"))
        XCTAssertThrowsError(try store.createFile(".swift", in: "Alpha"))
    }

    // MARK: Path traversal

    /// Every API that maps a name to a URL must reject `..`/separator-laden
    /// names and must never touch anything outside `root`, even the ones
    /// (load/delete) that historically skipped syntax validation entirely.
    /// `escapeTarget` is a file just outside `root` that a successful
    /// traversal would have touched; every case below must leave it alone.

    private var escapeTarget: URL {
        root.deletingLastPathComponent().appendingPathComponent("escaped.marker")
    }

    private func plantEscapeTarget() throws {
        try "untouched".write(to: escapeTarget, atomically: true, encoding: .utf8)
    }

    private func assertEscapeTargetUntouched(file: StaticString = #filePath, line: UInt = #line) throws {
        let contents = try String(contentsOf: escapeTarget, encoding: .utf8)
        XCTAssertEqual(contents, "untouched", file: file, line: line)
    }

    func testCreateProjectRejectsTraversal() throws {
        try plantEscapeTarget()
        for bad in ["../escaped", "..", ".", "a/../../b", "/etc/passwd", "", "   ", ".hidden"] {
            XCTAssertThrowsError(try store.createProject(bad), "expected rejection for \"\(bad)\"")
        }
        try assertEscapeTargetUntouched()
    }

    func testDeleteProjectRejectsTraversal() throws {
        try plantEscapeTarget()
        for bad in ["../escaped", "..", "a/../../b"] {
            XCTAssertThrowsError(try store.deleteProject(bad), "expected rejection for \"\(bad)\"")
        }
        try assertEscapeTargetUntouched()
    }

    func testLoadProjectRejectsTraversal() throws {
        for bad in ["../escaped", "..", "a/../../b"] {
            XCTAssertThrowsError(try store.loadProject(bad), "expected rejection for \"\(bad)\"")
        }
    }

    func testRenameProjectRejectsTraversalOnEitherSide() throws {
        _ = try store.createProject("Alpha")
        try plantEscapeTarget()
        XCTAssertThrowsError(try store.renameProject("../escaped", to: "Alpha2"))
        XCTAssertThrowsError(try store.renameProject("Alpha", to: "../escaped"))
        XCTAssertThrowsError(try store.renameProject("Alpha", to: ".."))
        try assertEscapeTargetUntouched()
        // Alpha must be exactly where it was — no partial rename occurred.
        XCTAssertEqual(store.projectNames(), ["Alpha"])
    }

    func testCreateFileRejectsTraversal() throws {
        _ = try store.createProject("Alpha")
        try plantEscapeTarget()
        for bad in ["../escaped", "..", "../../escaped.swift", "a/../../b.swift"] {
            XCTAssertThrowsError(try store.createFile(bad, in: "Alpha"), "expected rejection for \"\(bad)\"")
        }
        // The project name itself is also a traversal surface.
        XCTAssertThrowsError(try store.createFile("x.swift", in: "../escaped"))
        try assertEscapeTargetUntouched()
    }

    func testSaveFileRejectsTraversal() throws {
        _ = try store.createProject("Alpha")
        try plantEscapeTarget()
        XCTAssertThrowsError(
            try store.saveFile(ProjectFile(name: "../../escaped.marker", contents: "pwned"), in: "Alpha")
        )
        try assertEscapeTargetUntouched()
    }

    func testRenameFileRejectsTraversalOnEitherSide() throws {
        _ = try store.createProject("Alpha")
        _ = try store.createFile("Old.swift", in: "Alpha")
        try plantEscapeTarget()
        XCTAssertThrowsError(try store.renameFile("../../escaped.marker", to: "New.swift", in: "Alpha"))
        XCTAssertThrowsError(try store.renameFile("Old.swift", to: "../../escaped.marker", in: "Alpha"))
        try assertEscapeTargetUntouched()
        // Old.swift must be exactly where it was — no partial rename occurred.
        let loaded = try store.loadProject("Alpha")
        XCTAssertTrue(loaded.files.contains { $0.name == "Old.swift" })
    }

    func testDeleteFileRejectsTraversal() throws {
        _ = try store.createProject("Alpha")
        try plantEscapeTarget()
        XCTAssertThrowsError(try store.deleteFile("../../escaped.marker", in: "Alpha"))
        XCTAssertThrowsError(try store.deleteFile("..", in: "Alpha"))
        try assertEscapeTargetUntouched()
    }

    // MARK: Project model helpers

    func testOrderedFilesPutsMainLast() {
        let project = Project(name: "P", files: [
            ProjectFile(name: "main.swift", contents: ""),
            ProjectFile(name: "Beta.swift", contents: ""),
            ProjectFile(name: "Alpha.swift", contents: ""),
        ])
        XCTAssertEqual(project.orderedFiles.map(\.name), ["Alpha.swift", "Beta.swift", "main.swift"])
    }

    func testInferredMode() {
        let ui = Project(name: "P", files: [
            ProjectFile(name: "main.swift", contents: "struct V: View { var body: some View { Text(\"x\") } }"),
        ])
        XCTAssertEqual(ui.inferredMode, .preview)
        let console = Project(name: "P", files: [
            ProjectFile(name: "main.swift", contents: "print(\"hi\")"),
        ])
        XCTAssertEqual(console.inferredMode, .console)
    }
}
