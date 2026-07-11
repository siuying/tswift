import Foundation

/// A named starter project seeded into the store on first launch. Mirrors the
/// single-file `Samples` gallery but as multi-file projects, and adds a console
/// program and a SwiftData sample (backed by the host `tswift.db` service).
struct ProjectTemplate {
    let name: String
    let files: [ProjectFile]

    static let all: [ProjectTemplate] = [counter, fruitList, consoleDemo, swiftData]

    /// A two-file SwiftUI project: a model file plus the view in `main.swift`.
    static let counter = ProjectTemplate(
        name: "Counter",
        files: [
            ProjectFile(name: "Counter.swift", contents: """
            /// A tiny model split into its own file to show multi-file resolution.
            struct Counter {
                var value = 0
                mutating func bump() { value += 1 }
            }
            """),
            ProjectFile(name: "main.swift", contents: """
            struct CounterView: View {
                @State private var counter = Counter()
                var body: some View {
                    VStack(spacing: 16) {
                        Text("\\(counter.value)")
                            .font(.largeTitle)
                            .fontWeight(.bold)
                        Button("Increment") { counter.bump() }
                            .foregroundColor(.white)
                            .padding()
                            .background(Color.blue)
                            .cornerRadius(8)
                    }
                }
            }
            """),
        ]
    )

    static let fruitList = ProjectTemplate(
        name: "Fruit List",
        files: [
            ProjectFile(name: "main.swift", contents: """
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
            """),
        ]
    )

    /// A console program: top-level statements print to the output pane.
    static let consoleDemo = ProjectTemplate(
        name: "Console Demo",
        files: [
            ProjectFile(name: "Primes.swift", contents: """
            /// Sieve of Eratosthenes, kept in its own file.
            func primes(upTo n: Int) -> [Int] {
                guard n >= 2 else { return [] }
                var sieve = Array(repeating: true, count: n + 1)
                var result: [Int] = []
                for i in 2...n where sieve[i] {
                    result.append(i)
                    var j = i * i
                    while j <= n { sieve[j] = false; j += i }
                }
                return result
            }
            """),
            ProjectFile(name: "main.swift", contents: """
            let ps = primes(upTo: 30)
            print("Primes up to 30:")
            print(ps.map(String.init).joined(separator: ", "))
            """),
        ]
    )

    /// A SwiftData sample. Runs against the host `tswift.db` service (real
    /// SQLite, in-memory store) wired by the Studio when running in console
    /// mode. Prints its progress so the output pane shows it worked.
    static let swiftData = ProjectTemplate(
        name: "SwiftData",
        files: [
            ProjectFile(name: "Movie.swift", contents: """
            import SwiftData

            @Model
            class Movie {
                var title: String
                var year: Int
                var rating: Double?
                init(title: String, year: Int, rating: Double? = nil) {
                    self.title = title
                    self.year = year
                    self.rating = rating
                }
            }
            """),
            ProjectFile(name: "main.swift", contents: """
            import SwiftData

            do {
                let config = ModelConfiguration(isStoredInMemoryOnly: true)
                let container = try ModelContainer(for: Movie.self, configurations: config)
                let ctx = container.mainContext

                let m = Movie(title: "Arrival", year: 2016)
                ctx.insert(m)
                try ctx.save()
                print("saved insert")

                m.year = 2017
                m.rating = 4.5
                try ctx.save()
                print("saved update")

                ctx.delete(m)
                try ctx.save()
                print("saved delete")
            } catch {
                print("unexpected: \\(error)")
            }
            """),
        ]
    )
}
