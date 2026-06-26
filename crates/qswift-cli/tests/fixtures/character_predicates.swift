// Character predicate properties on single-grapheme characters.
let samples: [Character] = ["A", "7", " ", "z"]
for ch in samples {
    print(ch.isLetter, ch.isNumber, ch.isWhitespace, ch.isUppercase)
}
let hex: Character = "F"
let notHex: Character = "G"
print(hex.isHexDigit, notHex.isHexDigit)
