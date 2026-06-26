// readLine reads successive lines from stdin, returning nil at end of input.
let name = readLine() ?? "?"
let age = readLine() ?? "?"
print("name: \(name)")
print("age: \(age)")
print(readLine() == nil)
