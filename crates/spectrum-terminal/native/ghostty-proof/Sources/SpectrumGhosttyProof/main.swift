import AppKit
import GhosttyKit

final class GhosttyProofAppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
  private var runtime: GhosttyProofRuntime?
  private var surfaceView: GhosttyProofSurfaceView?
  private var window: NSWindow?

  func applicationDidFinishLaunching(_ notification: Notification) {
    _ = notification
    do {
      let runtime = try GhosttyProofRuntime()
      let surfaceView = try GhosttyProofSurfaceView(
        runtime: runtime,
        workingDirectory: FileManager.default.currentDirectoryPath
      )
      surfaceView.autoresizingMask = [.width, .height]

      let window = NSWindow(
        contentRect: NSRect(x: 0, y: 0, width: 960, height: 600),
        styleMask: [.titled, .closable, .miniaturizable, .resizable],
        backing: .buffered,
        defer: false
      )
      window.title = "Spectrum Ghostty Proof"
      window.minSize = NSSize(width: 520, height: 320)
      window.contentView = surfaceView
      window.delegate = self
      window.center()
      window.makeKeyAndOrderFront(nil)
      window.makeFirstResponder(surfaceView)

      self.runtime = runtime
      self.surfaceView = surfaceView
      self.window = window
      NSApp.activate(ignoringOtherApps: true)
    } catch {
      let alert = NSAlert(error: error)
      alert.messageText = "Ghostty proof initialization failed"
      alert.runModal()
      NSApp.terminate(nil)
    }
  }

  func applicationDidBecomeActive(_ notification: Notification) {
    _ = notification
    runtime?.setFocused(true)
  }

  func applicationDidResignActive(_ notification: Notification) {
    _ = notification
    runtime?.setFocused(false)
  }

  func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    _ = sender
    return true
  }

  func windowShouldClose(_ sender: NSWindow) -> Bool {
    _ = sender
    guard let surfaceView, !surfaceView.shouldAllowWindowClose() else { return true }
    surfaceView.requestClose()
    return false
  }

  func applicationWillTerminate(_ notification: Notification) {
    _ = notification
    surfaceView?.destroySurface()
    surfaceView = nil
    window = nil
    runtime?.shutdown()
    runtime = nil
  }
}

guard ghostty_init(UInt(CommandLine.argc), CommandLine.unsafeArgv) == GHOSTTY_SUCCESS else {
  fputs("Ghostty global initialization failed. Verify bundled resources.\n", stderr)
  exit(1)
}

let application = NSApplication.shared
application.setActivationPolicy(.regular)
let delegate = GhosttyProofAppDelegate()
application.delegate = delegate
application.run()
