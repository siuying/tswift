// A protocol-composition typealias used as a conformance still resolves a
// default method supplied by one of its component protocols.
protocol Scorable { var score: Int { get }; func grade() -> String }
extension Scorable {
    func grade() -> String { score >= 90 ? "A" : "B" }
}
protocol Named { var name: String { get } }
typealias NamedAndScored = Named & Scorable

struct Student: NamedAndScored { let name: String; let score: Int }

let roster: [any NamedAndScored] = [
    Student(name: "Ada", score: 95),
    Student(name: "Bob", score: 72),
]
for s in roster { print("\(s.name): \(s.grade())") }
