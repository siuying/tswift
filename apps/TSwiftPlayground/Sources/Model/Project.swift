import Foundation
import TSwiftCore

/// One Swift source file inside a project. `name` is the bare filename
/// (`main.swift`), unique within a project; `contents` is its text.
struct ProjectFile: Identifiable, Equatable, Hashable {
    var name: String
    var contents: String
    var id: String { name }
}

/// An in-memory snapshot of a project: its name plus its ordered source files.
/// Persisted on disk as one folder per project under the store root (see
/// `ProjectStore`).
struct Project: Identifiable, Equatable {
    var name: String
    var files: [ProjectFile]
    var id: String { name }

    /// The file that may hold top-level executable statements. The runtime only
    /// allows `main.swift` (or a lone single file) to do so, so runs/previews
    /// always order it last.
    static let entryFileName = "main.swift"

    /// Files ordered for compilation: every non-entry file alphabetically,
    /// then `main.swift` last (top-level statements must come after the
    /// declarations they use).
    var orderedFiles: [ProjectFile] {
        let entry = files.filter { $0.name == Project.entryFileName }
        let rest = files.filter { $0.name != Project.entryFileName }
            .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        return rest + entry
    }

    /// The FFI module payload for this project, in compilation order.
    func module() -> TSwiftModule {
        TSwiftModule(files: orderedFiles.map {
            TSwiftSourceFile(path: $0.name, contents: $0.contents)
        })
    }

    /// Heuristic run mode: a project defining a SwiftUI `View` renders live;
    /// otherwise it runs as a console program. Used as the default when a
    /// project is opened; the user can override it in the Studio toolbar.
    var inferredMode: RunMode {
        let joined = files.map(\.contents).joined(separator: "\n")
        let looksLikeView = joined.contains(": View") && joined.contains("var body")
        return looksLikeView ? .preview : .console
    }
}

/// How a project's `Run` action executes it.
enum RunMode: String, CaseIterable, Identifiable {
    /// Compile a SwiftUI `View` and render it live (via `PreviewSession`).
    case preview
    /// Run top-level/`main.swift` statements and capture stdout (via
    /// `TSwiftCore.run(module:)`).
    case console

    var id: String { rawValue }
    var label: String { self == .preview ? "Preview" : "Console" }
    var systemImage: String { self == .preview ? "eye" : "terminal" }
}
