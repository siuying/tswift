// Literal conversion selects the matching literal initializer overload.
struct LiteralBox: ExpressibleByStringLiteral, ExpressibleByIntegerLiteral, ExpressibleByBooleanLiteral, ExpressibleByDictionaryLiteral {
    var tag: String
    init(stringLiteral value: String) { tag = "s:\(value)" }
    init(integerLiteral value: Int) { tag = "i:\(value)" }
    init(booleanLiteral value: Bool) { tag = value ? "b:true" : "b:false" }
    init(dictionaryLiteral elements: (String, Int)...) { tag = "d:\(elements.count)" }
}

let s: LiteralBox = "hi"
let i: LiteralBox = 7
let b: LiteralBox = false
let d: LiteralBox = ["a": 1, "b": 2]
let neg: LiteralBox = -5
func echo(_ box: LiteralBox) { print(box.tag) }
print(s.tag)
print(i.tag)
print(b.tag)
print(d.tag)
print(neg.tag)
echo("arg")
