// expected-no-diagnostics
// oracle-gap: C msf predates SE-0452 integer generic parameters

struct FixedBuffer<let N: Int> {
    var storage: [Int] = []
    var capacity: Int { N }

    mutating func push(_ v: Int) -> Bool {
        if storage.count >= N { return false }
        storage.append(v)
        return true
    }
}

struct Matrix<let R: Int, let C: Int> {
    var cells: Int { R * C }
}

var buffer = FixedBuffer<4>()
let _ = buffer.push(1)
let _ = Matrix<3, 4>().cells
