// `#sourceLocation(file:line:)` line control is accepted and is a no-op for the
// tree-walker; surrounding code still runs in order.
print("a")
#sourceLocation(file: "generated.swift", line: 1000)
print("b")
func work() -> Int {
    #sourceLocation(file: "macro.swift", line: 1)
    let x = 21
    return x * 2
}
print(work())
#sourceLocation()
print("c")
