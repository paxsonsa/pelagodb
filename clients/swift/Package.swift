// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "PelagoDBClient",
    platforms: [.macOS(.v13), .iOS(.v16)],
    products: [
        .library(name: "PelagoDBClient", targets: ["PelagoDBClient"])
    ],
    dependencies: [
        .package(url: "https://github.com/apple/swift-protobuf.git", from: "1.28.0"),
        .package(url: "https://github.com/grpc/grpc-swift.git", from: "1.24.0")
    ],
    targets: [
        .target(
            name: "PelagoDBClient",
            dependencies: [
                .product(name: "SwiftProtobuf", package: "swift-protobuf"),
                .product(name: "GRPC", package: "grpc-swift"),
                .product(name: "NIO", package: "grpc-swift")
            ]
        )
    ]
)
