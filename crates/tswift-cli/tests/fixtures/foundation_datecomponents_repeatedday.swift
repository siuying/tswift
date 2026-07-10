import Foundation

// --- isRepeatedDay property ---
// Default: nil (not specified). Not settable via the initializer (no such
// label on Darwin's DateComponents init) — only assignable after the fact.
var dc = DateComponents(year: 2024, month: 11, day: 3, hour: 1, minute: 30)
print(dc.isRepeatedDay == nil)

dc.isRepeatedDay = true
print(dc.isRepeatedDay!)
print(dc)

dc.isRepeatedDay = false
print(dc.isRepeatedDay!)

dc.isRepeatedDay = nil
print(dc.isRepeatedDay == nil)

// A fresh DateComponents also defaults to nil.
let empty = DateComponents()
print(empty.isRepeatedDay == nil)
