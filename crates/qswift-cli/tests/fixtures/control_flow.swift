var total = 0
for i in 0..<5 where i % 2 == 0 { total += i }
switch total {
case 0...3: print("small \(total)")
default:    print("big \(total)")
}
