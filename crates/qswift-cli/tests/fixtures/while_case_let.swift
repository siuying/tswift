// `while let` and `while case` — refutable condition forms drive the loop and
// re-bind their pattern variables each iteration.

var n: Int? = 3
while let x = n {
    print(x)
    n = x > 1 ? x - 1 : nil
}

enum Step { case go(Int), stop }
let steps = [Step.go(1), Step.go(2), Step.stop, Step.go(3)]
var i = 0
while case .go(let m) = steps[i] {
    print("go \(m)")
    i += 1
}
print("done at \(i)")
