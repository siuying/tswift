import Foundation

// MARK: - Demo kind

/// Discriminates how a catalog item is executed/rendered.
enum DemoKind: Hashable {
    case console
    case swiftUI(needsNetwork: Bool)
}

// MARK: - Catalog item

/// A single runnable Swift example.
struct CatalogItem: Identifiable, Hashable {
    let id: String          // stable, human-readable slug
    let title: String
    let subtitle: String
    let source: String      // Swift source code shown on the left panel
    let kind: DemoKind
}

// MARK: - Catalog group

/// A named collection of related items shown as one sidebar section.
struct CatalogGroup: Identifiable {
    let id: String
    let name: String
    let items: [CatalogItem]
}

// MARK: - Static catalog

/// Full catalog ported from website/src/components/FullPlayground.astro ALL_PRESETS.
enum Catalog {
    static let all: [CatalogGroup] = [
        appsGroup,
        basicsGroup,
        functionsGroup,
        valueTypesGroup,
        referenceTypesGroup,
        protocolsGroup,
        errorsGroup,
        swiftUIGroup,
        stdlibGroup,
    ]

    // MARK: Apps
    private static let appsGroup = CatalogGroup(
        id: "apps",
        name: "Apps",
        items: [
            CatalogItem(
                id: "apps-hacker-news",
                title: "Hacker News Reader",
                subtitle: "Networked end-to-end app",
                source: """
                import Foundation

                struct HNStory: Decodable {
                    let id: Int
                    let title: String
                    let by: String
                    let score: Int
                    let url: String?
                }

                struct HNReaderView: View {
                    @State private var stories: [HNStory] = []
                    @State private var status = "Loading top stories…"

                    func loadStories() async {
                        do {
                            let topURL = URL(string: "https://hacker-news.firebaseio.com/v0/topstories.json")!
                            let (idsData, _) = try await URLSession.shared.data(from: topURL)
                            let ids = try JSONDecoder().decode([Int].self, from: idsData)
                            var result: [HNStory] = []
                            for id in ids.prefix(10) {
                                let itemURL = URL(string: "https://hacker-news.firebaseio.com/v0/item/\\(id).json")!
                                if let (itemData, _) = try? await URLSession.shared.data(from: itemURL),
                                   let story = try? JSONDecoder().decode(HNStory.self, from: itemData) {
                                    result.append(story)
                                }
                            }
                            stories = result
                            status = "Top \\(result.count) stories"
                        } catch {
                            status = "Error: \\(error)"
                        }
                    }

                    var body: some View {
                        NavigationStack {
                            List {
                                Section(status) {
                                    ForEach(stories, id: \\.id) { story in
                                        VStack(alignment: .leading, spacing: 4) {
                                            Text(story.title)
                                                .font(.headline)
                                            Text("\\(story.score) pts · by \\(story.by)")
                                                .font(.caption)
                                                .foregroundColor(.secondary)
                                        }
                                        .padding(.vertical, 2)
                                    }
                                }
                            }
                            .navigationTitle("Hacker News")
                        }
                        .task {
                            await loadStories()
                        }
                    }
                }
                """,
                kind: .swiftUI(needsNetwork: true)
            ),
        ]
    )

    // MARK: Basics
    private static let basicsGroup = CatalogGroup(
        id: "basics",
        name: "Basics",
        items: [
            CatalogItem(
                id: "basics-hello-world",
                title: "Hello World",
                subtitle: "Variables, print, ternary",
                source: """
                // Hello, tswift!
                let language = "Swift"
                let version = 6
                print("Hello from \\(language) \\(version)! 👋")

                let π = 3.14159
                let radius = 5.0
                let area = π * radius * radius
                print("Circle area (r=\\(radius)): \\(area)")

                let score = 87
                let grade = score >= 90 ? "A" : score >= 80 ? "B" : score >= 70 ? "C" : "F"
                print("Score \\(score) → Grade \\(grade)")
                """,
                kind: .console
            ),
            CatalogItem(
                id: "basics-fibonacci",
                title: "Fibonacci",
                subtitle: "Recursion, iteration",
                source: """
                func fib(_ n: Int) -> Int {
                    if n < 2 { return n }
                    return fib(n - 1) + fib(n - 2)
                }

                print("Fibonacci sequence:")
                for i in 0...12 {
                    print("  fib(\\(i)) = \\(fib(i))")
                }

                func fibFast(_ n: Int) -> Int {
                    var a = 0, b = 1
                    for _ in 0..<n { (a, b) = (b, a + b) }
                    return a
                }
                print("fibFast(20) = \\(fibFast(20))")
                """,
                kind: .console
            ),
        ]
    )

