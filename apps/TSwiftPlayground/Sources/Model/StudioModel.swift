import Foundation
import SwiftUI
import TSwiftCore
import TSwiftUI

/// Editing + run state for one open project. Owns the on-disk autosave, the
/// live `PreviewSession` (SwiftUI mode), the console runner (console mode), and
/// the symbol outline. Backed by a `ProjectStore` folder.
@MainActor
final class StudioModel: ObservableObject {
    let store: ProjectStore
    let projectName: String

    @Published var files: [ProjectFile]
    /// Name of the file currently shown in the editor.
    @Published var selectedFileName: String
    @Published var mode: RunMode
    @Published private(set) var consoleOutput: String = ""
    @Published private(set) var consoleIsError = false
    @Published private(set) var symbols: [TSwiftSymbol] = []
    /// A pending editor jump request (from tapping a symbol), consumed by the
    /// editor. Carries a token so repeated jumps to the same line re-fire.
    @Published var jumpTarget: JumpTarget?

    /// The live SwiftUI preview session (used in `.preview` mode).
    let session = PreviewSession()

    private var autosaveTask: Task<Void, Never>?
    private var recomputeTask: Task<Void, Never>?
    private static let debounce: Duration = .milliseconds(300)
    /// Bumped by every structural file change (create/rename/delete). A
    /// pending autosave captures the generation it was scheduled under and
    /// bails if it no longer matches when it wakes, so a rename/delete that
    /// lands mid-debounce can never have a stale autosave resurrect the old
    /// name or recreate a just-deleted file.
    private var autosaveGeneration = 0

    init(store: ProjectStore, project: Project) {
        self.store = store
        self.projectName = project.name
        self.files = project.files.isEmpty
            ? [ProjectFile(name: Project.entryFileName, contents: "")]
            : project.files
        self.selectedFileName = project.files.first?.name ?? Project.entryFileName
        self.mode = project.inferredMode
        // Wire the host services (Foundation + SwiftData over real SQLite) so
        // the bundled samples work; failures are non-fatal (capability stays
        // off). Installed on the shared session context, so both the live
        // preview and console runs see them.
        try? session.context.installFoundationHostServices()
        try? session.context.installDatabaseHostServices()
        recomputeNow()
    }

    /// The current project snapshot (files as edited in memory).
    var project: Project { Project(name: projectName, files: files) }

    /// The file currently selected for editing, if it still exists.
    var selectedIndex: Int? {
        files.firstIndex { $0.name == selectedFileName }
    }

    // MARK: Editing

    /// A binding to the selected file's text, wired to autosave + recompute.
    var selectedText: Binding<String> {
        Binding(
            get: { [weak self] in
                guard let self, let i = self.selectedIndex else { return "" }
                return self.files[i].contents
            },
            set: { [weak self] newValue in
                guard let self, let i = self.selectedIndex else { return }
                guard self.files[i].contents != newValue else { return }
                self.files[i].contents = newValue
                self.scheduleAutosave()
                self.scheduleRecompute()
            }
        )
    }

    func select(_ name: String) {
        selectedFileName = name
        recomputeNow()
    }

    // MARK: File CRUD (mirrors the store, keeps in-memory `files` in sync)

    func createFile(_ name: String) {
        do {
            let file = try store.createFile(name, in: projectName, contents: "")
            files.append(file)
            files.sort { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
            selectedFileName = file.name
            recomputeNow()
        } catch { report(error) }
    }

    func renameFile(_ old: String, to new: String) {
        invalidatePendingAutosave()
        do {
            let clean = try ProjectStore.validFileName(new)
            try store.renameFile(old, to: clean, in: projectName)
            if let i = files.firstIndex(where: { $0.name == old }) {
                files[i].name = clean
            }
            if selectedFileName == old { selectedFileName = clean }
            recomputeNow()
        } catch { report(error) }
    }

    func deleteFile(_ name: String) {
        guard files.count > 1 else {
            report(ProjectStoreError.underlying("A project must keep at least one file"))
            return
        }
        invalidatePendingAutosave()
        do {
            try store.deleteFile(name, in: projectName)
            files.removeAll { $0.name == name }
            if selectedFileName == name {
                selectedFileName = files.first?.name ?? Project.entryFileName
            }
            recomputeNow()
        } catch { report(error) }
    }

    // MARK: Run

    /// Run the project in the current `mode`: render live (preview) or capture
    /// stdout (console).
    func run() {
        switch mode {
        case .preview:
            session.compile(module: project.module())
        case .console:
            runConsole()
        }
    }

    private func runConsole() {
        let result = TSwiftCore.run(module: project.module(), in: session.context)
        consoleIsError = !result.ok
        if result.ok {
            consoleOutput = result.stdout.isEmpty ? "(no output)" : result.stdout
        } else {
            let diag = result.diagnostics.isEmpty ? result.raw : result.diagnostics
            consoleOutput = diag
        }
    }

    // MARK: Symbols & diagnostics

    private func scheduleRecompute() {
        recomputeTask?.cancel()
        recomputeTask = Task { [weak self] in
            try? await Task.sleep(for: Self.debounce)
            guard !Task.isCancelled else { return }
            self?.recomputeNow()
        }
    }

    /// Refresh diagnostics + symbol outline, and (in preview mode) recompile the
    /// live render. Console mode only re-runs on an explicit Run tap.
    func recomputeNow() {
        let module = project.module()
        session.diagnose(module: module)
        symbols = TSwiftCore.listSymbols(module: module).symbols
        if mode == .preview {
            session.compile(module: module)
        }
    }

    /// Jump the editor to a symbol: switch to its file and request a caret move.
    func jump(to symbol: TSwiftSymbol) {
        if selectedFileName != symbol.file, files.contains(where: { $0.name == symbol.file }) {
            selectedFileName = symbol.file
        }
        jumpTarget = JumpTarget(line: symbol.line)
    }

    // MARK: Persistence

    private func scheduleAutosave() {
        invalidatePendingAutosave()
        let generation = autosaveGeneration
        let snapshot = files
        let name = projectName
        autosaveTask = Task { [weak self] in
            try? await Task.sleep(for: Self.debounce)
            guard !Task.isCancelled else { return }
            guard let self, self.autosaveGeneration == generation else { return }
            for file in snapshot {
                // Re-validate against the *live* model, not just the
                // generation counter: only write files that still exist under
                // their scheduled name, so a rename/delete racing this write
                // can never recreate a deleted file or resurrect a stale name.
                guard self.files.contains(where: { $0.name == file.name }) else { continue }
                try? self.store.saveFile(file, in: name)
            }
        }
    }

    /// Cancel any in-flight autosave and bump the generation token so it can
    /// no longer write even if it's already past its cancellation check.
    /// Called before any structural change (rename/delete) that a stale
    /// autosave snapshot could otherwise undo.
    private func invalidatePendingAutosave() {
        autosaveTask?.cancel()
        autosaveTask = nil
        autosaveGeneration += 1
    }

    /// Flush any pending edits immediately (e.g. when leaving the Studio).
    func saveNow() {
        invalidatePendingAutosave()
        for file in files {
            try? store.saveFile(file, in: projectName)
        }
    }

    private func report(_ error: Error) {
        let message = (error as? ProjectStoreError)?.errorDescription ?? error.localizedDescription
        consoleOutput = message
        consoleIsError = true
    }
}

/// A one-shot editor caret-move request. `token` makes two jumps to the same
/// line distinct so the editor re-applies them.
struct JumpTarget: Equatable {
    let line: Int
    let token = UUID()
}
