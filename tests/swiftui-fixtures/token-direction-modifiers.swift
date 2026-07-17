import SwiftUI

struct TokenDirectionModifiers: View {
    var body: some View {
        VStack {
            Text("Direction")
                .writingDirection(.leftToRight)
            TabView {
                Text("Tab")
            }
            .tabViewSearchActivation(.searchTabSelection)
        }
    }
}
