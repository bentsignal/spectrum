import AppKit
import Dispatch
import GhosttyKit

let spectrumGhosttyBridgeABIVersion: UInt32 = 1

public typealias SpectrumGhosttyEventCallback =
  @convention(c) (
    UnsafeMutableRawPointer?, UInt64, UInt32, UnsafePointer<CChar>?, Int, Bool
  ) -> Void

@_cdecl("spectrum_ghostty_bridge_abi_version")
public func spectrumGhosttyBridgeVersion() -> UInt32 {
  spectrumGhosttyBridgeABIVersion
}

@_cdecl("spectrum_ghostty_global_init")
public func spectrumGhosttyGlobalInit() -> Int32 {
  Int32(ghostty_init(UInt(CommandLine.argc), CommandLine.unsafeArgv))
}

@_cdecl("spectrum_ghostty_runtime_create")
public func spectrumGhosttyRuntimeCreate(
  _ callback: SpectrumGhosttyEventCallback,
  _ userdata: UnsafeMutableRawPointer?
) -> UnsafeMutableRawPointer? {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let runtime = try? SpectrumGhosttyRuntime(callback: callback, userdata: userdata) else {
    return nil
  }
  return Unmanaged.passRetained(runtime).toOpaque()
}

@_cdecl("spectrum_ghostty_runtime_tick")
public func spectrumGhosttyRuntimeTick(_ raw: UnsafeMutableRawPointer?) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  Unmanaged<SpectrumGhosttyRuntime>.fromOpaque(raw).takeUnretainedValue().tick()
}

@_cdecl("spectrum_ghostty_runtime_set_focus")
public func spectrumGhosttyRuntimeSetFocus(_ raw: UnsafeMutableRawPointer?, _ focused: Bool) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  Unmanaged<SpectrumGhosttyRuntime>.fromOpaque(raw).takeUnretainedValue().setFocused(focused)
}

@_cdecl("spectrum_ghostty_runtime_destroy")
public func spectrumGhosttyRuntimeDestroy(_ raw: UnsafeMutableRawPointer?) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  let runtime = Unmanaged<SpectrumGhosttyRuntime>.fromOpaque(raw).takeRetainedValue()
  runtime.shutdown()
}

@_cdecl("spectrum_ghostty_surface_create")
public func spectrumGhosttySurfaceCreate(
  _ runtimeRaw: UnsafeMutableRawPointer?,
  _ parentRaw: UnsafeMutableRawPointer?,
  _ sessionID: UInt64,
  _ workingDirectoryRaw: UnsafePointer<CChar>?,
  _ environmentRaw: UnsafePointer<CChar>?
) -> UnsafeMutableRawPointer? {
  dispatchPrecondition(condition: .onQueue(.main))
  guard
    let runtimeRaw,
    let parentRaw,
    let workingDirectoryRaw,
    let environmentRaw
  else {
    return nil
  }
  let runtime = Unmanaged<SpectrumGhosttyRuntime>.fromOpaque(runtimeRaw).takeUnretainedValue()
  let parent = Unmanaged<NSView>.fromOpaque(parentRaw).takeUnretainedValue()
  let environment = decodeEnvironment(String(cString: environmentRaw))
  guard
    let environment,
    let view = try? SpectrumGhosttySurfaceView(
      runtime: runtime,
      sessionID: sessionID,
      workingDirectory: String(cString: workingDirectoryRaw),
      environment: environment,
      parent: parent
    )
  else {
    return nil
  }
  parent.addSubview(view)
  return Unmanaged.passRetained(view).toOpaque()
}

@_cdecl("spectrum_ghostty_surface_set_state")
public func spectrumGhosttySurfaceSetState(
  _ raw: UnsafeMutableRawPointer?,
  _ x: Double,
  _ y: Double,
  _ width: Double,
  _ height: Double,
  _ visible: Bool,
  _ requestFocus: Bool
) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  Unmanaged<SpectrumGhosttySurfaceView>.fromOpaque(raw).takeUnretainedValue().setPresentation(
    x: x,
    top: y,
    width: width,
    height: height,
    visible: visible,
    requestFocus: requestFocus
  )
}

@_cdecl("spectrum_ghostty_surface_edit")
public func spectrumGhosttySurfaceEdit(
  _ raw: UnsafeMutableRawPointer?,
  _ action: UInt32
) -> Bool {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return false }
  return Unmanaged<SpectrumGhosttySurfaceView>.fromOpaque(raw).takeUnretainedValue()
    .performEditAction(action)
}

@_cdecl("spectrum_ghostty_surface_request_close")
public func spectrumGhosttySurfaceRequestClose(_ raw: UnsafeMutableRawPointer?) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  Unmanaged<SpectrumGhosttySurfaceView>.fromOpaque(raw).takeUnretainedValue().requestClose()
}

@_cdecl("spectrum_ghostty_surface_destroy")
public func spectrumGhosttySurfaceDestroy(_ raw: UnsafeMutableRawPointer?) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  let view = Unmanaged<SpectrumGhosttySurfaceView>.fromOpaque(raw).takeRetainedValue()
  view.destroySurface()
  view.removeFromSuperview()
}

private func decodeEnvironment(_ json: String) -> [String: String]? {
  guard let data = json.data(using: .utf8) else { return nil }
  return try? JSONDecoder().decode([String: String].self, from: data)
}
