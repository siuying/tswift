// Protocols: declaration, default impl, composition, existentials
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
    return "\(best.name): \(best.grade()) (\(best.score))"
}

let roster = [Student(name: "Ada", score: 95),
              Student(name: "Bob", score: 72),
              Student(name: "Eve", score: 88)]

for s in roster { print("\(s.name) → \(s.grade())") }
print("Top: \(topStudent(roster))")
