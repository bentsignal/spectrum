import AppKit
import GhosttyKit
import QuartzCore

final class SpectrumGhosttySurfaceView: NSView {
  let runtime: SpectrumGhosttyRuntime
  let sessionID: UInt64
  private(set) var surface: ghostty_surface_t?
  private weak var parentView: NSView?
  private var desiredVisible = false
  private var windowObservers: [NSObjectProtocol] = []

  override var acceptsFirstResponder: Bool { true }

  init(
    runtime: SpectrumGhosttyRuntime,
    sessionID: UInt64,
    workingDirectory: String,
    environment: [String: String],
    parent: NSView
  ) throws {
    self.runtime = runtime
    self.sessionID = sessionID
    parentView = parent
    surface = nil
    super.init(frame: .zero)
    isHidden = true
    wantsLayer = true

    guard let app = runtime.app else { throw SpectrumGhosttyError.appCreationFailed }
    var config = ghostty_surface_config_new()
    config.userdata = Unmanaged.passUnretained(self).toOpaque()
    config.platform_tag = GHOSTTY_PLATFORM_MACOS
    config.platform = ghostty_platform_u(
      macos: ghostty_platform_macos_s(nsview: Unmanaged.passUnretained(self).toOpaque())
    )
    config.scale_factor = Double(
      parent.window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 1)
    config.context = GHOSTTY_SURFACE_CONTEXT_TAB

    let strings = environment.sorted(by: { $0.key < $1.key }).flatMap { [$0.key, $0.value] }
      .map { value in value.withCString { strdup($0) } }
    defer {
      for string in strings {
        free(string)
      }
    }
    var variables = stride(from: 0, to: strings.count, by: 2).map { index in
      ghostty_env_var_s(key: strings[index], value: strings[index + 1])
    }
    let created = workingDirectory.withCString { directory in
      config.working_directory = directory
      return variables.withUnsafeMutableBufferPointer { buffer in
        config.env_vars = buffer.baseAddress
        config.env_var_count = buffer.count
        return ghostty_surface_new(app, &config)
      }
    }
    guard let created else { throw SpectrumGhosttyError.surfaceCreationFailed }
    surface = created
    ghostty_surface_set_occlusion(created, false)
  }

  required init?(coder: NSCoder) {
    fatalError("init(coder:) is not supported")
  }

  deinit {
    restoreParentResponderIfOwned()
    stopObservingWindow()
    if let surface {
      ghostty_surface_set_focus(surface, false)
      ghostty_surface_set_occlusion(surface, false)
      ghostty_surface_free(surface)
    }
  }

  func destroySurface() {
    guard let surface else { return }
    restoreParentResponderIfOwned()
    self.surface = nil
    stopObservingWindow()
    ghostty_surface_set_focus(surface, false)
    ghostty_surface_set_occlusion(surface, false)
    ghostty_surface_free(surface)
  }

  func setPresentation(
    x: Double,
    top: Double,
    width: Double,
    height: Double,
    visible: Bool,
    requestFocus: Bool
  ) {
    guard let parent = parentView else { return }
    let y = parent.isFlipped ? top : Double(parent.bounds.height) - top - height
    frame = NSRect(x: x, y: y, width: max(0, width), height: max(0, height))
    desiredVisible = visible
    isHidden = !visible
    if !visible {
      restoreParentResponderIfOwned()
    }
    syncSurfaceGeometry()
    syncOcclusion()
    if visible && requestFocus {
      window?.makeFirstResponder(self)
    }
    syncFocus()
  }

  override func viewDidMoveToWindow() {
    super.viewDidMoveToWindow()
    stopObservingWindow()
    guard let window else {
      syncOcclusion()
      return
    }
    for name in [
      NSWindow.didChangeOcclusionStateNotification,
      NSWindow.didBecomeKeyNotification,
      NSWindow.didResignKeyNotification,
    ] {
      windowObservers.append(
        NotificationCenter.default.addObserver(forName: name, object: window, queue: .main) {
          [weak self] _ in
          self?.syncOcclusion()
          self?.syncFocus()
        }
      )
    }
    syncSurfaceGeometry()
    syncOcclusion()
    syncFocus()
    if let surface { ghostty_surface_refresh(surface) }
  }

  override func viewDidChangeBackingProperties() {
    super.viewDidChangeBackingProperties()
    syncSurfaceGeometry()
  }

  override func becomeFirstResponder() -> Bool {
    let accepted = super.becomeFirstResponder()
    if accepted, desiredVisible, let surface { ghostty_surface_set_focus(surface, true) }
    return accepted
  }

  override func resignFirstResponder() -> Bool {
    let resigned = super.resignFirstResponder()
    if resigned, let surface { ghostty_surface_set_focus(surface, false) }
    return resigned
  }

