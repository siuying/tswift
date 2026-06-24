outer: for i in 1...3 {
    for j in 1...3 {
        if j == 2 { continue outer }
        if i == 3 { break outer }
        print("\(i),\(j)")
    }
}
