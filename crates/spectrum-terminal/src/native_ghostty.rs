#[cfg(any(test, all(target_os = "macos", feature = "ghostty-terminal")))]
use eframe::egui;

#[cfg(any(test, all(target_os = "macos", feature = "ghostty-terminal")))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeSurfacePresentation {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    visible: bool,
    focus_requested: bool,
}

#[cfg(any(test, all(target_os = "macos", feature = "ghostty-terminal")))]
impl NativeSurfacePresentation {
    pub fn for_terminal(
        rect: egui::Rect,
        terminal_visible: bool,
        active_session: bool,
        modal_open: bool,
        focus_requested: bool,
    ) -> Self {
        let width = rect.width().max(0.0) as f64;
        let height = rect.height().max(0.0) as f64;
        let visible =
            terminal_visible && active_session && !modal_open && width >= 1.0 && height >= 1.0;
        Self {
            x: rect.left() as f64,
            y: rect.top() as f64,
            width,
            height,
            visible,
            focus_requested: visible && focus_requested,
        }
    }

    #[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
    fn hidden() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            visible: false,
            focus_requested: false,
        }
    }
}

#[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
mod macos {
    use std::{
        collections::{BTreeMap, BTreeSet},
        ffi::{CStr, CString, c_char, c_int, c_void},
        path::{Path, PathBuf},
        ptr::NonNull,
        sync::mpsc::{self, Receiver, Sender},
    };

    use crate::TerminalContext;
    use anyhow::{Context, Result, bail};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::NativeSurfacePresentation;

    const BRIDGE_ABI_VERSION: u32 = 1;
    const BRIDGE_FILENAME: &str = "libSpectrumGhosttyBridge.dylib";
    const EVENT_TITLE: u32 = 1;
    const EVENT_CLOSED: u32 = 2;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[repr(u32)]
    pub enum NativeEditAction {
        Copy = 1,
        Paste = 2,
    }

    type RuntimeHandle = *mut c_void;
    type SurfaceHandle = *mut c_void;
    type EventCallback = unsafe extern "C" fn(*mut c_void, u64, u32, *const c_char, usize, bool);

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum NativeTerminalEvent {
        Title {
            session_id: u64,
            title: String,
        },
        Closed {
            session_id: u64,
            process_alive: bool,
        },
    }

    #[derive(Clone, Copy)]
    struct BridgeApi {
        global_init: unsafe extern "C" fn() -> c_int,
        runtime_create: unsafe extern "C" fn(EventCallback, *mut c_void) -> RuntimeHandle,
        runtime_tick: unsafe extern "C" fn(RuntimeHandle),
        runtime_set_focus: unsafe extern "C" fn(RuntimeHandle, bool),
        runtime_destroy: unsafe extern "C" fn(RuntimeHandle),
        surface_create: unsafe extern "C" fn(
            RuntimeHandle,
            *mut c_void,
            u64,
            *const c_char,
            *const c_char,
        ) -> SurfaceHandle,
        surface_set_state: unsafe extern "C" fn(SurfaceHandle, f64, f64, f64, f64, bool, bool),
        surface_edit: unsafe extern "C" fn(SurfaceHandle, u32) -> bool,
        surface_request_close: unsafe extern "C" fn(SurfaceHandle),
        surface_destroy: unsafe extern "C" fn(SurfaceHandle),
    }

    trait LifecycleBridge {
        type Runtime: Copy;
        type Surface: Copy;

        fn destroy_surface(&self, surface: Self::Surface);
        fn destroy_runtime(&self, runtime: Self::Runtime);
    }

    impl LifecycleBridge for BridgeApi {
        type Runtime = RuntimeHandle;
        type Surface = SurfaceHandle;

        fn destroy_surface(&self, surface: Self::Surface) {
            // Safety: handles originate from this loaded bridge and remain live
            // until they are removed exactly once from OwnedNativeObjects.
            unsafe { (self.surface_destroy)(surface) };
        }

        fn destroy_runtime(&self, runtime: Self::Runtime) {
            // Safety: every surface is drained before the runtime handle.
            unsafe { (self.runtime_destroy)(runtime) };
        }
    }

