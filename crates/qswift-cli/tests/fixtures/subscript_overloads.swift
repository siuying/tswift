// Multiple `subscript` overloads selected by arity, with get/set accessors,
// plus nested-subscript assignment into a 2-D array.
struct Grid {
    var data: [[Int]]

    // Two-parameter element access (read/write).
    subscript(_ r: Int, _ c: Int) -> Int {
        get { data[r][c] }
        set { data[r][c] = newValue }
    }

    // One-parameter row access (read-only).
    subscript(_ row: Int) -> [Int] {
        get { data[row] }
    }
}

var g = Grid(data: [[1, 2], [3, 4]])
print(g[0, 1])
g[0, 1] = 9
print(g[0, 1])
g[1, 0] += 10
print(g[1, 0])
print(g[1])

// Direct nested-subscript assignment on a 2-D array.
var m = [[0, 0], [0, 0]]
m[0][1] = 5
m[1][0] = 7
m[0][1] += 1
print(m)

// Dictionary subscript with insert, update, and remove still works.
var counts = ["a": 1]
counts["b"] = 2
counts["a"] = nil
print(counts.count)
print(counts["b"]!)
