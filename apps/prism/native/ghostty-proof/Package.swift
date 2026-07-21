// swift-tools-version: 5.9
import PackageDescription

let package = Package(
  name: "PrismGhosttyProof",
  platforms: [.macOS(.v13)],
  products: [
    .executable(name: "PrismGhosttyProof", targets: ["PrismGhosttyProof"])
  ],
  targets: [
    // The build script stages this package under target/ and places the
    // verified, generated XCFramework at this relative path.
    .binaryTarget(
      name: "GhosttyKit",
      path: "Artifacts/GhosttyKit.xcframework"
    ),
    .executableTarget(
      name: "PrismGhosttyProof",
      dependencies: ["GhosttyKit"],
      path: "Sources/PrismGhosttyProof",
      linkerSettings: [
        .linkedFramework("AppKit"),
        .linkedFramework("Carbon"),
        .linkedFramework("CoreGraphics"),
        .linkedFramework("CoreText"),
        .linkedFramework("CoreVideo"),
        .linkedFramework("IOSurface"),
        .linkedFramework("Metal"),
        .linkedFramework("QuartzCore"),
      ]
    ),
  ]
)
