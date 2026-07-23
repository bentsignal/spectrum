import AppKit
import XCTest

@testable import SpectrumGhosttyBridge

final class SpectrumGhosttyFocusHandoffTests: XCTestCase {
  @MainActor
  func testRepeatedTerminalTogglesRestoreHostFocusBeforeHiding() {
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 800, height: 600),
      styleMask: [.titled],
      backing: .buffered,
      defer: false
    )
    let host = FocusableView(frame: window.contentView?.bounds ?? .zero)
    let terminal = FocusableView(frame: host.bounds)
    window.contentView = host
    host.addSubview(terminal)

    for _ in 0..<4 {
      SpectrumGhosttyFocusHandoff.applyVisibility(true, to: terminal, hostView: host)
      XCTAssertFalse(terminal.isHidden)
      XCTAssertTrue(window.makeFirstResponder(terminal))
      XCTAssertTrue(window.firstResponder === terminal)

      SpectrumGhosttyFocusHandoff.applyVisibility(false, to: terminal, hostView: host)
      XCTAssertTrue(terminal.isHidden)
      XCTAssertTrue(window.firstResponder === host)
    }
  }

  @MainActor
  func testHidingAnUnfocusedTerminalPreservesTheCurrentResponder() {
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 800, height: 600),
      styleMask: [.titled],
      backing: .buffered,
      defer: false
    )
    let host = FocusableView(frame: window.contentView?.bounds ?? .zero)
    let terminal = FocusableView(frame: host.bounds)
    let editor = FocusableView(frame: host.bounds)
    window.contentView = host
    host.addSubview(terminal)
    host.addSubview(editor)
    XCTAssertTrue(window.makeFirstResponder(editor))

    SpectrumGhosttyFocusHandoff.applyVisibility(false, to: terminal, hostView: host)

    XCTAssertTrue(terminal.isHidden)
    XCTAssertTrue(window.firstResponder === editor)
  }
}

private final class FocusableView: NSView {
  override var acceptsFirstResponder: Bool { true }
}
