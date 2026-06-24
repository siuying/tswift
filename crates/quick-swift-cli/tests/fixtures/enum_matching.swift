enum Shape { case circle(r: Int); case rect(Int, Int) }
func area(_ s: Shape) -> Int {
    switch s {
    case .circle(let r): return 3 * r * r
    case .rect(let w, let h): return w * h
    }
}
print(area(.circle(r: 5)))
print(area(Shape.rect(3, 4)))
