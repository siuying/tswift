// String APIs: split, join, prefix/suffix, search, case, reverse
let text = "The quick brown fox jumps over the lazy dog"
print("length: \(text.count)")
print("upper:  \(text.uppercased())")
print("prefix: \(text.prefix(9))")
print("suffix: \(text.suffix(8))")
print("hasPrefix 'The':   \(text.hasPrefix("The"))")
print("contains 'fox':    \(text.contains("fox"))")

let words = text.split(separator: " ")
print("word count: \(words.count)")

let long = words.filter { $0.count > 4 }.map { $0.uppercased() }
print("long words: \(long.joined(separator: ", "))")

let rev = String("Hello".reversed())
print("reversed: \(rev)")

// Interpolation + multiline
let name = "qswift"
let version = 6
let banner = """
  ╔═══════════════════════╗
  ║  Welcome to \(name)!  ║
  ║  Swift version \(version)       ║
  ╚═══════════════════════╝
  """
print(banner)
