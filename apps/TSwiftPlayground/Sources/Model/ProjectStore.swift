import Foundation

/// A failure performing a project/file operation, surfaced to the UI.
enum ProjectStoreError: LocalizedError, Equatable {
    case invalidName(String)
    case alreadyExists(String)
    case notFound(String)
    case underlying(String)

    var errorDescription: String? {
        switch self {
        case .invalidName(let n): return "Invalid name: \(n)"
        case .alreadyExists(let n): return "\u{201C}\(n)\u{201D} already exists"
        case .notFound(let n): return "\u{201C}\(n)\u{201D} was not found"
        case .underlying(let m): return m
        }
    }
}

/// File-backed store for multi-file projects: one folder per project under
/// `root`, each holding `.swift` files. Pure, synchronous, and injectable
/// (`root` defaults to `Documents/TSwiftProjects` but tests pass a temp dir),
/// so all CRUD/persistence is unit-testable without a UI or the app sandbox.
///
/// Not an `ObservableObject` on purpose — it owns no published state, only the
/// disk. The view layer (`ProjectsModel`) wraps it for SwiftUI.
///
/// Security invariant: every API that turns a caller-supplied name into a
/// filesystem `URL` — mutating *and* read-only — routes through
/// `resolvedURL(_:in:isDirectory:)`. That single choke point rejects empty/
/// dot/separator-laden names *and* re-checks the resolved, standardized path
/// is still lexically contained in the expected parent directory, so a name
/// like `"../../etc"` (or a bare `".."`) can never resolve outside `root`,
/// even for callers (delete/load) that don't otherwise validate syntax.
final class ProjectStore {
    let root: URL
    private let fileManager: FileManager

    /// The default on-device location: `Documents/TSwiftProjects`.
    static var defaultRoot: URL {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        return docs.appendingPathComponent("TSwiftProjects", isDirectory: true)
    }

    init(root: URL = ProjectStore.defaultRoot, fileManager: FileManager = .default) {
        self.root = root
        self.fileManager = fileManager
        try? fileManager.createDirectory(at: root, withIntermediateDirectories: true)
    }

    // MARK: - Projects

