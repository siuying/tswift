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
            from: "1.17.0"
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
