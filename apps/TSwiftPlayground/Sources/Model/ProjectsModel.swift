import Foundation

/// SwiftUI-facing wrapper over `ProjectStore`: publishes the project name list
/// for the project-list screen and funnels mutations through the store,
/// surfacing failures as a presentable `error`.
@MainActor
final class ProjectsModel: ObservableObject {
    let store: ProjectStore
    @Published private(set) var projectNames: [String] = []
    @Published var error: String?

    init(store: ProjectStore = ProjectStore()) {
        self.store = store
        seedSamplesIfEmpty()
        reload()
    }

    func reload() {
        projectNames = store.projectNames()
    }

    @discardableResult
    func createProject(_ name: String, seed: [ProjectFile]? = nil) -> Bool {
        do {
            try store.createProject(name, seed: seed)
            reload()
            return true
        } catch {
            self.error = (error as? ProjectStoreError)?.errorDescription ?? error.localizedDescription
            return false
        }
    }

    func deleteProject(_ name: String) {
        do {
            try store.deleteProject(name)
            reload()
        } catch {
            self.error = (error as? ProjectStoreError)?.errorDescription ?? error.localizedDescription
        }
    }

    func renameProject(_ old: String, to new: String) {
        do {
            try store.renameProject(old, to: new)
            reload()
        } catch {
            self.error = (error as? ProjectStoreError)?.errorDescription ?? error.localizedDescription
        }
    }

    /// On first launch, populate the store with the bundled starter projects so
    /// the IDE is not an empty screen. Only runs when no projects exist yet.
    private func seedSamplesIfEmpty() {
        guard store.projectNames().isEmpty else { return }
        for template in ProjectTemplate.all {
            _ = try? store.createProject(template.name, seed: template.files)
        }
    }
}
