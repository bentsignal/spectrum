use std::{
    path::PathBuf,
    sync::{OnceLock, mpsc::Sender},
};

use objc2::{
    ffi,
    runtime::{AnyObject, Imp, ProtocolObject, Sel},
    sel,
};
use objc2_app_kit::{NSApplication, NSApplicationDelegate};
use objc2_foundation::{MainThreadMarker, NSArray, NSURL};

static OPEN_DOCUMENT_SENDER: OnceLock<Sender<PathBuf>> = OnceLock::new();
static OPEN_DOCUMENT_REPAINT: OnceLock<eframe::egui::Context> = OnceLock::new();

unsafe extern "C-unwind" fn application_open_urls(
    _delegate: *mut AnyObject,
    _selector: Sel,
    _application: *mut NSApplication,
    urls: *mut NSArray<NSURL>,
) {
    let (Some(sender), Some(urls)) = (OPEN_DOCUMENT_SENDER.get(), unsafe { urls.as_ref() }) else {
        return;
    };
    let mut queued = false;
    for url in urls {
        // Launch Services may hand Prism a security-scoped URL for protected folders such as
        // Downloads. Keep that access for the process lifetime because the live revision store
        // needs to continue reading and writing the project after this callback returns.
        let _ = unsafe { url.startAccessingSecurityScopedResource() };
        if let Some(path) = url.to_file_path() {
            queued |= sender.send(path).is_ok();
        }
    }
    if queued && let Some(context) = OPEN_DOCUMENT_REPAINT.get() {
        context.request_repaint();
    }
}

pub(super) fn install_open_document_repaint(context: eframe::egui::Context) {
    let _ = OPEN_DOCUMENT_REPAINT.set(context);
}

pub(super) fn install_open_document_handler(sender: Sender<PathBuf>) {
    let _ = OPEN_DOCUMENT_SENDER.set(sender);
    let marker = MainThreadMarker::new().expect("Prism starts on the macOS main thread");
    let application = NSApplication::sharedApplication(marker);
    let delegate = application
        .delegate()
        .expect("winit configures an application delegate while creating its event loop");
    let delegate_protocol: &ProtocolObject<dyn NSApplicationDelegate> = &delegate;
    let delegate_object: &AnyObject = delegate_protocol.as_ref();
    let class = delegate_object.class();
    let implementation: Imp = unsafe {
        std::mem::transmute(application_open_urls as unsafe extern "C-unwind" fn(_, _, _, _))
    };
    let added = unsafe {
        ffi::class_addMethod(
            class as *const _ as *mut _,
            sel!(application:openURLs:),
            implementation,
            c"v@:@@".as_ptr(),
        )
    };
    assert!(
        added.as_bool(),
        "could not install Prism's macOS open-document handler"
    );
}
