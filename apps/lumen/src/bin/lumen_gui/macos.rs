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

unsafe extern "C-unwind" fn application_open_urls(
    _delegate: *mut AnyObject,
    _selector: Sel,
    _application: *mut NSApplication,
    urls: *mut NSArray<NSURL>,
) {
    let (Some(sender), Some(urls)) = (OPEN_DOCUMENT_SENDER.get(), unsafe { urls.as_ref() }) else {
        return;
    };
    for url in urls {
        let _ = unsafe { url.startAccessingSecurityScopedResource() };
        if let Some(path) = url.to_file_path() {
            let _ = sender.send(path);
        }
    }
}

pub(super) fn install_open_document_handler(sender: Sender<PathBuf>) {
    let _ = OPEN_DOCUMENT_SENDER.set(sender);
    let marker = MainThreadMarker::new().expect("Lumen starts on the macOS main thread");
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
        "could not install Lumen's macOS open-document handler"
    );
}
