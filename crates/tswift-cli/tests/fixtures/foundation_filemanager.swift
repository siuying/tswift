import Foundation

// `FileManager.default` is a stable singleton; the CLI backs it with the
// real filesystem (see `tswift-cli/src/fs.rs`). The golden-fixture harness
// runs this file with its working directory set to a fresh, unique temp
// directory (see `<name>.isolated_cwd` in `tests/golden.rs`) — never a
// predictable shared path — so `root` below is a relative path resolved
// against that fresh cwd, and is guaranteed not to pre-exist.
struct CocoaError: Error { let code: Int; let message: String }

let fm = FileManager.default
let root = "tswift_filemanager_fixture"

// createDirectory / fileExists.
try! fm.createDirectory(atPath: root, withIntermediateDirectories: true)
print(fm.fileExists(atPath: root))
print(fm.fileExists(atPath: root + "/missing"))

// createFile / contents round-trip through Data <-> String.
let filePath = root + "/hello.txt"
let created = fm.createFile(atPath: filePath, contents: Data("Hello, tswift!".utf8))
print(created)
if let data = fm.contents(atPath: filePath) {
    print(String(data: data, encoding: .utf8)!)
}
print(fm.contents(atPath: root + "/nope.txt") == nil)
let attributes = try! fm.attributesOfItem(atPath: filePath)
print(attributes["size"] != nil)

// contentsOfDirectory lists entry names.
_ = fm.createFile(atPath: root + "/b.txt", contents: Data("b".utf8))
_ = fm.createFile(atPath: root + "/a.txt", contents: Data("a".utf8))
let names = try! fm.contentsOfDirectory(atPath: root)
print(names.sorted())

// copyItem / moveItem.
let copyDst = root + "/hello-copy.txt"
try! fm.copyItem(atPath: filePath, toPath: copyDst)
print(fm.fileExists(atPath: filePath))
print(fm.fileExists(atPath: copyDst))

let moveDst = root + "/hello-moved.txt"
try! fm.moveItem(atPath: copyDst, toPath: moveDst)
print(fm.fileExists(atPath: copyDst))
print(fm.fileExists(atPath: moveDst))

// removeItem.
try! fm.removeItem(atPath: moveDst)
print(fm.fileExists(atPath: moveDst))

// A throwing failure is catchable with a message.
do {
    try fm.removeItem(atPath: root + "/does-not-exist.txt")
} catch let e as CocoaError {
    print("caught: \(e.code > 0 && e.message.isEmpty == false)")
}

// File-URL loading: String(contentsOfFile:), Data(contentsOf:), and the
// matching write(to:)/write(toFile:) helpers.
let urlPath = root + "/via-url.txt"
try! "written via String".write(toFile: urlPath, atomically: true, encoding: .utf8)
print(try! String(contentsOfFile: urlPath))

let fileURL = URL(fileURLWithPath: urlPath)
let urlData = try! Data(contentsOf: fileURL)
print(String(data: urlData, encoding: .utf8)!)

let dataURL = URL(fileURLWithPath: root + "/via-data-url.txt")
try! Data("data write(to:)".utf8).write(to: dataURL)
print(try! String(contentsOf: dataURL))

// Host-provided portable directories map into the same sandbox and persist
// for this interpreter run.
let documentsFile = URL.documentsDirectory.appendingPathComponent("session.txt")
try! "documents persist".write(to: documentsFile, atomically: true, encoding: .utf8)
print(try! String(contentsOf: documentsFile))

// Clean up.
try! fm.removeItem(atPath: root)
print(fm.fileExists(atPath: root))
