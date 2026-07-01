protocol Container {
  associatedtype Item
  var items: [Item] { get }
}
struct IntBox: Container { var items: [Int] }
struct StrBox: Container { var items: [String] }

func allItemsMatch<C1: Container, C2: Container>(_ a: C1, _ b: C2) -> Bool
where C1.Item == C2.Item, C1.Item: Equatable {
  if a.items.count != b.items.count { return false }
  for i in 0..<a.items.count {
    if a.items[i] != b.items[i] { return false }
  }
  return true
}
print(allItemsMatch(IntBox(items: [1, 2]), IntBox(items: [1, 2])))
print(allItemsMatch(IntBox(items: [1]), IntBox(items: [2])))

extension Container where Item == Int {
  func total() -> Int { return items.reduce(0, +) }
}
print(IntBox(items: [3, 4, 5]).total())

func sum<C: Container>(_ c: C) -> Int where C.Item == Int {
  var t = 0
  for x in c.items { t += x }
  return t
}
print(sum(IntBox(items: [10, 20])))
