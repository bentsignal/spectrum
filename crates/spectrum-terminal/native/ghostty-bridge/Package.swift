// swift-tools-version: 5.9
import PackageDescription

let package = Package(
  name: "SpectrumGhosttyBridge",
  platforms: [.macOS(.v13)],
  products: [
    .library(name: "SpectrumGhosttyBridge", type: .dynamic, targets: ["SpectrumGhosttyBridge"])
  ],
  targets: [
    // The packaging script stages the pinned, verified XCFramework here. It is
    // never downloaded or rebuilt as part of the ordinary Spectrum Cargo build.
    .binaryTarget(name: "GhosttyKit", path: "Artifacts/GhosttyKit.xcframework"),
    .target(
      name: "SpectrumGhosttyBridge",
      dependencies: ["GhosttyKit"],
      path: "Sources/SpectrumGhosttyBridge",
      linkerSettings: [
        .linkedLibrary("c++"),
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
    .testTarget(
      name: "SpectrumGhosttyBridgeTests",
      dependencies: ["SpectrumGhosttyBridge"],
      path: "Tests/SpectrumGhosttyBridgeTests"
    ),
  ]
)