    /// Every project name (a directory under `root`), case-insensitively sorted.
    func projectNames() -> [String] {
        let entries = (try? fileManager.contentsOfDirectory(
            at: root, includingPropertiesForKeys: [.isDirectoryKey], options: [.skipsHiddenFiles]
        )) ?? []
        return entries
            .filter { (try? $0.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true }
            .map { $0.lastPathComponent }
            .sorted { $0.localizedStandardCompare($1) == .orderedAscending }
    }

    /// Create an empty project seeded with a default `main.swift`. Returns the
    /// loaded project.
    @discardableResult
    func createProject(_ name: String, seed: [ProjectFile]? = nil) throws -> Project {
        let clean = try Self.validProjectName(name)
        let dir = try projectDir(clean)
        if fileManager.fileExists(atPath: dir.path) {
            throw ProjectStoreError.alreadyExists(clean)
        }
        try create(dir)
        let files = seed ?? [ProjectFile(name: Project.entryFileName, contents: Self.starterSource)]
        for file in files {
            try write(file, inProjectDir: dir)
        }
        return try loadProject(clean)
    }

    /// Delete a project and all its files.
    func deleteProject(_ name: String) throws {
        let dir = try projectDir(name)
        guard fileManager.fileExists(atPath: dir.path) else {
            throw ProjectStoreError.notFound(name)
        }
        try remove(dir)
    }

    /// Rename a project's folder, preserving its files.
    func renameProject(_ oldName: String, to newName: String) throws {
        let from = try projectDir(oldName)
        let clean = try Self.validProjectName(newName)
        let to = try projectDir(clean)
        guard fileManager.fileExists(atPath: from.path) else {
            throw ProjectStoreError.notFound(oldName)
        }
        if clean != oldName, fileManager.fileExists(atPath: to.path) {
            throw ProjectStoreError.alreadyExists(clean)
        }
        do { try fileManager.moveItem(at: from, to: to) }
        catch { throw ProjectStoreError.underlying(error.localizedDescription) }
    }

    /// Load a project's files from disk.
    func loadProject(_ name: String) throws -> Project {
        let dir = try projectDir(name)
        guard fileManager.fileExists(atPath: dir.path) else {
            throw ProjectStoreError.notFound(name)
        }
        let entries = (try? fileManager.contentsOfDirectory(
            at: dir, includingPropertiesForKeys: nil, options: [.skipsHiddenFiles]
        )) ?? []
        let files = entries
            .filter { $0.pathExtension == "swift" }
            .map { url -> ProjectFile in
                let contents = (try? String(contentsOf: url, encoding: .utf8)) ?? ""
                return ProjectFile(name: url.lastPathComponent, contents: contents)
            }
            .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        return Project(name: name, files: files)
    }

    // MARK: - Files

    /// Create a new empty `.swift` file in a project.
    @discardableResult
    func createFile(_ name: String, in project: String, contents: String = "") throws -> ProjectFile {
        let clean = try Self.validFileName(name)
        let dir = try projectDir(project)
        guard fileManager.fileExists(atPath: dir.path) else {
            throw ProjectStoreError.notFound(project)
        }
        let url = try fileURL(clean, inProjectDir: dir)
        if fileManager.fileExists(atPath: url.path) {
            throw ProjectStoreError.alreadyExists(clean)
        }
        let file = ProjectFile(name: clean, contents: contents)
        try write(file, inProjectDir: dir)
        return file
    }

    /// Overwrite a file's contents (autosave). Creates it if missing.
    func saveFile(_ file: ProjectFile, in project: String) throws {
        let dir = try projectDir(project)
        try write(file, inProjectDir: dir)
    }

    /// Rename a file within a project.
    func renameFile(_ oldName: String, to newName: String, in project: String) throws {
        let dir = try projectDir(project)
        let from = try fileURL(oldName, inProjectDir: dir)
        let clean = try Self.validFileName(newName)
        let to = try fileURL(clean, inProjectDir: dir)
        guard fileManager.fileExists(atPath: from.path) else {
            throw ProjectStoreError.notFound(oldName)
        }
        if clean != oldName, fileManager.fileExists(atPath: to.path) {
            throw ProjectStoreError.alreadyExists(clean)
        }
        do { try fileManager.moveItem(at: from, to: to) }
        catch { throw ProjectStoreError.underlying(error.localizedDescription) }
    }

    /// Delete a file from a project.
    func deleteFile(_ name: String, in project: String) throws {
        let dir = try projectDir(project)
        let url = try fileURL(name, inProjectDir: dir)
        guard fileManager.fileExists(atPath: url.path) else {
            throw ProjectStoreError.notFound(name)
        }
        try remove(url)
    }

    // MARK: - Validation

    /// A project name with no path separators, `.`/`..`, or leading dots,
    /// trimmed. This is the *syntax* check surfaced to callers for early
    /// feedback (e.g. disabling the "Create" button); path containment is
    /// re-verified independently by `resolvedURL` on every disk access.
    static func validProjectName(_ name: String) throws -> String {
        try sanitizedComponent(name)
    }

    /// A `.swift` filename with no path separators. A bare name gains `.swift`.
    static func validFileName(_ name: String) throws -> String {
        var trimmed = try sanitizedComponent(name)
        if !trimmed.hasSuffix(".swift") { trimmed += ".swift" }
        // Reject a name that is only the extension.
        guard trimmed != ".swift" else { throw ProjectStoreError.invalidName(name) }
        return trimmed
    }

    /// Trim and reject anything that isn't a single, plain path component:
    /// empty, `.`/`..`, a leading dot (hidden files), or any path/volume
    /// separator. Shared by `validProjectName`, `validFileName`, and
    /// `resolvedURL` so all three apply exactly the same rule.
    private static func sanitizedComponent(_ name: String) throws -> String {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty,
              trimmed != ".",
              trimmed != "..",
              !trimmed.hasPrefix("."),
              !trimmed.contains("/"),
              !trimmed.contains("\\"),
              !trimmed.contains(":")
        else { throw ProjectStoreError.invalidName(name) }
        return trimmed
    }

    /// The single choke point mapping a caller-supplied name to a URL inside
    /// `dir`: sanitizes the name, resolves it, then verifies the
    /// *standardized* result is still lexically inside the *standardized*
    /// `dir` before returning it. Used by every project- and file-URL builder
    /// below — no call site is allowed to `appendingPathComponent` a raw name
    /// itself.
    private static func resolvedURL(_ name: String, in dir: URL, isDirectory: Bool) throws -> URL {
        let trimmed = try sanitizedComponent(name)
        let candidate = dir.appendingPathComponent(trimmed, isDirectory: isDirectory).standardizedFileURL
        let base = dir.standardizedFileURL
        let basePrefix = base.path.hasSuffix("/") ? base.path : base.path + "/"
        guard candidate.path.hasPrefix(basePrefix) else {
            throw ProjectStoreError.invalidName(name)
        }
        return candidate
    }

    static let starterSource = """
    // A new file. Define a SwiftUI View for a live preview, or write
    // top-level statements in main.swift for a console program.
    print("Hello, tswift!")
    """

    // MARK: - Private helpers

    /// A project's directory, validated + containment-checked against `root`.
    private func projectDir(_ name: String) throws -> URL {
        try Self.resolvedURL(name, in: root, isDirectory: true)
    }

    /// A file's URL within an already-resolved project directory, validated +
    /// containment-checked against that directory.
    private func fileURL(_ name: String, inProjectDir dir: URL) throws -> URL {
        try Self.resolvedURL(name, in: dir, isDirectory: false)
    }

    private func create(_ dir: URL) throws {
        do { try fileManager.createDirectory(at: dir, withIntermediateDirectories: true) }
        catch { throw ProjectStoreError.underlying(error.localizedDescription) }
    }

    private func remove(_ url: URL) throws {
        do { try fileManager.removeItem(at: url) }
        catch { throw ProjectStoreError.underlying(error.localizedDescription) }
    }

    private func write(_ file: ProjectFile, inProjectDir dir: URL) throws {
        let url = try fileURL(file.name, inProjectDir: dir)
        do { try file.contents.write(to: url, atomically: true, encoding: .utf8) }
        catch { throw ProjectStoreError.underlying(error.localizedDescription) }
    }
}
