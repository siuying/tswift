import SwiftUI

// SE-0279 multiple trailing closures: `NavigationLink { } label: { }` and
// `Button { } label: { }` — the labeled trailing closure supplies the view's
// content instead of a plain title string.
struct RootView: View {
    @State private var count = 0
    var body: some View {
        NavigationStack {
            VStack {
                Text("Count: \(count)")
                Button {
                    count += 1
                } label: {
                    Text("Increment")
                }
                NavigationLink {
                    Text("Count: \(count)")
                } label: {
                    Text("Show Detail")
                }
            }
        }
    }
}
