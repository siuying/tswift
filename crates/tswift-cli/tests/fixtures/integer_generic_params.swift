struct FixedBuffer<let N: Int> {
  var storage: [Int] = []
  var capacity: Int { N }
  var remaining: Int { N - storage.count }
  mutating func push(_ v: Int) -> Bool {
    if storage.count >= N { return false }
    storage.append(v)
    return true
  }
}
var b = FixedBuffer<4>()
print(b.capacity)
print(b.push(1), b.push(2), b.push(3), b.push(4), b.push(5))
print(b.remaining)

struct Matrix<let R: Int, let C: Int> {
  var cells: Int { R * C }
}
print(Matrix<3, 4>().cells)

// Delegating initializers keep the specialization, and integer generic
// arguments accept any Swift radix.
struct Buf<let N: Int> {
  var used = 0
  init() {}
  init(prefill: Int) {
    self.init()
    self.used = prefill
  }
  var capacity: Int { N }
}
let b2 = Buf<8>(prefill: 3)
print(b2.capacity, b2.used)
print(Buf<0x10>().capacity)
