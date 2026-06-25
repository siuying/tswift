// oracle-gap: the C msf does not lex unicode operator characters like √ and °.
// Tier 9b — custom operators using non-ASCII operator characters.

prefix operator √
postfix operator °

prefix func √ (value: Double) -> Double {
    var guess = value
    for _ in 0 ..< 20 {
        guess = (guess + value / guess) / 2
    }
    return guess
}

postfix func ° (degrees: Double) -> Double {
    degrees * 3.14159 / 180
}

let root = √16.0
let radians = 90.0°

let _ = (root, radians)
