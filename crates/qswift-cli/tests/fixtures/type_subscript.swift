// Type subscripts: `static subscript` addressed through the type name.

struct Multiplier {
    static subscript(i: Int) -> Int {
        return i * 3
    }
}

print(Multiplier[4])

struct Matrix {
    static subscript(row: Int, col: Int) -> Int {
        return row * 10 + col
    }
}

print(Matrix[2, 7])

class Registry {
    static subscript(id: Int) -> String {
        return "item-\(id)"
    }
}

print(Registry[42])
