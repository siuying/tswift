// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "UiirRenderer",
    platforms: [.iOS(.v16), .macOS(.v13)],
    products: [
        .library(name: "UiirRenderer", targets: ["UiirRenderer"]),
    ],
    dependencies: [
        .package(
            url: "https://github.com/pointfreeco/swift-snapshot-testing",
            // Pinned below 1.19.0: 1.19.x added Swift-Testing `Attachment.record`
            // code that fails to compile under the Swift 6.3 toolchain (Data /
            // image wrappers don't conform to `Attachable` in the shipped 6.3
            // Testing framework). 1.18.x has all APIs these tests use.
            "1.18.0" ..< "1.19.0"
        ),
    ],
    targets: [
        .target(name: "UiirRenderer"),
        .testTarget(
            name: "UiirRendererTests",
            dependencies: [
                "UiirRenderer",
                .product(name: "SnapshotTesting", package: "swift-snapshot-testing"),
            ]
        ),
    ]
)
