let package = Package(
    name: "Demo",
    targets: [
        .target(name: "Core"),
        .testTarget(name: "CoreTests", dependencies: ["Core"]),
        .testTarget(name: "ExtraTests", dependencies: ["Core"]),
    ]
)
