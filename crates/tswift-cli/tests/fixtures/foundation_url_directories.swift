import Foundation

// Well-known filesystem-location statics resolve to file:// directory URLs.
// $HOME / temp dir vary by environment, so assert on stable structure
// (scheme/isFileURL/hasDirectoryPath and the fixed trailing path components).
print(URL.temporaryDirectory.isFileURL, URL.temporaryDirectory.hasDirectoryPath)
print(URL.homeDirectory.isFileURL, URL.homeDirectory.hasDirectoryPath)
print(URL.documentsDirectory.lastPathComponent)
print(URL.cachesDirectory.lastPathComponent)
print(URL.applicationSupportDirectory.lastPathComponent)
print(URL.libraryDirectory.lastPathComponent)
print(URL.desktopDirectory.lastPathComponent)
print(URL.downloadsDirectory.lastPathComponent)
print(URL.moviesDirectory.lastPathComponent)
print(URL.musicDirectory.lastPathComponent)
print(URL.picturesDirectory.lastPathComponent)
print(URL.sharedPublicDirectory.lastPathComponent)
print(URL.trashDirectory.lastPathComponent)
print(URL.applicationDirectory.absoluteString)
print(URL.userDirectory.absoluteString)
print(URL.currentDirectory().isFileURL, URL.currentDirectory().hasDirectoryPath)

// dataRepresentation is the UTF-8 encoding of the absolute string.
let u = URL(string: "https://a.com/x")!
print(u.dataRepresentation.count)

// standardizedFileURL resolves `.` / `..` lexically.
print(URL(fileURLWithPath: "/a/b/../c").standardizedFileURL.path)
