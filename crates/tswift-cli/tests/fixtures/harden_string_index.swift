// harden_string_index.swift — edge cases from Apple's StringIndex.swift / subString.swift.
// Ground-truthed with local Swift toolchain. Skips: utf8/utf16/unicodeScalars views.

// ── 1. Empty string indices ──────────────────────────────────────────────────
let empty = ""
print(empty.startIndex == empty.endIndex)      // true
print(empty.count == 0)                        // true
print(empty.isEmpty)                           // true
print(empty.distance(from: empty.startIndex, to: empty.endIndex))  // 0

// ── 2. index(offsetBy: 0) is a no-op regardless of position ──────────────────
let hello = "Hello"
let hStart = hello.startIndex
let hEnd   = hello.endIndex
print(hello.index(hStart, offsetBy: 0) == hStart)  // true
print(hello.index(hEnd,   offsetBy: 0) == hEnd)    // true

// ── 3. Negative offsetBy goes backwards ──────────────────────────────────────
let neg2 = hello.index(hEnd, offsetBy: -2)     // 2 back from end = 'l'
print(hello[neg2])                             // l
let neg5 = hello.index(hEnd, offsetBy: -5)     // = startIndex
print(neg5 == hStart)                          // true

// ── 4. distance symmetry ─────────────────────────────────────────────────────
let h4 = hello.index(hStart, offsetBy: 4)
let fwd = hello.distance(from: hStart, to: h4)
let bwd = hello.distance(from: h4, to: hStart)
print(fwd)                                     // 4
print(bwd)                                     // -4
print(fwd == -bwd)                             // true

// ── 5. distance(start, end) == count ─────────────────────────────────────────
print(hello.distance(from: hStart, to: hEnd) == hello.count)  // true

// ── 6. limitedBy: offset exactly reaches limit → returns index (NOT nil) ─────
// "Hello" count=5; index(start, offsetBy:5) == endIndex
let atLimit = hello.index(hStart, offsetBy: 5, limitedBy: hEnd)
print(atLimit == nil)                          // false
print(atLimit == hEnd)                         // true

// ── 7. limitedBy: offset crosses limit → nil ─────────────────────────────────
let crossLimit = hello.index(hStart, offsetBy: 6, limitedBy: hEnd)
print(crossLimit == nil)                       // true

// ── 8. limitedBy: offsetBy 0 never triggers the limit ────────────────────────
// n=0 means "don't move", so limit must not apply regardless of its position.
let zeroNoMove = hello.index(hEnd, offsetBy: 0, limitedBy: hStart)
print(zeroNoMove == nil)                       // false
print(zeroNoMove == hEnd)                      // true

// ── 9. limitedBy backward: destination exactly equals limit → not nil ─────────
// index(end=5, offsetBy:-2) = 3; limit h3 = 3 → exactly at limit, not crossed
let h3 = hello.index(hStart, offsetBy: 3)
let atLimitBack = hello.index(hEnd, offsetBy: -2, limitedBy: h3)
print(atLimitBack == nil)                      // false
print(atLimitBack == h3)                       // true

// ── 10. limitedBy backward: crosses limit → nil ──────────────────────────────
// index(end=5, offsetBy:-3) = 2; limit h3=3 → 2 < 3 → crossed → nil
let crossLimitBack = hello.index(hEnd, offsetBy: -3, limitedBy: h3)
print(crossLimitBack == nil)                   // true

// ── 11. CRLF is a single grapheme cluster ────────────────────────────────────
let crlf = "\r\n"
print(crlf.count)                              // 1
print(crlf.index(after: crlf.startIndex) == crlf.endIndex)   // true
print(crlf.distance(from: crlf.startIndex, to: crlf.endIndex))  // 1

// ── 12. CRLF in mixed ASCII string ────────────────────────────────────────────
let mixed = "ab\r\ncd"
print(mixed.count)                             // 5 (a, b, \r\n, c, d)
print(mixed.distance(from: mixed.startIndex, to: mixed.endIndex))  // 5
// Verify \r\n is one cluster: index after 'b' steps over \r\n in one hop
let mixedAt2 = mixed.index(mixed.startIndex, offsetBy: 2)
let mixedAt3 = mixed.index(mixed.startIndex, offsetBy: 3)
print(mixed.index(after: mixedAt2) == mixedAt3)  // true