    struct OwnedNativeObjects<B: LifecycleBridge> {
        bridge: B,
        runtime: Option<B::Runtime>,
        surfaces: BTreeMap<u64, B::Surface>,
    }

    impl<B: LifecycleBridge> OwnedNativeObjects<B> {
        fn new(bridge: B, runtime: B::Runtime) -> Self {
            Self {
                bridge,
                runtime: Some(runtime),
                surfaces: BTreeMap::new(),
            }
        }

        fn remove_surface(&mut self, session_id: u64) {
            if let Some(surface) = self.surfaces.remove(&session_id) {
                self.bridge.destroy_surface(surface);
            }
        }

        fn shutdown(&mut self) {
            let surfaces = std::mem::take(&mut self.surfaces);
            for (_, surface) in surfaces {
                self.bridge.destroy_surface(surface);
            }
            if let Some(runtime) = self.runtime.take() {
                self.bridge.destroy_runtime(runtime);
            }
        }
    }

    impl<B: LifecycleBridge> Drop for OwnedNativeObjects<B> {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    // Intentionally has no Drop implementation. The bridge is opened with
    // RTLD_NODELETE and its reference stays process-resident so a delayed
    // main-queue Swift release can never execute unmapped code.
    struct DynamicLibrary(NonNull<c_void>);

    impl DynamicLibrary {
        fn open(path: &Path) -> Result<Self> {
            let path = CString::new(path.as_os_str().as_encoded_bytes())
                .context("Ghostty bridge path contains an interior NUL")?;
            // Safety: the path is a live NUL-terminated string and RTLD_NOW asks
            // dyld to resolve the complete bridge before Spectrum uses any symbol.
            let handle = unsafe { dlopen(path.as_ptr(), RTLD_NOW | RTLD_LOCAL | RTLD_NODELETE) };
            NonNull::new(handle)
                .map(Self)
                .ok_or_else(|| anyhow::anyhow!("could not load Ghostty bridge: {}", dlerror_text()))
        }

        fn symbol(&self, name: &'static CStr) -> Result<*mut c_void> {
            // Clear an old loader error, then distinguish a missing symbol from
            // a valid symbol whose representation happens to be null.
            unsafe {
                let _ = dlerror();
                let symbol = dlsym(self.0.as_ptr(), name.as_ptr());
                let error = dlerror();
                if !error.is_null() {
                    bail!(
                        "Ghostty bridge symbol {} is unavailable: {}",
                        name.to_string_lossy(),
                        CStr::from_ptr(error).to_string_lossy()
                    );
                }
                if symbol.is_null() {
                    bail!(
                        "Ghostty bridge symbol {} resolved to null",
                        name.to_string_lossy()
                    );
                }
                Ok(symbol)
            }
        }
    }

    trait SymbolSource {
        fn symbol(&self, name: &'static CStr) -> Result<*mut c_void>;
    }

    impl SymbolSource for DynamicLibrary {
        fn symbol(&self, name: &'static CStr) -> Result<*mut c_void> {
            DynamicLibrary::symbol(self, name)
        }
    }

    trait LibraryLoader {
        type Library: SymbolSource;

        fn open(&self, path: &Path) -> Result<Self::Library>;
    }

    struct SystemLibraryLoader;

    impl LibraryLoader for SystemLibraryLoader {
        type Library = DynamicLibrary;

        fn open(&self, path: &Path) -> Result<Self::Library> {
            DynamicLibrary::open(path)
        }
    }

    struct CallbackContext {
        sender: Sender<NativeTerminalEvent>,
    }

    struct ReadyHost {
        objects: OwnedNativeObjects<BridgeApi>,
        callback: Box<CallbackContext>,
        receiver: Receiver<NativeTerminalEvent>,
        parent_nsview: NonNull<c_void>,
        library: DynamicLibrary,
    }

