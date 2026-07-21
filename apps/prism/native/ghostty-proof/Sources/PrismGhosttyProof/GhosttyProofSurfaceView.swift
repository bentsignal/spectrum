import AppKit
import GhosttyKit
import QuartzCore

final class GhosttyProofSurfaceView: NSView {
  private let runtime: GhosttyProofRuntime
  private(set) var surface: ghostty_surface_t?
  private var allowWindowClose = false
  private var occlusionObserver: NSObjectProtocol?

  override var acceptsFirstResponder: Bool { true }

  init(
    runtime: GhosttyProofRuntime,
    workingDirectory: String,
    frame: NSRect = NSRect(x: 0, y: 0, width: 960, height: 600)
  ) throws {
    self.runtime = runtime
    surface = nil
    super.init(frame: frame)

    guard let app = runtime.app else {
      throw GhosttyProofError.appCreationFailed
    }

    var config = ghostty_surface_config_new()
    config.userdata = Unmanaged.passUnretained(self).toOpaque()
    config.platform_tag = GHOSTTY_PLATFORM_MACOS
    config.platform = ghostty_platform_u(
      macos: ghostty_platform_macos_s(
        nsview: Unmanaged.passUnretained(self).toOpaque()
      )
    )
    config.scale_factor = NSScreen.main?.backingScaleFactor ?? 1
    config.context = GHOSTTY_SURFACE_CONTEXT_WINDOW

    let created = workingDirectory.withCString { directory in
      config.working_directory = directory
      return ghostty_surface_new(app, &config)
    }
    guard let created else {
      throw GhosttyProofError.surfaceCreationFailed
    }
    surface = created
    // Despite the C API name, Ghostty 1.3.1 defines this argument as
    // `visible`. The view has not entered a window yet, so rendering starts
    // suspended until AppKit reports that its attached window is visible.
    ghostty_surface_set_occlusion(created, false)
    syncSurfaceGeometry()
  }

  required init?(coder: NSCoder) {
    fatalError("init(coder:) is not supported")
  }

  deinit {
    stopObservingWindowOcclusion()
    if let surface {
      ghostty_surface_set_occlusion(surface, false)
      ghostty_surface_free(surface)
    }
  }

  func destroySurface() {
    guard let surface else { return }
    stopObservingWindowOcclusion()
    self.surface = nil
    ghostty_surface_set_focus(surface, false)
    ghostty_surface_set_occlusion(surface, false)
    ghostty_surface_free(surface)
  }

  override func viewDidMoveToWindow() {
    super.viewDidMoveToWindow()
    stopObservingWindowOcclusion()
    guard let window else {
      setSurfaceVisible(false)
      return
    }

    occlusionObserver = NotificationCenter.default.addObserver(
      forName: NSWindow.didChangeOcclusionStateNotification,
      object: window,
      queue: .main
    ) { [weak self] _ in
      self?.syncWindowOcclusion()
    }
    syncSurfaceGeometry()
    syncWindowOcclusion()
    if let surface {
      ghostty_surface_refresh(surface)
    }
  }

  override func setFrameSize(_ newSize: NSSize) {
    super.setFrameSize(newSize)
    syncSurfaceGeometry()
  }

  override func viewDidChangeBackingProperties() {
    super.viewDidChangeBackingProperties()
    syncSurfaceGeometry()
  }

  private func stopObservingWindowOcclusion() {
    guard let occlusionObserver else { return }
    NotificationCenter.default.removeObserver(occlusionObserver)
    self.occlusionObserver = nil
  }

  private func syncWindowOcclusion() {
    setSurfaceVisible(window?.occlusionState.contains(.visible) == true)
  }

  private func setSurfaceVisible(_ visible: Bool) {
    guard let surface else { return }
    ghostty_surface_set_occlusion(surface, visible)
  }

  private func syncSurfaceGeometry() {
    guard let surface, bounds.width > 0, bounds.height > 0 else { return }
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

  override func becomeFirstResponder() -> Bool {
    let accepted = super.becomeFirstResponder()
    if accepted, let surface {
      ghostty_surface_set_focus(surface, true)
    }
    return accepted
  }

  override func resignFirstResponder() -> Bool {
    let resigned = super.resignFirstResponder()
    if resigned, let surface {
      ghostty_surface_set_focus(surface, false)
    }
    return resigned
  }

  override func mouseDown(with event: NSEvent) {
    window?.makeFirstResponder(self)
    super.mouseDown(with: event)
  }

  override func keyDown(with event: NSEvent) {
    sendKey(event, action: event.isARepeat ? GHOSTTY_ACTION_REPEAT : GHOSTTY_ACTION_PRESS)
  }

  override func keyUp(with event: NSEvent) {
    sendKey(event, action: GHOSTTY_ACTION_RELEASE)
  }

  private func sendKey(_ event: NSEvent, action: ghostty_input_action_e) {
    guard let surface else { return }
    var key = ghostty_input_key_s()
    key.action = action
    key.mods = Self.ghosttyModifiers(event.modifierFlags)
    key.consumed_mods = Self.ghosttyModifiers(
      event.modifierFlags.subtracting([.command, .control])
    )
    key.keycode = UInt32(event.keyCode)
    key.composing = false
    if let scalar = event.characters(byApplyingModifiers: [])?.unicodeScalars.first {
      key.unshifted_codepoint = scalar.value
    }

    let text = event.characters
    let onlyScalar = text?.unicodeScalars.count == 1 ? text?.unicodeScalars.first : nil
    let isFunctionKey = onlyScalar.map { (0xF700...0xF8FF).contains($0.value) } ?? false
    if let text,
      let first = text.utf8.first,
      first >= 0x20,
      !isFunctionKey
    {
      _ = text.withCString { pointer in
        key.text = pointer
        return ghostty_surface_key(surface, key)
      }
    } else {
      _ = ghostty_surface_key(surface, key)
    }
  }

  private static func ghosttyModifiers(
    _ flags: NSEvent.ModifierFlags
  ) -> ghostty_input_mods_e {
    var raw = GHOSTTY_MODS_NONE.rawValue
    if flags.contains(.shift) { raw |= GHOSTTY_MODS_SHIFT.rawValue }
    if flags.contains(.control) { raw |= GHOSTTY_MODS_CTRL.rawValue }
    if flags.contains(.option) { raw |= GHOSTTY_MODS_ALT.rawValue }
    if flags.contains(.command) { raw |= GHOSTTY_MODS_SUPER.rawValue }
    if flags.contains(.capsLock) { raw |= GHOSTTY_MODS_CAPS.rawValue }
    return ghostty_input_mods_e(raw)
  }

  func requestClose() {
    guard let surface else {
      allowWindowClose = true
      window?.performClose(nil)
      return
    }
    ghostty_surface_request_close(surface)
  }

  func completeCloseRequest(processAlive: Bool) {
    if processAlive {
      let alert = NSAlert()
      alert.messageText = "Stop the proof terminal?"
      alert.informativeText = "The shell or its foreground process is still running."
      alert.addButton(withTitle: "Stop and Close")
      alert.addButton(withTitle: "Cancel")
      guard alert.runModal() == .alertFirstButtonReturn else { return }
    }
    destroySurface()
    allowWindowClose = true
    window?.performClose(nil)
  }

  func shouldAllowWindowClose() -> Bool {
    allowWindowClose || surface == nil
  }
}
