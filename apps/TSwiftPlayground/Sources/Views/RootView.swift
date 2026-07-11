import SwiftUI

/// The app's top level: a tab bar with the multi-file **Projects** IDE and the
/// quick single-file **Scratchpad** playground (the original experience, kept
/// reachable).
struct RootView: View {
    @StateObject private var projects = ProjectsModel()

    var body: some View {
        TabView {
            ProjectListView(model: projects)
                .tabItem { Label("Projects", systemImage: "folder") }

            PlaygroundView()
                .tabItem { Label("Scratchpad", systemImage: "bolt") }
        }
    }
}