    impl ReadyHost {
        fn load(
            creation: &eframe::CreationContext<'_>,
            bridge_override_variable: &str,
        ) -> Result<Self> {
            let parent_nsview = match creation
                .window_handle()
                .context("Spectrum's native window handle is unavailable")?
                .as_raw()
            {
                RawWindowHandle::AppKit(handle) => handle.ns_view,
                _ => bail!("Spectrum did not receive an AppKit window handle"),
            };
            let library_path = bridge_path(bridge_override_variable)?;
            let (library, api) = load_api_table(&SystemLibraryLoader, &library_path)
                .with_context(|| format!("could not open {}", library_path.display()))?;
            let (sender, receiver) = mpsc::channel();
            let callback = Box::new(CallbackContext { sender });
            let callback_raw = (&*callback as *const CallbackContext).cast_mut().cast();
            // Safety: callback stays boxed at a stable address until after the
            // runtime and all of its surfaces are destroyed.
            let runtime = initialize_runtime(&api, receive_event, callback_raw)?;
            Ok(Self {
                objects: OwnedNativeObjects::new(api, runtime),
                callback,
                receiver,
                parent_nsview,
                library,
            })
        }

        fn present(
            &mut self,
            session_id: u64,
            context: &TerminalContext,
            presentation: NativeSurfacePresentation,
        ) -> Result<()> {
            self.ensure_surface(session_id, context)?;
            let api = self.objects.bridge;
            for (id, surface) in &self.objects.surfaces {
                let state = if *id == session_id {
                    presentation
                } else {
                    NativeSurfacePresentation::hidden()
                };
                // Safety: every surface is owned by objects, and geometry is in
                // top-left logical points as required by the versioned bridge.
                unsafe {
                    (api.surface_set_state)(
                        *surface,
                        state.x,
                        state.y,
                        state.width,
                        state.height,
                        state.visible,
                        state.focus_requested,
                    );
                }
            }
            // Safety: runtime is live while objects exists.
            if let Some(runtime) = self.objects.runtime {
                unsafe {
                    (api.runtime_set_focus)(runtime, presentation.visible);
                    (api.runtime_tick)(runtime);
                }
            }
            Ok(())
        }

        fn ensure_surface(&mut self, session_id: u64, context: &TerminalContext) -> Result<()> {
            if self.objects.surfaces.contains_key(&session_id) {
                return Ok(());
            }
            let runtime = self
                .objects
                .runtime
                .context("Ghostty runtime was already shut down")?;
            let working_directory = encoded_working_directory(context)?;
            let environment = encoded_environment(context)?;
            // Safety: the bridge copies the two C strings synchronously. The
            // returned surface is tied to the still-live runtime and parent.
            let surface = unsafe {
                (self.objects.bridge.surface_create)(
                    runtime,
                    self.parent_nsview.as_ptr(),
                    session_id,
                    working_directory.as_ptr(),
                    environment.as_ptr(),
                )
            };
            if surface.is_null() {
                bail!("Ghostty could not create terminal surface {session_id}");
            }
            self.objects.surfaces.insert(session_id, surface);
            Ok(())
        }

        fn hide_all(&mut self) {
            let api = self.objects.bridge;
            for surface in self.objects.surfaces.values() {
                // Safety: the surface remains owned by objects.
                unsafe {
                    (api.surface_set_state)(*surface, 0.0, 0.0, 0.0, 0.0, false, false);
                }
            }
            if let Some(runtime) = self.objects.runtime {
                // Safety: runtime remains owned by objects.
                unsafe { (api.runtime_set_focus)(runtime, false) };
            }
        }

        fn edit(&self, session_id: u64, action: NativeEditAction) -> Result<()> {
            let surface = self
                .objects
                .surfaces
                .get(&session_id)
                .context("native terminal surface is not ready")?;
            // Safety: the surface remains owned for the complete synchronous
            // bridge call and the action value belongs to ABI version 1.
            let handled = unsafe { (self.objects.bridge.surface_edit)(*surface, action as u32) };
            if !handled {
                bail!("Ghostty did not handle the requested edit action");
            }
            Ok(())
        }

        fn request_close(&self, session_id: u64) -> Result<()> {
            let surface = self
                .objects
                .surfaces
                .get(&session_id)
                .context("native terminal surface is not ready")?;
            // Safety: the surface remains owned during the synchronous call.
            unsafe { (self.objects.bridge.surface_request_close)(*surface) };
            Ok(())
        }

