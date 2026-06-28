// Compiler-synthesized Equatable/Hashable (structs + enums) and the
// enum-only synthesized Comparable (Swift 5.3+).

// Struct Equatable + Hashable synthesis.
struct Point: Hashable { let x: Int; let y: Int }
print(Point(x: 1, y: 2) == Point(x: 1, y: 2))
print(Point(x: 1, y: 2) == Point(x: 1, y: 3))
let points: Set<Point> = [Point(x: 1, y: 2), Point(x: 1, y: 2), Point(x: 3, y: 4)]
print(points.count)

// Enum Equatable + Hashable synthesis (with associated values).
enum Token: Hashable { case number(Int); case word(String); case eof }
print(Token.number(3) == Token.number(3))
print(Token.number(3) == Token.number(4))
print(Token.word("a") == Token.word("a"))
let tokens: Set<Token> = [.number(1), .number(1), .word("x"), .eof]
print(tokens.count)

// Enum Comparable synthesis: order by case declaration, then by payload.
enum Suit: Comparable { case clubs, diamonds, hearts, spades }
print(Suit.clubs < Suit.spades)
print(Suit.hearts > Suit.diamonds)
print([Suit.spades, Suit.clubs, Suit.hearts].sorted().map { "\($0)" })

enum Size: Comparable {
    case small
    case medium(Int)
    case large(Int, Int)
}
print(Size.small < Size.medium(0))
print(Size.medium(1) < Size.medium(2))
print(Size.large(1, 9) < Size.large(2, 0))
print(Size.large(2, 1) < Size.large(2, 5))
print(Size.medium(2) >= Size.medium(2))
print([Size.large(1, 1), Size.small, Size.medium(9)].sorted().map { "\($0)" })
print(min(Size.medium(3), Size.small) == Size.small)

// A custom `<` still wins over synthesis when both are present.
enum Priority: Comparable {
    case low, high
    static func < (a: Priority, b: Priority) -> Bool {
        // Reversed: `high` sorts before `low`.
        a == .high && b == .low
    }
}
print(Priority.high < Priority.low)
