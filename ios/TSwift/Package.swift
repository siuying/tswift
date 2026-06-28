// swift-tools-version: 6.0
import Foundation
import PackageDescription

// The native FFI binary is consumed in one of two ways (ADR-0008):
//   * a locally built, git-ignored xcframework at Artifacts/ wins when present
//     (fast local iteration — run scripts/build-xcframework.sh); otherwise
//   * the pinned GitHub Release asset described by ffi.pin is downloaded.
let packageRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let localXcframework = packageRoot
    .appendingPathComponent("Artifacts/TSwiftFFI.xcframework")

struct FFIPin: Decodable { let version, url, checksum: String }

func pinnedBinaryTarget() -> Target {
    let pinURL = packageRoot.appendingPathComponent("ffi.pin")
    guard let data = try? Data(contentsOf: pinURL),
          let pin = try? JSONDecoder().decode(FFIPin.self, from: data)
    else {
        fatalError("""
        No local xcframework at Artifacts/ and ffi.pin is missing or invalid.
        Run scripts/build-xcframework.sh, or restore a valid ffi.pin.
        """)
    }
    // A placeholder pin (all-zero checksum) means no release has been published
    // yet (scripts/publish-xcframework.sh fills it). Fail clearly rather than
    // handing SwiftPM a checksum that will never match.
    guard !pin.checksum.allSatisfy({ $0 == "0" }) else {
        fatalError("""
        ffi.pin is a placeholder — no TSwiftFFI release is published yet.
        Build the framework locally with scripts/build-xcframework.sh.
        """)
    }
    return .binaryTarget(name: "TSwiftFFI", url: pin.url, checksum: pin.checksum)
}

let ffiTarget: Target = FileManager.default.fileExists(atPath: localXcframework.path)
    ? .binaryTarget(name: "TSwiftFFI", path: "Artifacts/TSwiftFFI.xcframework")
    : pinnedBinaryTarget()

let package = Package(
    name: "TSwift",
    platforms: [.iOS(.v16), .macOS(.v13)],
    products: [
        .library(name: "TSwiftCore", targets: ["TSwiftCore"]),
        .library(name: "TSwiftUI", targets: ["TSwiftUI"]),
    ],
    dependencies: [
        .package(path: "../UiirRenderer"),
    ],
    targets: [
        ffiTarget,
        .target(name: "TSwiftCore", dependencies: ["TSwiftFFI"]),
        .testTarget(name: "TSwiftCoreTests", dependencies: ["TSwiftCore"]),
        .target(
            name: "TSwiftUI",
            dependencies: [
                "TSwiftCore",
                "TSwiftFFI",
                .product(name: "UiirRenderer", package: "UiirRenderer"),
            ]
        ),
        .testTarget(name: "TSwiftUITests", dependencies: ["TSwiftUI"]),
    ]
)