        fn retain_sessions(&mut self, session_ids: &BTreeSet<u64>) {
            let stale: Vec<_> = self
                .objects
                .surfaces
                .keys()
                .copied()
                .filter(|id| !session_ids.contains(id))
                .collect();
            for id in stale {
                self.objects.remove_surface(id);
            }
        }

        fn poll(&mut self) -> Vec<NativeTerminalEvent> {
            self.receiver.try_iter().collect()
        }

        fn shutdown(&mut self) {
            self.objects.shutdown();
            let _ = &self.callback;
            let _ = &self.library;
        }
    }

    enum HostState {
        Disabled,
        Failed(String),
        Ready(ReadyHost),
    }

    #[derive(Clone, Copy, Debug)]
    pub struct NativeTerminalConfig {
        experiment_variable: &'static str,
        bridge_override_variable: &'static str,
    }

    impl NativeTerminalConfig {
        pub const fn new(
            experiment_variable: &'static str,
            bridge_override_variable: &'static str,
        ) -> Self {
            Self {
                experiment_variable,
                bridge_override_variable,
            }
        }
    }

    pub struct NativeTerminalHost {
        state: HostState,
    }

    impl NativeTerminalHost {
        pub fn from_environment(
            creation: &eframe::CreationContext<'_>,
            config: NativeTerminalConfig,
        ) -> Self {
            if !experiment_requested(std::env::var_os(config.experiment_variable).as_deref()) {
                return Self {
                    state: HostState::Disabled,
                };
            }
            let state = match ReadyHost::load(creation, config.bridge_override_variable) {
                Ok(host) => HostState::Ready(host),
                Err(error) => HostState::Failed(portable_fallback_diagnostic(&error)),
            };
            Self { state }
        }

        pub fn is_ready(&self) -> bool {
            matches!(self.state, HostState::Ready(_))
        }

        pub fn fallback_diagnostic(&self) -> Option<&str> {
            match &self.state {
                HostState::Failed(diagnostic) => Some(diagnostic),
                HostState::Disabled | HostState::Ready(_) => None,
            }
        }

        pub fn present(
            &mut self,
            session_id: u64,
            context: &TerminalContext,
            presentation: NativeSurfacePresentation,
        ) -> Result<()> {
            match &mut self.state {
                HostState::Ready(host) => host.present(session_id, context, presentation),
                HostState::Disabled => bail!("Ghostty experiment is disabled"),
                HostState::Failed(diagnostic) => bail!("{diagnostic}"),
            }
        }

        pub fn hide_all(&mut self) {
            if let HostState::Ready(host) = &mut self.state {
                host.hide_all();
            }
        }

        pub fn reset(&mut self, session_id: u64) {
            if let HostState::Ready(host) = &mut self.state {
                host.objects.remove_surface(session_id);
            }
        }

        pub fn edit(&self, session_id: u64, action: NativeEditAction) -> Result<()> {
            match &self.state {
                HostState::Ready(host) => host.edit(session_id, action),
                HostState::Disabled => bail!("Ghostty experiment is disabled"),
                HostState::Failed(diagnostic) => bail!("{diagnostic}"),
            }
        }

        pub fn request_close(&self, session_id: u64) -> Result<()> {
            match &self.state {
                HostState::Ready(host) => host.request_close(session_id),
                HostState::Disabled => bail!("Ghostty experiment is disabled"),
                HostState::Failed(diagnostic) => bail!("{diagnostic}"),
            }
        }

        pub fn retain_sessions(&mut self, session_ids: &BTreeSet<u64>) {
            if let HostState::Ready(host) = &mut self.state {
                host.retain_sessions(session_ids);
            }
        }

        pub fn poll(&mut self) -> Vec<NativeTerminalEvent> {
            match &mut self.state {
                HostState::Ready(host) => host.poll(),
                HostState::Disabled | HostState::Failed(_) => Vec::new(),
            }
        }

        pub fn shutdown(&mut self) {
            if let HostState::Ready(host) = &mut self.state {
                host.shutdown();
            }
            self.state = HostState::Disabled;
        }
    }