  override func mouseDown(with event: NSEvent) {
    guard desiredVisible, !isHidden else { return }
    window?.makeFirstResponder(self)
    super.mouseDown(with: event)
  }

  override func keyDown(with event: NSEvent) {
    guard ownsPresentedFocus else { return }
    sendKey(event, action: event.isARepeat ? GHOSTTY_ACTION_REPEAT : GHOSTTY_ACTION_PRESS)
  }

  override func keyUp(with event: NSEvent) {
    guard ownsPresentedFocus else { return }
    sendKey(event, action: GHOSTTY_ACTION_RELEASE)
  }

  func publishTitle(_ title: String) {
    runtime.emit(sessionID: sessionID, event: 1, text: title)
  }

  func publishClosed(processAlive: Bool) {
    runtime.emit(sessionID: sessionID, event: 2, processAlive: processAlive)
  }

  func requestClose() {
    if let surface { ghostty_surface_request_close(surface) }
  }

  func performEditAction(_ action: UInt32) -> Bool {
    guard ownsPresentedFocus, let surface else { return false }
    let binding: String
    switch action {
    case 1: binding = "copy_to_clipboard"
    case 2: binding = "paste_from_clipboard"
    default: return false
    }
    return binding.withCString { pointer in
      ghostty_surface_binding_action(surface, pointer, UInt(binding.utf8.count))
    }
  }

  private func stopObservingWindow() {
    for observer in windowObservers {
      NotificationCenter.default.removeObserver(observer)
    }
    windowObservers.removeAll()
  }

  private func restoreParentResponderIfOwned() {
    guard let window, window.firstResponder === self else { return }
    let restored = parentView.map { window.makeFirstResponder($0) } ?? false
    if !restored {
      window.makeFirstResponder(nil)
    }
  }

  private func syncOcclusion() {
    guard let surface else { return }
    let windowVisible = window?.occlusionState.contains(.visible) == true
    ghostty_surface_set_occlusion(surface, desiredVisible && windowVisible && !isHidden)
  }

  private func syncFocus() {
    guard let surface else { return }
    ghostty_surface_set_focus(surface, ownsPresentedFocus)
  }

  private var ownsPresentedFocus: Bool {
    desiredVisible && !isHidden && NSApp.isActive
      && window?.isKeyWindow == true && window?.firstResponder === self
  }

  private func syncSurfaceGeometry() {
    guard let surface, bounds.width >= 1, bounds.height >= 1 else { return }
    let scale = window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 1
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    layer?.contentsScale = scale
    CATransaction.commit()
    ghostty_surface_set_content_scale(surface, scale, scale)
    let backingSize = convertToBacking(bounds).size
    ghostty_surface_set_size(
      surface,
      UInt32(max(1, backingSize.width.rounded())),
      UInt32(max(1, backingSize.height.rounded()))
    )
  }

  private func sendKey(_ event: NSEvent, action: ghostty_input_action_e) {
    guard let surface else { return }
    var key = ghostty_input_key_s()
    key.action = action
    key.mods = Self.ghosttyModifiers(event.modifierFlags)
    key.consumed_mods = Self.ghosttyModifiers(
      event.modifierFlags.subtracting([.command, .control]))
    key.keycode = UInt32(event.keyCode)
    key.composing = false
    if let scalar = event.characters(byApplyingModifiers: [])?.unicodeScalars.first {
      key.unshifted_codepoint = scalar.value
    }
    let text = event.characters
    let onlyScalar = text?.unicodeScalars.count == 1 ? text?.unicodeScalars.first : nil
    let isFunctionKey = onlyScalar.map { (0xF700...0xF8FF).contains($0.value) } ?? false
    if let text, let first = text.utf8.first, first >= 0x20, !isFunctionKey {
      _ = text.withCString { pointer in
        key.text = pointer
        return ghostty_surface_key(surface, key)
      }
    } else {
      _ = ghostty_surface_key(surface, key)
    }
  }

  private static func ghosttyModifiers(_ flags: NSEvent.ModifierFlags) -> ghostty_input_mods_e {
    var raw = GHOSTTY_MODS_NONE.rawValue
    if flags.contains(.shift) { raw |= GHOSTTY_MODS_SHIFT.rawValue }
    if flags.contains(.control) { raw |= GHOSTTY_MODS_CTRL.rawValue }
    if flags.contains(.option) { raw |= GHOSTTY_MODS_ALT.rawValue }
    if flags.contains(.command) { raw |= GHOSTTY_MODS_SUPER.rawValue }
    if flags.contains(.capsLock) { raw |= GHOSTTY_MODS_CAPS.rawValue }
    return ghostty_input_mods_e(raw)
  }
}
