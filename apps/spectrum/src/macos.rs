use std::{
    path::PathBuf,
    sync::{
        OnceLock,
        mpsc::{self, Sender},
    },
};

use objc2::{
    ffi,
    runtime::{Imp, ProtocolObject, Sel},
    sel,
};
use objc2_app_kit::{NSApplication, NSApplicationDelegate};
use objc2_foundation::{MainThreadMarker, NSArray, NSURL};
use winit::platform::macos::EventLoopBuilderExtMacOS;

use super::*;

static OPEN_DOCUMENT_SENDER: OnceLock<Sender<PathBuf>> = OnceLock::new();
static APP_REPAINT: OnceLock<egui::Context> = OnceLock::new();

unsafe extern "C-unwind" fn application_open_urls(
    _delegate: *mut objc2::runtime::AnyObject,
    _selector: Sel,
    _application: *mut NSApplication,
    urls: *mut NSArray<NSURL>,
) {
    let (Some(sender), Some(urls)) = (OPEN_DOCUMENT_SENDER.get(), unsafe { urls.as_ref() }) else {
        return;
    };
    let mut queued = false;
    for url in urls {
        let _ = unsafe { url.startAccessingSecurityScopedResource() };
        if let Some(path) = url.to_file_path() {
            queued |= sender.send(path).is_ok();
        }
    }
    if queued && let Some(context) = APP_REPAINT.get() {
        context.request_repaint();
    }
}

fn install_open_documents(sender: Sender<PathBuf>) {
    let _ = OPEN_DOCUMENT_SENDER.set(sender);
    let marker = MainThreadMarker::new().expect("Spectrum starts on the macOS main thread");
    let application = NSApplication::sharedApplication(marker);
    let delegate = application
        .delegate()
        .expect("winit configures the macOS application delegate");
    let delegate_protocol: &ProtocolObject<dyn NSApplicationDelegate> = &delegate;
    let class = delegate_protocol.as_ref().class();
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
        "could not install Spectrum's open-document handler"
    );
}

pub(super) fn run() -> eframe::Result {
    let startup_path = std::env::args_os().nth(1).map(PathBuf::from);
    let (open_document_sender, open_document_receiver) = mpsc::channel();
    let mut event_loop_builder =
        winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event();
    event_loop_builder.with_default_menu(false);
    let event_loop = event_loop_builder.build()?;
    install_open_documents(open_document_sender);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 940.0])
            .with_min_inner_size([1060.0, 680.0])
            .with_icon(spectrum_icon()),
        centered: true,
        ..Default::default()
    };
    let mut application = eframe::create_native(
        "Spectrum",
        options,
        Box::new(move |creation| {
            let _ = APP_REPAINT.set(creation.egui_ctx.clone());
            Ok(Box::new(SpectrumApp::new(
                creation,
                startup_path.clone(),
                open_document_receiver,
            )))
        }),
        &event_loop,
    );
    event_loop.run_app(&mut application)?;
    Ok(())
}
