// expected-no-diagnostics
import Foundation

let emptyPath = IndexPath()
let singlePath = IndexPath(index: 1)
var path = IndexPath(indexes: [2, 4])
path.append(6)
let longer = path.appending(8)
let pathCount = longer.count

let emptySet = IndexSet()
let singleSet = IndexSet(integer: 9)
let closedSet = IndexSet(integersIn: 3 ... 5)
var set = IndexSet(integersIn: 3 ..< 6)
let hasFour = set.contains(4)
let first = set.first
let inserted = set.insert(8)
