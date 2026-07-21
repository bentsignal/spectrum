#!/usr/bin/env swift

import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

private let outputSize = 1_024
private let superellipseExponent = 5.0
private let pathSamples = 512

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("error: \(message)\n".utf8))
    exit(1)
}

guard CommandLine.arguments.count == 3 else {
    fail("usage: render-squircle-app-icon.swift <source.png> <destination.png>")
}

let sourceURL = URL(fileURLWithPath: CommandLine.arguments[1])
let destinationURL = URL(fileURLWithPath: CommandLine.arguments[2])

guard
    let source = CGImageSourceCreateWithURL(sourceURL as CFURL, nil),
    let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
else {
    fail("could not decode \(sourceURL.path)")
}

let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()
guard let context = CGContext(
    data: nil,
    width: outputSize,
    height: outputSize,
    bitsPerComponent: 8,
    bytesPerRow: outputSize * 4,
    space: colorSpace,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
) else {
    fail("could not create the output bitmap")
}

let center = Double(outputSize) / 2.0
let radius = center
let power = 2.0 / superellipseExponent
let path = CGMutablePath()

for sample in 0...pathSamples {
    let angle = Double(sample) * 2.0 * Double.pi / Double(pathSamples)
    let cosine = cos(angle)
    let sine = sin(angle)
    let x = center + radius * copysign(pow(abs(cosine), power), cosine)
    let y = center + radius * copysign(pow(abs(sine), power), sine)
    let point = CGPoint(x: x, y: y)
    if sample == 0 {
        path.move(to: point)
    } else {
        path.addLine(to: point)
    }
}
path.closeSubpath()

context.setShouldAntialias(true)
context.setAllowsAntialiasing(true)
context.interpolationQuality = .high
context.addPath(path)
context.clip()
context.draw(image, in: CGRect(x: 0, y: 0, width: outputSize, height: outputSize))

guard let output = context.makeImage() else {
    fail("could not render the output image")
}

try? FileManager.default.createDirectory(
    at: destinationURL.deletingLastPathComponent(),
    withIntermediateDirectories: true
)
guard let destination = CGImageDestinationCreateWithURL(
    destinationURL as CFURL,
    UTType.png.identifier as CFString,
    1,
    nil
) else {
    fail("could not create \(destinationURL.path)")
}

CGImageDestinationAddImage(destination, output, nil)
guard CGImageDestinationFinalize(destination) else {
    fail("could not write \(destinationURL.path)")
}
