// expected-no-diagnostics
// Tier 4a — synthesized Equatable / Hashable, manual Comparable, CaseIterable.

struct Coordinate: Equatable, Hashable {
    var x: Int
    var y: Int
}

struct Version: Comparable {
    let major: Int
    let minor: Int
    static func < (lhs: Version, rhs: Version) -> Bool {
        (lhs.major, lhs.minor) < (rhs.major, rhs.minor)
    }
}

enum Suit: String, CaseIterable, Equatable {
    case hearts, spades, clubs, diamonds
}

let pointsEqual = Coordinate(x: 1, y: 2) == Coordinate(x: 1, y: 2)
var visited: Set<Coordinate> = []
visited.insert(Coordinate(x: 0, y: 0))
let ordered = Version(major: 1, minor: 0) < Version(major: 1, minor: 5)
let suits = Suit.allCases

let _ = (pointsEqual, visited.count, ordered, suits.count)
