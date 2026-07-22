import AppKit
import GhosttyKit

enum SpectrumGhosttyError: Error {
  case configCreationFailed
  case appCreationFailed
  case surfaceCreationFailed
}

final class SpectrumGhosttyRuntime {
  private(set) var app: ghostty_app_t?
  private let callback: SpectrumGhosttyEventCallback
  private let callbackUserdata: UnsafeMutableRawPointer?
  private var active = true
  private var desiredFocus = false
  private var activationObservers: [NSObjectProtocol] = []

  init(callback: @escaping SpectrumGhosttyEventCallback, userdata: UnsafeMutableRawPointer?) throws
  {
    app = nil
    self.callback = callback
    callbackUserdata = userdata

    guard let config = ghostty_config_new() else {
      throw SpectrumGhosttyError.configCreationFailed
    }
    defer { ghostty_config_free(config) }
    ghostty_config_finalize(config)

    var runtimeConfig = ghostty_runtime_config_s(
      userdata: Unmanaged.passUnretained(self).toOpaque(),
      supports_selection_clipboard: false,
      wakeup_cb: { userdata in SpectrumGhosttyRuntime.wakeup(userdata) },
      action_cb: { app, target, action in
        guard let app else { return false }
        return SpectrumGhosttyRuntime.action(app, target: target, action: action)
      },
      read_clipboard_cb: { userdata, location, state in
        SpectrumGhosttyRuntime.readClipboard(userdata, location: location, state: state)
      },
      confirm_read_clipboard_cb: { userdata, string, state, request in
        SpectrumGhosttyRuntime.confirmReadClipboard(
          userdata, string: string, state: state, request: request
        )
      },
      write_clipboard_cb: { userdata, location, content, count, confirm in
        SpectrumGhosttyRuntime.writeClipboard(
          userdata, location: location, content: content, count: count, confirm: confirm
        )
      },
      close_surface_cb: { userdata, processAlive in
        SpectrumGhosttyRuntime.closeSurface(userdata, processAlive: processAlive)
      }
    )

    guard let created = ghostty_app_new(&runtimeConfig, config) else {
      throw SpectrumGhosttyError.appCreationFailed
    }
    app = created
    desiredFocus = true
    syncFocus()
    for name in [
      NSApplication.didBecomeActiveNotification, NSApplication.didResignActiveNotification,
    ] {
      activationObservers.append(
        NotificationCenter.default.addObserver(forName: name, object: NSApp, queue: .main) {
          [weak self] _ in self?.syncFocus()
        }
      )
    }
  }

  deinit {
    for observer in activationObservers {
      NotificationCenter.default.removeObserver(observer)
    }
    if let app { ghostty_app_free(app) }
  }

  func tick() {
    if active, let app { ghostty_app_tick(app) }
  }

  func setFocused(_ focused: Bool) {
    desiredFocus = focused
    syncFocus()
  }

  func shutdown() {
    active = false
    for observer in activationObservers {
      NotificationCenter.default.removeObserver(observer)
    }
    activationObservers.removeAll()
    guard let app else { return }
    self.app = nil
    ghostty_app_free(app)
  }

  private func syncFocus() {
    if active, let app { ghostty_app_set_focus(app, desiredFocus && NSApp.isActive) }
  }

  func emit(sessionID: UInt64, event: UInt32, text: String? = nil, processAlive: Bool = false) {
    guard active else { return }
    guard let text else {
      callback(callbackUserdata, sessionID, event, nil, 0, processAlive)
      return
    }
    text.withCString { pointer in
      callback(callbackUserdata, sessionID, event, pointer, text.utf8.count, processAlive)
    }
  }

  private static func runtime(from userdata: UnsafeMutableRawPointer?) -> SpectrumGhosttyRuntime? {
    guard let userdata else { return nil }
    return Unmanaged<SpectrumGhosttyRuntime>.fromOpaque(userdata).takeUnretainedValue()
  }

  private static func wakeup(_ userdata: UnsafeMutableRawPointer?) {
    guard let runtime = runtime(from: userdata) else { return }
    onMain { runtime.tick() }
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
    let view = Unmanaged<SpectrumGhosttySurfaceView>.fromOpaque(userdata).takeUnretainedValue()

    switch action.tag {
    case GHOSTTY_ACTION_RENDER:
      ghostty_surface_draw(surface)
      return true
    case GHOSTTY_ACTION_SET_TITLE:
      guard let title = action.action.set_title.title else { return false }
      let value = String(cString: title)
      onMain { view.publishTitle(value) }
      return true
    case GHOSTTY_ACTION_SET_TAB_TITLE:
      guard let title = action.action.set_tab_title.title else { return false }
      let value = String(cString: title)
      onMain { view.publishTitle(value) }
      return true
    case GHOSTTY_ACTION_RING_BELL:
      onMain { NSSound.beep() }
      return true
    case GHOSTTY_ACTION_CLOSE_WINDOW, GHOSTTY_ACTION_CLOSE_TAB, GHOSTTY_ACTION_QUIT:
      onMain { view.requestClose() }
      return true
    default:
      return false
    }
  }

  private static func surface(from userdata: UnsafeMutableRawPointer?) -> ghostty_surface_t? {
    guard let userdata else { return nil }
    return Unmanaged<SpectrumGhosttySurfaceView>.fromOpaque(userdata).takeUnretainedValue().surface
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
    _ = string
    guard let surface = surface(from: userdata) else { return }
    // Confirmation-requiring OSC 52 reads remain denied until Spectrum owns a
    // trusted approval sheet outside the native child view.
    ghostty_surface_complete_clipboard_request(surface, "", state, true)
  }

  private static func writeClipboard(
    _ userdata: UnsafeMutableRawPointer?,
    location: ghostty_clipboard_e,
    content: UnsafePointer<ghostty_clipboard_content_s>?,
    count: Int,
    confirm: Bool
  ) {
    _ = userdata
    guard location == GHOSTTY_CLIPBOARD_STANDARD, !confirm, let content else { return }
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

  private static func closeSurface(_ userdata: UnsafeMutableRawPointer?, processAlive: Bool) {
    guard let userdata else { return }
    let view = Unmanaged<SpectrumGhosttySurfaceView>.fromOpaque(userdata).takeUnretainedValue()
    onMain { view.publishClosed(processAlive: processAlive) }
  }

  private static func onMain(_ action: @escaping () -> Void) {
    if Thread.isMainThread {
      action()
    } else {
      DispatchQueue.main.async(execute: action)
    }
  }
}
