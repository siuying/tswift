func greet(_ name: String, greeting: String = "Hello") -> String {
    return greeting + ", " + name
}
print(greet("Sam"))
print(greet("Sam", greeting: "Hi"))
func volume(width w: Int, height h: Int) -> Int {
    return w * h
}
print(volume(width: 3, height: 4))
