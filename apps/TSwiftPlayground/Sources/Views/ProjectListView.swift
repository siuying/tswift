import SwiftUI

/// The project browser: create, open, rename, and delete projects. Each row
/// pushes the `StudioView` mini-IDE for that project.
struct ProjectListView: View {
    @ObservedObject var model: ProjectsModel
    @State private var showingNew = false
    @State private var newName = ""
    @State private var renaming: String?
    @State private var renameText = ""
    @State private var deleting: String?

    var body: some View {
        NavigationStack {
            List {
                if model.projectNames.isEmpty {
                    EmptyStateView(
                        title: "No Projects",
                        systemImage: "folder.badge.plus",
                        message: "Tap + to create your first project."
                    )
                } else {
                    ForEach(model.projectNames, id: \.self) { name in
                        NavigationLink(value: name) {
                            Label(name, systemImage: "swift")
                        }
                        .swipeActions(edge: .trailing) {
                            Button(role: .destructive) {
                                deleting = name
                            } label: { Label("Delete", systemImage: "trash") }
                            Button {
                                renaming = name
                                renameText = name
                            } label: { Label("Rename", systemImage: "pencil") }
                            .tint(.blue)
                        }
                    }
                }
            }
            .navigationTitle("Projects")
            .navigationDestination(for: String.self) { name in
                studio(for: name)
            }
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        newName = ""
                        showingNew = true
                    } label: { Image(systemName: "plus") }
                }
            }
            .alert("New Project", isPresented: $showingNew) {
                TextField("Name", text: $newName)
                Button("Create") { _ = model.createProject(newName) }
                Button("Cancel", role: .cancel) {}
            }
            .alert("Rename Project", isPresented: Binding(
                get: { renaming != nil },
                set: { if !$0 { renaming = nil } }
            )) {
                TextField("Name", text: $renameText)
                Button("Rename") {
                    if let old = renaming { model.renameProject(old, to: renameText) }
                    renaming = nil
                }
                Button("Cancel", role: .cancel) { renaming = nil }
            }
            .alert("Error", isPresented: Binding(
                get: { model.error != nil },
                set: { if !$0 { model.error = nil } }
            )) {
                Button("OK", role: .cancel) { model.error = nil }
            } message: {
                Text(model.error ?? "")
            }
            .confirmationDialog(
                "Delete \u{201C}\(deleting ?? "")\u{201D}?",
                isPresented: Binding(
                    get: { deleting != nil },
                    set: { if !$0 { deleting = nil } }
                ),
                titleVisibility: .visible
            ) {
                Button("Delete", role: .destructive) {
                    if let name = deleting { model.deleteProject(name) }
                    deleting = nil
                }
                Button("Cancel", role: .cancel) { deleting = nil }
            } message: {
                Text("This deletes the project and all its files. This cannot be undone.")
            }
        }
    }

    @ViewBuilder
    private func studio(for name: String) -> some View {
        if let project = try? model.store.loadProject(name) {
            StudioView(model: StudioModel(store: model.store, project: project))
        } else {
            EmptyStateView(title: "Could not open project", systemImage: "exclamationmark.triangle")
        }
    }
}

/// A back-deployable empty-state placeholder (iOS 16 has no
/// `ContentUnavailableView`).
struct EmptyStateView: View {
    let title: String
    let systemImage: String
    var message: String? = nil

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: systemImage)
                .font(.largeTitle)
                .foregroundStyle(.secondary)
            Text(title)
                .font(.headline)
            if let message {
                Text(message)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 40)
    }
}
