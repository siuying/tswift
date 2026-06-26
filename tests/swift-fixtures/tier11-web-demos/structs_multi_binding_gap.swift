// frontend-gap: multi-variable binding 'var a, b, c, d: T' not yet parsed —
// the parser treats the comma-separated names after the first as separate
// statements and emits "consecutive statements must be separated by ';'".
//
// Valid Swift 6 per TSPL §Declarations §Variable Declaration.
// Remove this file once the parser handles multi-binding stored properties.

struct Matrix2x2 {
    var a, b, c, d: Double
    func determinant() -> Double { a * d - b * c }
}

let m = Matrix2x2(a: 1, b: 2, c: 3, d: 4)
let _ = m.determinant()
