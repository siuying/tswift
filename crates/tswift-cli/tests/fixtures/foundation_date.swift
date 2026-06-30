import Foundation

let unix = Date(timeIntervalSince1970: 978307210.0)
let reference = Date(timeIntervalSinceReferenceDate: 40.0)
let relative = Date(timeInterval: 5.0, since: unix)

print(unix.timeIntervalSinceReferenceDate)
print(unix.timeIntervalSince1970)
print(Date.timeIntervalBetween1970AndReferenceDate)
print(relative.timeIntervalSinceReferenceDate)
print(reference.timeIntervalSince(unix))
print(unix.distance(to: reference))

let added = unix.addingTimeInterval(2.5)
print(added.timeIntervalSinceReferenceDate)

var mutated = unix
mutated.addTimeInterval(3.0)
print(mutated.timeIntervalSinceReferenceDate)

let advanced = unix.advanced(by: 4.0)
print(advanced.timeIntervalSinceReferenceDate)

let plus = unix + 5.0
let minus = reference - 10.0
print(plus.timeIntervalSinceReferenceDate)
print(minus.timeIntervalSinceReferenceDate)
print(reference - unix)

var assigned = unix
assigned += 6.0
assigned -= 1.0
print(assigned.timeIntervalSinceReferenceDate)

print(unix < reference)
print(reference > unix)
print(unix == Date(timeIntervalSinceReferenceDate: 10.0))
print(unix.compare(reference))
print(Date.distantPast < unix)
print(Date.distantFuture > reference)
