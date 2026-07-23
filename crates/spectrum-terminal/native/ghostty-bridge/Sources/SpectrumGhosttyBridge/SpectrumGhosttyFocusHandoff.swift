import AppKit

enum SpectrumGhosttyFocusHandoff {
  static func applyVisibility(_ visible: Bool, to surfaceView: NSView, hostView: NSView?) {
    if !visible {
      restoreHostResponderIfOwned(by: surfaceView, hostView: hostView)
    }
    surfaceView.isHidden = !visible
  }

  private static func restoreHostResponderIfOwned(by surfaceView: NSView, hostView: NSView?) {
    guard let window = surfaceView.window, window.firstResponder === surfaceView else { return }
    let restored = hostView.map { window.makeFirstResponder($0) } ?? false
    if !restored {
      window.makeFirstResponder(nil)
    }
  }
}