    // MARK: Functions
    private static let functionsGroup = CatalogGroup(
        id: "functions",
        name: "Functions",
        items: [
            CatalogItem(
                id: "functions-closures-hof",
                title: "Closures & HOF",
                subtitle: "Map, filter, reduce",
                source: """
                let numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

                let doubled = numbers.map { $0 * 2 }
                print("doubled: \\(doubled)")

                let evens = numbers.filter { $0 % 2 == 0 }
                print("evens: \\(evens)")

                let sum = numbers.reduce(0, +)
                print("sum: \\(sum)")

                let sumOfSquaredEvens = numbers
                    .filter { $0 % 2 == 0 }
                    .map    { $0 * $0 }
                    .reduce(0, +)
                print("sum of squared evens: \\(sumOfSquaredEvens)")

                func makeMultiplier(_ factor: Int) -> (Int) -> Int { { $0 * factor } }
                let triple = makeMultiplier(3)
                print("triple(7) = \\(triple(7))")

                let words = ["swift", "is", "fast", "and", "safe"]
                let shout = words.map { $0.uppercased() }.joined(separator: " ")
                print(shout)
                """,
                kind: .console
            ),
        ]
    )

    // MARK: Value Types
    private static let valueTypesGroup = CatalogGroup(
        id: "value-types",
        name: "Value Types",
        items: [
            CatalogItem(
                id: "value-types-structs",
                title: "Structs",
                subtitle: "Struct, mutation, copy",
                source: """
                struct Point {
                    var x: Double
                    var y: Double
                    var magnitude: Double { (x*x + y*y).squareRoot() }
                    mutating func translate(dx: Double, dy: Double) { x += dx; y += dy }
                    func scaled(by factor: Double) -> Point { Point(x: x * factor, y: y * factor) }
                }

                var p = Point(x: 3, y: 4)
                print("p = (\\(p.x), \\(p.y)), |p| = \\(p.magnitude)")

                var q = p
                q.translate(dx: 10, dy: 0)
                print("after translating q: p=\\(p.x), q=\\(q.x)  ← independent copies")

                let big = p.scaled(by: 2)
                print("scaled: (\\(big.x), \\(big.y))")
                """,
                kind: .console
            ),
            CatalogItem(
                id: "value-types-enums",
                title: "Enums",
                subtitle: "Cases, payloads, raw values",
                source: """
                enum Direction: CaseIterable {
                    case north, south, east, west
                    var opposite: Direction {
                        switch self {
                        case .north: return .south; case .south: return .north
                        case .east:  return .west;  case .west:  return .east
                        }
                    }
                }
                print("All directions: \\(Direction.allCases)")
                print("Opposite of north: \\(Direction.north.opposite)")

                enum Shape {
                    case circle(radius: Double)
                    case rectangle(width: Double, height: Double)
                    var area: Double {
                        switch self {
                        case .circle(let r):            return 3.14159 * r * r
                        case .rectangle(let w, let h):  return w * h
                        }
                    }
                }
                let shapes: [Shape] = [.circle(radius: 5), .rectangle(width: 4, height: 6)]
                for s in shapes { print("area = \\(s.area)") }

                enum Planet: Int { case mercury = 1, venus, earth, mars }
                print("Earth = \\(Planet.earth.rawValue)")
                """,
                kind: .console
            ),
            CatalogItem(
                id: "value-types-optionals",
                title: "Optionals",
                subtitle: "Optional binding, chaining",
                source: """
                let values: [Int?] = [1, nil, 3, nil, 5]
                for v in values {
                    if let x = v { print("got \\(x)") }
                    else { print("nil") }
                }

                func divide(_ a: Int, _ b: Int) -> Int? {
                    guard b != 0 else { return nil }
                    return a / b
                }

                let result = divide(10, 2)
                print("10/2 = \\(result ?? -1)")

                let bad = divide(5, 0)
                print("5/0 = \\(bad ?? -1)")

                // Optional chaining
                struct User { var address: Address? }
                struct Address { var city: String }
                let user = User(address: Address(city: "Cupertino"))
                print(user.address?.city ?? "unknown")
                """,
                kind: .console
            ),
        ]
    )

