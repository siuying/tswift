func process() {
    print("start")
    defer { print("defer 1") }
    defer { print("defer 2") }
    for i in 1...3 {
        defer { print("loop defer \(i)") }
        print("iter \(i)")
    }
    print("end")
}
process()