    impl Drop for NativeTerminalHost {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    fn encoded_environment(context: &TerminalContext) -> Result<CString> {
        let mut values = BTreeMap::from([
            ("COLORTERM".to_owned(), "truecolor".to_owned()),
            ("TERM".to_owned(), "xterm-ghostty".to_owned()),
        ]);
        for (key, value) in context.environment_variables() {
            let key = key
                .to_str()
                .context("terminal environment key is not valid Unicode")?;
            let value = value
                .to_str()
                .context("terminal environment value is not valid Unicode")?;
            values.insert(key.to_owned(), value.to_owned());
        }
        let json =
            serde_json::to_string(&values).context("could not encode terminal environment")?;
        CString::new(json).context("terminal environment contains an interior NUL")
    }

    fn encoded_working_directory(context: &TerminalContext) -> Result<CString> {
        let directory = context
            .working_directory()
            .to_str()
            .context("terminal working directory is not valid Unicode")?;
        CString::new(directory).context("terminal working directory contains an interior NUL")
    }

    fn bridge_path(override_variable: &str) -> Result<PathBuf> {
        if let Some(path) = std::env::var_os(override_variable) {
            return Ok(PathBuf::from(path));
        }
        let executable = std::env::current_exe().context("could not locate Spectrum executable")?;
        let macos = executable
            .parent()
            .context("Spectrum executable has no containing directory")?;
        Ok(macos
            .parent()
            .context("Spectrum executable is not inside an app bundle")?
            .join("Frameworks")
            .join(BRIDGE_FILENAME))
    }

    fn load_api_table<L: LibraryLoader>(
        loader: &L,
        path: &Path,
    ) -> Result<(L::Library, BridgeApi)> {
        let library = loader.open(path)?;
        let api = resolve_api_table(&library)?;
        Ok((library, api))
    }

    fn resolve_api_table(symbols: &impl SymbolSource) -> Result<BridgeApi> {
        macro_rules! load {
            ($name:literal, $ty:ty) => {{
                let symbol = symbols.symbol(c_str($name))?;
                // Safety: each symbol is checked against the versioned bridge
                // ABI before any handle is created.
                unsafe { std::mem::transmute::<*mut c_void, $ty>(symbol) }
            }};
        }
        let abi_version = load!(
            "spectrum_ghostty_bridge_abi_version\0",
            unsafe extern "C" fn() -> u32
        );
        // Safety: version query takes no arguments and has no side effects.
        let actual_version = unsafe { abi_version() };
        if actual_version != BRIDGE_ABI_VERSION {
            bail!("unsupported Ghostty bridge ABI {actual_version}; expected {BRIDGE_ABI_VERSION}");
        }
        Ok(BridgeApi {
            global_init: load!(
                "spectrum_ghostty_global_init\0",
                unsafe extern "C" fn() -> c_int
            ),
            runtime_create: load!(
                "spectrum_ghostty_runtime_create\0",
                unsafe extern "C" fn(EventCallback, *mut c_void) -> RuntimeHandle
            ),
            runtime_tick: load!(
                "spectrum_ghostty_runtime_tick\0",
                unsafe extern "C" fn(RuntimeHandle)
            ),
            runtime_set_focus: load!(
                "spectrum_ghostty_runtime_set_focus\0",
                unsafe extern "C" fn(RuntimeHandle, bool)
            ),
            runtime_destroy: load!(
                "spectrum_ghostty_runtime_destroy\0",
                unsafe extern "C" fn(RuntimeHandle)
            ),
            surface_create: load!(
                "spectrum_ghostty_surface_create\0",
                unsafe extern "C" fn(
                    RuntimeHandle,
                    *mut c_void,
                    u64,
                    *const c_char,
                    *const c_char,
                ) -> SurfaceHandle
            ),
            surface_set_state: load!(
                "spectrum_ghostty_surface_set_state\0",
                unsafe extern "C" fn(SurfaceHandle, f64, f64, f64, f64, bool, bool)
            ),
            surface_edit: load!(
                "spectrum_ghostty_surface_edit\0",
                unsafe extern "C" fn(SurfaceHandle, u32) -> bool
            ),
            surface_request_close: load!(
                "spectrum_ghostty_surface_request_close\0",
                unsafe extern "C" fn(SurfaceHandle)
            ),
            surface_destroy: load!(
                "spectrum_ghostty_surface_destroy\0",
                unsafe extern "C" fn(SurfaceHandle)
            ),
        })
    }