    // MARK: Reference Types
    private static let referenceTypesGroup = CatalogGroup(
        id: "reference-types",
        name: "Reference Types",
        items: [
            CatalogItem(
                id: "reference-types-classes",
                title: "Classes",
                subtitle: "Inheritance, references",
                source: """
                class Animal {
                    let name: String
                    init(_ name: String) { self.name = name }
                    func speak() -> String { "..." }
                }
                class Dog: Animal {
                    override func speak() -> String { "Woof!" }
                }
                class Cat: Animal {
                    override func speak() -> String { "Meow!" }
                }

                let animals: [Animal] = [Dog("Rex"), Cat("Whiskers"), Dog("Buddy")]
                for a in animals { print("\\(a.name): \\(a.speak())") }

                // Reference semantics
                class Counter {
                    var count = 0
                    func increment() { count += 1 }
                }
                let c1 = Counter()
                let c2 = c1   // same object
                c1.increment()
                c1.increment()
                print("c1=\\(c1.count), c2=\\(c2.count)  ← same reference")
                """,
                kind: .console
            ),
        ]
    )

    // MARK: Protocols
    private static let protocolsGroup = CatalogGroup(
        id: "protocols",
        name: "Protocols",
        items: [
            CatalogItem(
                id: "protocols-protocols",
                title: "Protocols",
                subtitle: "Extensions, composition",
                source: """
                protocol Scorable {
                    var score: Int { get }
                    func grade() -> String
                }
                extension Scorable {
                    func grade() -> String {
                        switch score {
                        case 90...: return "A"
                        case 80..<90: return "B"
                        case 70..<80: return "C"
                        default: return "F"
                        }
                    }
                }
                protocol Named { var name: String { get } }
                typealias NamedAndScored = Named & Scorable

                struct Student: NamedAndScored {
                    let name: String
                    let score: Int
                }

                func topStudent(_ students: [any NamedAndScored]) -> String {
                    guard let best = students.max(by: { $0.score < $1.score }) else { return "none" }
                    return "\\(best.name): \\(best.grade()) (\\(best.score))"
                }

                let roster = [Student(name: "Ada", score: 95),
                              Student(name: "Bob", score: 72),
                              Student(name: "Eve", score: 88)]
                for s in roster { print("\\(s.name) → \\(s.grade())") }
                print("Top: \\(topStudent(roster))")
                """,
                kind: .console
            ),
            CatalogItem(
                id: "protocols-generics",
                title: "Generics",
                subtitle: "Type parameters, stack",
                source: """
                func maxOf<T: Comparable>(_ a: T, _ b: T) -> T { a > b ? a : b }
                print(maxOf(3, 7))
                print(maxOf("apple", "banana"))

                struct Stack<Element> {
                    private var items: [Element] = []
                    mutating func push(_ item: Element) { items.append(item) }
                    mutating func pop() -> Element? { items.popLast() }
                    var top: Element? { items.last }
                    var isEmpty: Bool { items.isEmpty }
                }

                var s = Stack<Int>()
                s.push(1); s.push(2); s.push(3)
                print("top: \\(s.top!)")
                while let x = s.pop() { print("popped \\(x)") }
                """,
                kind: .console
            ),
        ]
    )