// ── 13. Flag emoji (regional indicators pair into one cluster) ────────────────
let usFlag = "🇺🇸"
print(usFlag.count)                            // 1
print(usFlag.index(after: usFlag.startIndex) == usFlag.endIndex)  // true

// ── 14. Two flags are two clusters ───────────────────────────────────────────
let twoFlags = "🇺🇸🇨🇦"
print(twoFlags.count)                          // 2
let flagMid = twoFlags.index(after: twoFlags.startIndex)
print(flagMid == twoFlags.index(twoFlags.startIndex, offsetBy: 1))  // true
print(twoFlags.distance(from: twoFlags.startIndex, to: twoFlags.endIndex))  // 2

// ── 15. Decomposed combining accent = 1 grapheme ─────────────────────────────
let eAcute = "e\u{301}"   // e + combining acute = é
print(eAcute.count)                            // 1
print(eAcute.index(after: eAcute.startIndex) == eAcute.endIndex)   // true

// ── 16. Precomposed vs decomposed — same grapheme count ───────────────────────
let precomposed  = "café"
let decomposed   = "cafe\u{301}"
print(precomposed.count)                       // 4
print(decomposed.count)                        // 4

// ── 17. Single-char string: after(start) == end, before(end) == start ─────────
let single = "A"
print(single.index(after: single.startIndex) == single.endIndex)    // true
print(single.index(before: single.endIndex) == single.startIndex)   // true

// ── 18. Substring: startIndex/endIndex are base-relative ──────────────────────
let base = "Hello, World"
let bi3 = base.index(base.startIndex, offsetBy: 3)
let bi8 = base.index(base.startIndex, offsetBy: 8)
let sub  = base[bi3..<bi8]
print(sub)                                     // lo, W
print(sub.startIndex == bi3)                   // true
print(sub.endIndex   == bi8)                   // true
print(sub.count)                               // 5
print(sub.distance(from: sub.startIndex, to: sub.endIndex))  // 5

// ── 19. Substring of Substring keeps base-relative indices ───────────────────
let bi4 = base.index(base.startIndex, offsetBy: 4)
let sub2 = sub[bi4..<bi8]                      // "o, W" (base offsets 4–8)
print(sub2)                                    // o, W
print(sub2.startIndex == bi4)                  // true
print(sub2.count)                              // 4

// ── 20. Substring.base returns the full parent string ────────────────────────
print(sub.base == base)                        // true

// ── 21. Substring.base of sub-of-sub ────────────────────────────────────────
print(sub2.base == base)                       // true

// ── 22. Substring limitedBy at its boundary (not nil) ────────────────────────
let sentence = "Swift is great"
let si5  = sentence.index(sentence.startIndex, offsetBy: 5)
let si13 = sentence.index(sentence.startIndex, offsetBy: 13)
let word = sentence[si5..<si13]               // " is grea"
let wordAtEnd = word.index(word.startIndex, offsetBy: word.count, limitedBy: word.endIndex)
print(wordAtEnd == nil)                        // false
print(wordAtEnd == word.endIndex)              // true

// ── 23. Substring limitedBy past end → nil ────────────────────────────────────
let nilInSub = word.index(word.startIndex, offsetBy: 20, limitedBy: word.endIndex)
print(nilInSub == nil)                         // true

// ── 24. Substring replaceSubrange is COW — original string unchanged ──────────
var baseStr = "abcdefg"
let mbi2 = baseStr.index(baseStr.startIndex, offsetBy: 2)
let mbi4 = baseStr.index(baseStr.startIndex, offsetBy: 4)
var mSub = baseStr[mbi2..<mbi4]               // "cd"
mSub.replaceSubrange(mSub.startIndex..<mSub.endIndex, with: "XY")
print(mSub)                                    // XY
print(baseStr)                                 // abcdefg

// ── 25. index(offsetBy:) within substring uses base-relative indices ──────────
let si2 = word.index(word.startIndex, offsetBy: 2)   // base offset 7 = 's'
print(word[si5..<si2])                         // _i  (space + i, where _ = space)
