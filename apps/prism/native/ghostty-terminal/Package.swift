// swift-tools-version: 5.9
import PackageDescription

let package = Package(
  name: "PrismGhosttyBridge",
  platforms: [.macOS(.v13)],
  products: [
    .library(name: "PrismGhosttyBridge", type: .dynamic, targets: ["PrismGhosttyBridge"])
  ],
  targets: [
    // The packaging script stages the pinned, verified XCFramework here. It is
    // never downloaded or rebuilt as part of the ordinary Prism Cargo build.
    .binaryTarget(name: "GhosttyKit", path: "Artifacts/GhosttyKit.xcframework"),
    .target(
      name: "PrismGhosttyBridge",
      dependencies: ["GhosttyKit"],
      path: "Sources/PrismGhosttyBridge",
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
  ]
)