    // MARK: Errors
    private static let errorsGroup = CatalogGroup(
        id: "errors",
        name: "Errors",
        items: [
            CatalogItem(
                id: "errors-error-handling",
                title: "Error Handling",
                subtitle: "Throw, catch, try?",
                source: """
                enum ValidationError: Error {
                    case empty
                    case tooShort(Int)
                    case invalidChar(Character)
                }

                func validate(_ password: String) throws -> String {
                    guard !password.isEmpty else { throw ValidationError.empty }
                    guard password.count >= 6 else { throw ValidationError.tooShort(password.count) }
                    for ch in password {
                        guard ch.isLetter || ch.isNumber else { throw ValidationError.invalidChar(ch) }
                    }
                    return "✓ valid"
                }

                let inputs = ["", "hi", "hello!", "secure123"]
                for pwd in inputs {
                    do {
                        let msg = try validate(pwd)
                        print("'\\(pwd)' → \\(msg)")
                    } catch ValidationError.empty {
                        print("'\\(pwd)' → error: empty")
                    } catch ValidationError.tooShort(let n) {
                        print("'\\(pwd)' → error: too short (\\(n) chars)")
                    } catch ValidationError.invalidChar(let c) {
                        print("'\\(pwd)' → error: bad char '\\(c)'"  )
                    }
                }

                let safe = try? validate("abc123")
                print("try? 'abc123' → \\(safe ?? "nil")")
                """,
                kind: .console
            ),
        ]
    )

    // MARK: SwiftUI
    private static let swiftUIGroup = CatalogGroup(
        id: "swiftui",
        name: "SwiftUI",
        items: [
            CatalogItem(
                id: "swiftui-counter",
                title: "Counter",
                subtitle: "State, button tap",
                source: """
                struct CounterView: View {
                    @State private var count = 0
                    var body: some View {
                        VStack {
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
                """,
                kind: .swiftUI(needsNetwork: false)
            ),
            CatalogItem(
                id: "swiftui-toggle",
                title: "Toggle",
                subtitle: "Toggle, conditional view",
                source: """
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
                """,
                kind: .swiftUI(needsNetwork: false)
            ),
            CatalogItem(
                id: "swiftui-list",
                title: "List",
                subtitle: "ForEach, HStack",
                source: """
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
                """,
                kind: .swiftUI(needsNetwork: false)
            ),
            CatalogItem(
                id: "swiftui-profile",
                title: "Profile",
                subtitle: "VStack layout card",
                source: """
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
                """,
                kind: .swiftUI(needsNetwork: false)
            ),
        ]
    )

    // MARK: Stdlib
    private static let stdlibGroup = CatalogGroup(
        id: "stdlib",
        name: "Stdlib",
        items: [
            CatalogItem(
                id: "stdlib-collections",
                title: "Collections",
                subtitle: "Array, Dict, Set",
                source: """
                // Array operations
                var fruits = ["apple", "banana", "cherry"]
                fruits.append("date")
                print("fruits: \\(fruits)")
                print("sorted: \\(fruits.sorted())")
                print("filtered: \\(fruits.filter { $0.count > 5 })")

                // Dictionary
                var scores: [String: Int] = ["Alice": 95, "Bob": 72, "Eve": 88]
                scores["Charlie"] = 91
                print("scores: \\(scores)")
                let avg = scores.values.reduce(0, +) / scores.count
                print("average: \\(avg)")

                // Set
                var set1: Set = [1, 2, 3, 4]
                let set2: Set = [3, 4, 5, 6]
                print("union: \\(set1.union(set2).sorted())")
                print("intersection: \\(set1.intersection(set2).sorted())")
                """,
                kind: .console
            ),
        ]
    )
}
