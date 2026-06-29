import Foundation

/// A starter snippet for the gallery picker.
struct Sample: Identifiable {
    let id = UUID()
    let label: String
    let code: String
}

/// Bundled starter snippets, mirroring the website playground gallery so the two
/// surfaces stay discoverable in lockstep. Kept as inline strings (no resource
/// bundle wiring) for a dependency-light first cut.
enum Samples {
    static let all: [Sample] = [
        Sample(
            label: "Counter",
            code: """
            struct CounterView: View {
                @State private var count = 0
                var body: some View {
                    VStack(spacing: 16) {
                        Text("\\(count)")
                            .font(.largeTitle)
                            .fontWeight(.bold)
                        Button("Increment") { count += 1 }
                            .foregroundColor(.white)
                            .padding()
                            .background(Color.blue)
                            .cornerRadius(8)
                    }
                }
            }
            """
        ),
        Sample(
            label: "Toggle",
            code: """
            struct GreetingView: View {
                @State private var isOn = true
                var body: some View {
                    VStack(spacing: 16) {
                        Toggle("Show greeting", isOn: $isOn)
                            .padding()
                        if isOn {
                            Text("Hello, SwiftUI! 👋")
                                .font(.title)
                                .foregroundColor(.blue)
                        }
                    }
                    .padding()
                }
            }
            """
        ),
        Sample(
            label: "List",
            code: """
            struct FruitList: View {
                let fruits = ["Apple", "Banana", "Cherry", "Date"]
                var body: some View {
                    List {
                        ForEach(fruits, id: \\.self) { fruit in
                            HStack {
                                Text(fruit)
                                Spacer()
                                Text("🍎")
                            }
                        }
                    }
                }
            }
            """
        ),
        Sample(
            label: "Profile",
            code: """
            struct ProfileCard: View {
                var body: some View {
                    VStack(spacing: 12) {
                        Text("🦜")
                            .font(.largeTitle)
                        Text("Unlucky Parrot")
                            .font(.title)
                            .fontWeight(.bold)
                        Text("SwiftUI on tswift")
                            .foregroundColor(.secondary)
                        Button("Follow") { }
                            .foregroundColor(.white)
                            .padding()
                            .background(Color.blue)
                            .cornerRadius(10)
                    }
                    .padding()
                }
            }
            """
        ),
    ]

    /// The snippet the app opens with.
    static var initial: Sample { all[0] }
}
