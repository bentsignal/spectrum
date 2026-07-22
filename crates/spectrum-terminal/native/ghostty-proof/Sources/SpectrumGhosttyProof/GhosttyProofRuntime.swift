import AppKit
import GhosttyKit

enum GhosttyProofError: Error {
  case globalInitializationFailed
  case configCreationFailed
  case appCreationFailed
  case surfaceCreationFailed
}

final class GhosttyProofRuntime {
  private(set) var app: ghostty_app_t?

  init() throws {
    app = nil

    guard let config = ghostty_config_new() else {
      throw GhosttyProofError.configCreationFailed
    }
    defer { ghostty_config_free(config) }
    ghostty_config_finalize(config)

    var runtimeConfig = ghostty_runtime_config_s(
      userdata: Unmanaged.passUnretained(self).toOpaque(),
      supports_selection_clipboard: false,
      wakeup_cb: { userdata in GhosttyProofRuntime.wakeup(userdata) },
      action_cb: { app, target, action in
        guard let app else { return false }
        return GhosttyProofRuntime.action(app, target: target, action: action)
      },
      read_clipboard_cb: { userdata, location, state in
        GhosttyProofRuntime.readClipboard(userdata, location: location, state: state)
      },
      confirm_read_clipboard_cb: { userdata, string, state, request in
        GhosttyProofRuntime.confirmReadClipboard(
          userdata,
          string: string,
          state: state,
          request: request
        )
      },
      write_clipboard_cb: { userdata, location, content, count, confirm in
        GhosttyProofRuntime.writeClipboard(
          userdata,
          location: location,
          content: content,
          count: count,
          confirm: confirm
        )
      },
      close_surface_cb: { userdata, processAlive in
        GhosttyProofRuntime.closeSurface(userdata, processAlive: processAlive)
      }
    )

    guard let created = ghostty_app_new(&runtimeConfig, config) else {
      throw GhosttyProofError.appCreationFailed
    }
    app = created
    ghostty_app_set_focus(created, NSApp.isActive)
  }

  deinit {
    if let app {
      ghostty_app_free(app)
    }
  }

  func tick() {
    guard let app else { return }
    ghostty_app_tick(app)
  }

  func setFocused(_ focused: Bool) {
    guard let app else { return }
    ghostty_app_set_focus(app, focused)
  }

  func shutdown() {
    guard let app else { return }
    self.app = nil
    ghostty_app_free(app)
  }

  private static func wakeup(_ userdata: UnsafeMutableRawPointer?) {
    guard let userdata else { return }
    let runtime = Unmanaged<GhosttyProofRuntime>.fromOpaque(userdata).takeUnretainedValue()
    DispatchQueue.main.async { runtime.tick() }
  }

  private static func action(
    _ app: ghostty_app_t,
    target: ghostty_target_s,
    action: ghostty_action_s
  ) -> Bool {
    _ = app

    guard target.tag == GHOSTTY_TARGET_SURFACE,
      let surface = target.target.surface,
      let userdata = ghostty_surface_userdata(surface)
    else {
      return false
    }
    let view = Unmanaged<GhosttyProofSurfaceView>.fromOpaque(userdata).takeUnretainedValue()

    switch action.tag {
    case GHOSTTY_ACTION_RENDER:
      ghostty_surface_draw(surface)
      return true
    case GHOSTTY_ACTION_SET_TITLE:
      guard let titlePointer = action.action.set_title.title else { return false }
      let title = String(cString: titlePointer)
      DispatchQueue.main.async { view.window?.title = title }
      return true
    case GHOSTTY_ACTION_RING_BELL:
      DispatchQueue.main.async { NSSound.beep() }
      return true
    case GHOSTTY_ACTION_CLOSE_WINDOW, GHOSTTY_ACTION_QUIT:
      DispatchQueue.main.async { view.requestClose() }
      return true
    default:
      // Production integration must explicitly map URL, secure-input,
      // notification, search, tab, renderer-health, and mouse actions.
      return false
    }
  }

  private static func surface(
    from userdata: UnsafeMutableRawPointer?
  ) -> ghostty_surface_t? {
    guard let userdata else { return nil }
    let view = Unmanaged<GhosttyProofSurfaceView>.fromOpaque(userdata).takeUnretainedValue()
    return view.surface
  }

  private static func readClipboard(
    _ userdata: UnsafeMutableRawPointer?,
    location: ghostty_clipboard_e,
    state: UnsafeMutableRawPointer?
  ) -> Bool {
    guard location == GHOSTTY_CLIPBOARD_STANDARD,
      let surface = surface(from: userdata),
      let value = NSPasteboard.general.string(forType: .string)
    else {
      return false
    }
    value.withCString { pointer in
      ghostty_surface_complete_clipboard_request(surface, pointer, state, false)
    }
    return true
  }

  private static func confirmReadClipboard(
    _ userdata: UnsafeMutableRawPointer?,
    string: UnsafePointer<CChar>?,
    state: UnsafeMutableRawPointer?,
    request: ghostty_clipboard_request_e
  ) {
    _ = request
    guard let surface = surface(from: userdata), let string else { return }
    // The proof denies confirmation-requiring OSC 52 reads. Production
    // Spectrum clients must present a trusted confirmation surface before completing.
    ghostty_surface_complete_clipboard_request(surface, string, state, false)
  }

  private static func writeClipboard(
    _ userdata: UnsafeMutableRawPointer?,
    location: ghostty_clipboard_e,
    content: UnsafePointer<ghostty_clipboard_content_s>?,
    count: Int,
    confirm: Bool
  ) {
    _ = userdata
    guard location == GHOSTTY_CLIPBOARD_STANDARD,
      !confirm,
      let content
    else {
      return
    }
    for index in 0..<count {
      guard let mime = content[index].mime,
        let data = content[index].data,
        String(cString: mime) == "text/plain"
      else {
        continue
      }
      NSPasteboard.general.clearContents()
      NSPasteboard.general.setString(String(cString: data), forType: .string)
      return
    }
  }

  private static func closeSurface(
    _ userdata: UnsafeMutableRawPointer?,
    processAlive: Bool
  ) {
    guard let userdata else { return }
    let view = Unmanaged<GhosttyProofSurfaceView>.fromOpaque(userdata).takeUnretainedValue()
    DispatchQueue.main.async { view.completeCloseRequest(processAlive: processAlive) }
  }
}
