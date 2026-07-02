// error pattern catches
enum NetError: Error { case timeout(seconds: Int), offline }
func f(_ n: Int) throws {
  if n == 1 { throw NetError.timeout(seconds: 30) }
  if n == 2 { throw NetError.offline }
}
for n in 1...2 {
  do {
    try f(n)
  } catch NetError.timeout(let s) {
    print("timeout \(s)")
  } catch NetError.offline where n > 1 {
    print("offline late")
  }
}
// defer ordering incl. thrown paths
func g() throws {
  defer { print("d1") }
  defer { print("d2") }
  print("body")
  throw NetError.offline
}
try? g()
// Result
let r: Result<Int, NetError> = .success(3)
print(try! r.get())
let e: Result<Int, NetError> = .failure(.offline)
switch e {
case .success(let v): print("ok \(v)")
case .failure(let err): print("err \(err)")
}
// generic subscripts + conditional conformance
struct Box<T> { var items: [T] }
extension Box where T: Comparable {
  var maxItem: T? { items.max() }
}
print(Box(items: [3, 9, 1]).maxItem ?? -1)
