// expected-no-diagnostics
// Tier 11 / Web demo — Protocols: default impls, composition, any existential.

protocol Scorable {
    var score: Int { get }
    func grade() -> String
}

extension Scorable {
    func grade() -> String {
        switch score {
        case let n where n >= 90: return "A"
        case let n where n >= 80: return "B"
        case let n where n >= 70: return "C"
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
    return "\(best.name): \(best.grade()) (\(best.score))"
}

let roster = [
    Student(name: "Ada", score: 95),
    Student(name: "Bob", score: 72),
    Student(name: "Eve", score: 88),
]

for s in roster { print("\(s.name) → \(s.grade())") }
print("Top: \(topStudent(roster))")
