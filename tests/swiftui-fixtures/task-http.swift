import SwiftUI
import Foundation

struct TaskHTTPView: View {
    @State private var title = "Loading"

    var body: some View {
        Text(title)
            .task {
                let (data, _) = try! await URLSession.shared.data(from: URL(string: "https://fixture.example/title")!)
                title = String(data: data, encoding: .utf8)!
            }
    }
}
