// expected-no-diagnostics
// Tier 2d — instance subscripts (get/set, multi-param), overloads, and a
// static subscript.

struct Matrix {
    let rows: Int
    let cols: Int
    var grid: [Int]

    init(rows: Int, cols: Int) {
        self.rows = rows
        self.cols = cols
        self.grid = Array(repeating: 0, count: rows * cols)
    }

    subscript(r: Int, c: Int) -> Int {
        get { grid[r * cols + c] }
        set { grid[r * cols + c] = newValue }
    }
}

struct Lookup {
    subscript(key: String) -> Int { key.count }
    subscript(index: Int) -> String { "#\(index)" }
    static subscript(tag label: String) -> String { "static-\(label)" }
}

var m = Matrix(rows: 2, cols: 2)
m[0, 1] = 9
let cell = m[0, 1]

let table = Lookup()
let byString = table["hello"]
let byInt = table[3]
let byStatic = Lookup[tag: "x"]

let _ = (cell, byString, byInt, byStatic)