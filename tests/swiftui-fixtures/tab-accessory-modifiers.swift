import SwiftUI

struct TabAccessoryModifiers: View {
    var body: some View {
        TabView {
            Text("Home")
        }
        .tabViewBottomAccessory {
            Text("Now Playing")
        }
        .tabViewSidebarHeader {
            Text("Library")
        }
        .tabViewSidebarFooter {
            Text("Settings")
        }
        .tabViewSidebarBottomBar {
            Text("Account")
        }
        .textInputSuggestions {
            Text("Suggestion")
        }
    }
}
