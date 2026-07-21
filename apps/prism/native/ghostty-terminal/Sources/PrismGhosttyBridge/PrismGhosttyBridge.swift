import AppKit
import Dispatch
import GhosttyKit

let prismGhosttyBridgeABIVersion: UInt32 = 1

typealias PrismGhosttyEventCallback = @convention(c) (
  UnsafeMutableRawPointer?, UInt64, UInt32, UnsafePointer<CChar>?, Int, Bool
) -> Void

@_cdecl("prism_ghostty_bridge_abi_version")
public func prismGhosttyBridgeVersion() -> UInt32 {
  prismGhosttyBridgeABIVersion
}

@_cdecl("prism_ghostty_global_init")
public func prismGhosttyGlobalInit() -> Int32 {
  Int32(ghostty_init(UInt(CommandLine.argc), CommandLine.unsafeArgv))
}

@_cdecl("prism_ghostty_runtime_create")
public func prismGhosttyRuntimeCreate(
  _ callback: PrismGhosttyEventCallback,
  _ userdata: UnsafeMutableRawPointer?
) -> UnsafeMutableRawPointer? {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let runtime = try? PrismGhosttyRuntime(callback: callback, userdata: userdata) else {
    return nil
  }
  return Unmanaged.passRetained(runtime).toOpaque()
}

@_cdecl("prism_ghostty_runtime_tick")
public func prismGhosttyRuntimeTick(_ raw: UnsafeMutableRawPointer?) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  Unmanaged<PrismGhosttyRuntime>.fromOpaque(raw).takeUnretainedValue().tick()
}

@_cdecl("prism_ghostty_runtime_set_focus")
public func prismGhosttyRuntimeSetFocus(_ raw: UnsafeMutableRawPointer?, _ focused: Bool) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  Unmanaged<PrismGhosttyRuntime>.fromOpaque(raw).takeUnretainedValue().setFocused(focused)
}

@_cdecl("prism_ghostty_runtime_destroy")
public func prismGhosttyRuntimeDestroy(_ raw: UnsafeMutableRawPointer?) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  let runtime = Unmanaged<PrismGhosttyRuntime>.fromOpaque(raw).takeRetainedValue()
  runtime.shutdown()
}

@_cdecl("prism_ghostty_surface_create")
public func prismGhosttySurfaceCreate(
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
  let runtime = Unmanaged<PrismGhosttyRuntime>.fromOpaque(runtimeRaw).takeUnretainedValue()
  let parent = Unmanaged<NSView>.fromOpaque(parentRaw).takeUnretainedValue()
  let environment = decodeEnvironment(String(cString: environmentRaw))
  guard
    let environment,
    let view = try? PrismGhosttySurfaceView(
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

@_cdecl("prism_ghostty_surface_set_state")
public func prismGhosttySurfaceSetState(
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
  Unmanaged<PrismGhosttySurfaceView>.fromOpaque(raw).takeUnretainedValue().setPresentation(
    x: x,
    top: y,
    width: width,
    height: height,
    visible: visible,
    requestFocus: requestFocus
  )
}

@_cdecl("prism_ghostty_surface_edit")
public func prismGhosttySurfaceEdit(
  _ raw: UnsafeMutableRawPointer?,
  _ action: UInt32
) -> Bool {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return false }
  return Unmanaged<PrismGhosttySurfaceView>.fromOpaque(raw).takeUnretainedValue()
    .performEditAction(action)
}

@_cdecl("prism_ghostty_surface_request_close")
public func prismGhosttySurfaceRequestClose(_ raw: UnsafeMutableRawPointer?) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  Unmanaged<PrismGhosttySurfaceView>.fromOpaque(raw).takeUnretainedValue().requestClose()
}

@_cdecl("prism_ghostty_surface_destroy")
public func prismGhosttySurfaceDestroy(_ raw: UnsafeMutableRawPointer?) {
  dispatchPrecondition(condition: .onQueue(.main))
  guard let raw else { return }
  let view = Unmanaged<PrismGhosttySurfaceView>.fromOpaque(raw).takeRetainedValue()
  view.destroySurface()
  view.removeFromSuperview()
}

private func decodeEnvironment(_ json: String) -> [String: String]? {
  guard let data = json.data(using: .utf8) else { return nil }
  return try? JSONDecoder().decode([String: String].self, from: data)
}
