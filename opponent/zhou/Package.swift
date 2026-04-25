// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "gomoku-vcf",
    platforms: [
        .macOS(.v13),
    ],
    products: [
        .library(name: "GomokuVCFCore", targets: ["GomokuVCFCore"]),
        .executable(name: "gomoku-vcf", targets: ["GomokuVCFCLI"]),
    ],
    targets: [
        .target(
            name: "GomokuVCFCore"
        ),
        .executableTarget(
            name: "GomokuVCFCLI",
            dependencies: ["GomokuVCFCore"]
        ),
    ]
)
