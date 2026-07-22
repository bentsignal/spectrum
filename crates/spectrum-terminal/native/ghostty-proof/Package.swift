// swift-tools-version: 5.9
import PackageDescription

let package = Package(
  name: "SpectrumGhosttyProof",
  platforms: [.macOS(.v13)],
  products: [
    .executable(name: "SpectrumGhosttyProof", targets: ["SpectrumGhosttyProof"])
  ],
  targets: [
    // The build script stages this package under target/ and places the
    // verified, generated XCFramework at this relative path.
    .binaryTarget(
      name: "GhosttyKit",
      path: "Artifacts/GhosttyKit.xcframework"
    ),
    .executableTarget(
      name: "SpectrumGhosttyProof",
      dependencies: ["GhosttyKit"],
      path: "Sources/SpectrumGhosttyProof",
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
