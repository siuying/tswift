// expected-no-diagnostics
// Tier 0 — arithmetic / comparison / logical / bitwise / range / wrapping operators.

let arith = 5 + 3 * 2 - 8 / 4
let modulo = 17 % 5
let compare = arith > modulo && modulo >= 0 || arith == 9
let bits = (~modulo & 0xF) | (0x1 ^ 0x2)
let shifted = (arith << 2) >> 1

let halfOpen = 0 ..< 10
let closed = 0 ... 10
let fromStart = ..<5
let throughEnd = ...5

let coalesced = (nil as Int?) ?? 0
let wrappingAdd = UInt8.max &+ 1
let wrappingShift = 1 &<< 3