    fn initialize_runtime(
        api: &BridgeApi,
        callback: EventCallback,
        userdata: *mut c_void,
    ) -> Result<RuntimeHandle> {
        // Safety: the table has passed the ABI-version and required-symbol
        // checks, and callback userdata remains owned by ReadyHost.
        let initialized = unsafe { (api.global_init)() };
        if initialized != 0 {
            bail!("Ghostty global initialization failed ({initialized})");
        }
        // Safety: same table and callback lifetime contract as above.
        let runtime = unsafe { (api.runtime_create)(callback, userdata) };
        if runtime.is_null() {
            bail!("Ghostty runtime initialization failed");
        }
        Ok(runtime)
    }

    fn portable_fallback_diagnostic(error: &anyhow::Error) -> String {
        format!("Ghostty unavailable; using the portable terminal: {error:#}")
    }

    fn c_str(value: &'static str) -> &'static CStr {
        CStr::from_bytes_with_nul(value.as_bytes()).expect("bridge symbol names are NUL terminated")
    }

    unsafe extern "C" fn receive_event(
        userdata: *mut c_void,
        session_id: u64,
        event: u32,
        text: *const c_char,
        text_len: usize,
        process_alive: bool,
    ) {
        // Safety: the bridge received this pointer from the still-live runtime
        // owner and stops callbacks synchronously during runtime destruction.
        let Some(context) = (unsafe { (userdata as *const CallbackContext).as_ref() }) else {
            return;
        };
        let event = match event {
            EVENT_TITLE if !text.is_null() => {
                // Safety: the bridge guarantees that text points to text_len
                // live UTF-8 bytes for the duration of this callback.
                let bytes = unsafe { std::slice::from_raw_parts(text.cast::<u8>(), text_len) };
                NativeTerminalEvent::Title {
                    session_id,
                    title: String::from_utf8_lossy(bytes).into_owned(),
                }
            }
            EVENT_CLOSED => NativeTerminalEvent::Closed {
                session_id,
                process_alive,
            },
            _ => return,
        };
        let _ = context.sender.send(event);
    }

    fn experiment_requested(value: Option<&std::ffi::OsStr>) -> bool {
        value == Some(std::ffi::OsStr::new("1"))
    }

    fn dlerror_text() -> String {
        // Safety: dyld owns the NUL-terminated error until the next loader call.
        unsafe {
            let error = dlerror();
            if error.is_null() {
                "unknown dynamic loader error".to_owned()
            } else {
                CStr::from_ptr(error).to_string_lossy().into_owned()
            }
        }
    }

    const RTLD_LOCAL: c_int = 0x4;
    const RTLD_NOW: c_int = 0x2;
    const RTLD_NODELETE: c_int = 0x80;

    unsafe extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlerror() -> *const c_char;
    }

    #[cfg(test)]
    mod tests {
        include!("native_ghostty_macos_tests.rs");
    }
}

#[cfg(all(target_os = "macos", feature = "ghostty-terminal"))]
pub use macos::{NativeEditAction, NativeTerminalConfig, NativeTerminalEvent, NativeTerminalHost};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_uses_top_left_points_and_hides_for_overlays() {
        let rect = egui::Rect::from_min_size(egui::pos2(14.0, 22.0), egui::vec2(800.0, 500.0));
        let shown = NativeSurfacePresentation::for_terminal(rect, true, true, false, true);
        assert_eq!((shown.x, shown.y), (14.0, 22.0));
        assert!(shown.visible);
        assert!(shown.focus_requested);

        let hidden = NativeSurfacePresentation::for_terminal(rect, true, true, true, true);
        assert!(!hidden.visible);
        assert!(!hidden.focus_requested);
    }

    #[test]
    fn only_the_active_terminal_surface_is_visible() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 480.0));
        assert!(NativeSurfacePresentation::for_terminal(rect, true, true, false, false).visible);
        assert!(!NativeSurfacePresentation::for_terminal(rect, true, false, false, false).visible);
    }
}
