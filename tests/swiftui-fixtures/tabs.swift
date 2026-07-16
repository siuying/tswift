import SwiftUI

struct TabsView: View {
    @State private var selection = "home"
    var body: some View {
        TabView(selection: $selection) {
            VStack {
                Text("Home screen")
                Text("selected: \(selection)")
            }
            .tabItem {
                Label("Home", systemImage: "house.fill")
            }
            .tag("home")

            Text("Search screen")
                .tabItem {
                    Label("Search", systemImage: "magnifyingglass")
                }
                .tag("search")

            Text("Settings screen")
                .tabItem {
                    Label("Settings", systemImage: "gear")
                }
                .tag("settings")
        }
    }
}
