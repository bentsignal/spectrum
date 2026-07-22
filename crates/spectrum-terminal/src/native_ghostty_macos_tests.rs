use std::{cell::RefCell, rc::Rc};

use super::*;

#[derive(Clone)]
struct FakeBridge(Rc<RefCell<Vec<String>>>);

impl LifecycleBridge for FakeBridge {
    type Runtime = usize;
    type Surface = usize;

    fn destroy_surface(&self, surface: Self::Surface) {
        self.0.borrow_mut().push(format!("surface:{surface}"));
    }

    fn destroy_runtime(&self, runtime: Self::Runtime) {
        self.0.borrow_mut().push(format!("runtime:{runtime}"));
    }
}

#[test]
fn native_objects_destroy_each_surface_once_before_runtime() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut objects = OwnedNativeObjects::new(FakeBridge(calls.clone()), 9);
    objects.surfaces.insert(2, 22);
    objects.surfaces.insert(1, 11);
    objects.remove_surface(1);
    objects.remove_surface(1);
    objects.shutdown();
    objects.shutdown();
    drop(objects);
    assert_eq!(&*calls.borrow(), &["surface:11", "surface:22", "runtime:9"]);
}

#[test]
fn experiment_requires_exact_opt_in() {
    assert!(experiment_requested(Some(std::ffi::OsStr::new("1"))));
    assert!(!experiment_requested(Some(std::ffi::OsStr::new("true"))));
    assert!(!experiment_requested(None));
}

#[test]
fn non_unicode_working_directory_falls_back_before_crossing_the_bridge() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let context = TerminalContext::new(OsString::from_vec(vec![b'/', b't', b'm', b'p', 0xff]));
    let error = encoded_working_directory(&context)
        .expect_err("non-Unicode paths must not be repaired by the Swift C-string decoder");
    assert!(error.to_string().contains("not valid Unicode"));
}

#[derive(Clone, Copy)]
struct FakeSymbols {
    missing: Option<&'static str>,
    abi: unsafe extern "C" fn() -> u32,
}

impl SymbolSource for FakeSymbols {
    fn symbol(&self, name: &'static CStr) -> Result<*mut c_void> {
        let name = name.to_str().expect("test symbols are ASCII");
        if self.missing == Some(name) {
            anyhow::bail!("required bridge symbol is unavailable: {name}");
        }
        if name == "spectrum_ghostty_bridge_abi_version" {
            return Ok(self.abi as *const () as *mut c_void);
        }
        Ok(fake_unused_symbol as *const () as *mut c_void)
    }
}

struct FakeLibraryLoader {
    open_failure: bool,
    symbols: FakeSymbols,
}

impl LibraryLoader for FakeLibraryLoader {
    type Library = FakeSymbols;

    fn open(&self, _path: &Path) -> Result<Self::Library> {
        if self.open_failure {
            anyhow::bail!("could not load Ghostty bridge")
        }
        Ok(self.symbols)
    }
}

unsafe extern "C" fn fake_abi_v1() -> u32 {
    BRIDGE_ABI_VERSION
}

unsafe extern "C" fn fake_abi_v2() -> u32 {
    BRIDGE_ABI_VERSION + 1
}

unsafe extern "C" fn fake_unused_symbol() {}

unsafe extern "C" fn fake_global_init() -> c_int {
    0
}

unsafe extern "C" fn fake_runtime_create_failure(
    _callback: EventCallback,
    _userdata: *mut c_void,
) -> RuntimeHandle {
    std::ptr::null_mut()
}

fn fake_loader(
    open_failure: bool,
    missing: Option<&'static str>,
    abi: unsafe extern "C" fn() -> u32,
) -> FakeLibraryLoader {
    FakeLibraryLoader {
        open_failure,
        symbols: FakeSymbols { missing, abi },
    }
}

fn assert_portable_fallback(error: anyhow::Error, expected: &str) {
    let diagnostic = portable_fallback_diagnostic(&error);
    assert!(diagnostic.contains(expected));
    assert!(diagnostic.contains("using the portable terminal"));
}

#[test]
fn missing_dynamic_library_falls_back_to_portable() {
    let error = load_api_table(
        &fake_loader(true, None, fake_abi_v1),
        Path::new("/missing/bridge"),
    )
    .err()
    .expect("fake loader must reject a missing library");
    assert_portable_fallback(error, "could not load Ghostty bridge");
}

#[test]
fn missing_required_symbol_falls_back_to_portable() {
    let error = load_api_table(
        &fake_loader(false, Some("spectrum_ghostty_runtime_create"), fake_abi_v1),
        Path::new("/fake/bridge"),
    )
    .err()
    .expect("API resolution must reject a missing required symbol");
    assert_portable_fallback(error, "spectrum_ghostty_runtime_create");
}

#[test]
fn incompatible_abi_falls_back_to_portable() {
    let error = load_api_table(
        &fake_loader(false, None, fake_abi_v2),
        Path::new("/fake/bridge"),
    )
    .err()
    .expect("API resolution must reject an incompatible ABI");
    assert_portable_fallback(error, "unsupported Ghostty bridge ABI");
}

#[test]
fn runtime_creation_failure_falls_back_to_portable() {
    let (_, mut api) = load_api_table(
        &fake_loader(false, None, fake_abi_v1),
        Path::new("/fake/bridge"),
    )
    .expect("complete fake API table must resolve");
    api.global_init = fake_global_init;
    api.runtime_create = fake_runtime_create_failure;
    let error = initialize_runtime(&api, receive_event, std::ptr::null_mut())
        .expect_err("null runtime must trigger portable fallback");
    assert_portable_fallback(error, "Ghostty runtime initialization failed");
}
