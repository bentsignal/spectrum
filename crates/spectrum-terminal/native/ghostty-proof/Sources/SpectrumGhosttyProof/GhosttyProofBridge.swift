import AppKit
import Dispatch
import GhosttyKit

@_cdecl("spectrum_ghostty_global_init")
public func spectrumGhosttyGlobalInit(
  _ argc: Int32,
  _ argv: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
  Int32(ghostty_init(UInt(argc), argv))
}

@_cdecl("spectrum_ghostty_runtime_create")
public func spectrumGhosttyRuntimeCreate() -> UnsafeMutableRawPointer? {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let runtime = try? GhosttyProofRuntime() else { return nil }
  return Unmanaged.passRetained(runtime).toOpaque()
}

@_cdecl("spectrum_ghostty_runtime_tick")
public func spectrumGhosttyRuntimeTick(_ raw: UnsafeMutableRawPointer?) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  Unmanaged<GhosttyProofRuntime>.fromOpaque(raw).takeUnretainedValue().tick()
}

@_cdecl("spectrum_ghostty_runtime_set_focus")
public func spectrumGhosttyRuntimeSetFocus(_ raw: UnsafeMutableRawPointer?, _ focused: Bool) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  Unmanaged<GhosttyProofRuntime>.fromOpaque(raw).takeUnretainedValue().setFocused(focused)
}

@_cdecl("spectrum_ghostty_runtime_destroy")
public func spectrumGhosttyRuntimeDestroy(_ raw: UnsafeMutableRawPointer?) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  let runtime = Unmanaged<GhosttyProofRuntime>.fromOpaque(raw).takeRetainedValue()
  runtime.shutdown()
}

@_cdecl("spectrum_ghostty_surface_create")
public func spectrumGhosttySurfaceCreate(
  _ runtimeRaw: UnsafeMutableRawPointer?,
  _ parentRaw: UnsafeMutableRawPointer?,
  _ workingDirectoryRaw: UnsafePointer<CChar>?
) -> UnsafeMutableRawPointer? {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let runtimeRaw, let parentRaw, let workingDirectoryRaw else { return nil }
  let runtime = Unmanaged<GhosttyProofRuntime>.fromOpaque(runtimeRaw).takeUnretainedValue()
  let parent = Unmanaged<NSView>.fromOpaque(parentRaw).takeUnretainedValue()
  let workingDirectory = String(cString: workingDirectoryRaw)
  guard
    let view = try? GhosttyProofSurfaceView(
      runtime: runtime,
      workingDirectory: workingDirectory,
      frame: parent.bounds
    )
  else {
    return nil
  }
  view.autoresizingMask = [.width, .height]
  parent.addSubview(view)
  parent.window?.makeFirstResponder(view)
  return Unmanaged.passRetained(view).toOpaque()
}

@_cdecl("spectrum_ghostty_surface_set_frame")
public func spectrumGhosttySurfaceSetFrame(
  _ raw: UnsafeMutableRawPointer?,
  _ x: Double,
  _ y: Double,
  _ width: Double,
  _ height: Double
) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  let view = Unmanaged<GhosttyProofSurfaceView>.fromOpaque(raw).takeUnretainedValue()
  view.frame = NSRect(x: x, y: y, width: width, height: height)
}

@_cdecl("spectrum_ghostty_surface_set_focus")
public func spectrumGhosttySurfaceSetFocus(_ raw: UnsafeMutableRawPointer?, _ focused: Bool) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  let view = Unmanaged<GhosttyProofSurfaceView>.fromOpaque(raw).takeUnretainedValue()
  if focused {
    view.window?.makeFirstResponder(view)
  } else if let surface = view.surface {
    ghostty_surface_set_focus(surface, false)
  }
}

@_cdecl("spectrum_ghostty_surface_request_close")
public func spectrumGhosttySurfaceRequestClose(_ raw: UnsafeMutableRawPointer?) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  Unmanaged<GhosttyProofSurfaceView>.fromOpaque(raw).takeUnretainedValue().requestClose()
}

@_cdecl("spectrum_ghostty_surface_destroy")
public func spectrumGhosttySurfaceDestroy(_ raw: UnsafeMutableRawPointer?) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  let view = Unmanaged<GhosttyProofSurfaceView>.fromOpaque(raw).takeRetainedValue()
  view.destroySurface()
  view.removeFromSuperview()
}
