struct Point { var x: Int; var y: Int }
func distanceSquared(_ a: Point, _ b: Point) -> Int {
    let dx = a.x - b.x
    let dy = a.y - b.y
    return dx * dx + dy * dy
}
