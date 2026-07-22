use std::{
    path::PathBuf,
    sync::{
        OnceLock,
        mpsc::{self, Receiver, Sender},
    },
};

use objc2::{
    ffi,
    rc::Retained,
    runtime::{AnyObject, Imp, ProtocolObject, Sel},
    sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationDelegate, NSEventModifierFlags, NSMenu, NSMenuItem,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSString, NSURL};
use winit::platform::macos::EventLoopBuilderExtMacOS;

use super::*;
use crate::macos_menu_spec::{
    ACTION_MENU_ITEMS, ActionKeyEquivalent, KeyModifiers, NativeMenuAction, NativeMenuSection,
};

static OPEN_DOCUMENT_SENDER: OnceLock<Sender<PathBuf>> = OnceLock::new();
static NATIVE_MENU_SENDER: OnceLock<Sender<NativeMenuAction>> = OnceLock::new();
static APP_REPAINT: OnceLock<egui::Context> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct NativeMenuState {
    modal_open: bool,
    catalog_movable: bool,
    has_photos: bool,
    selection_present: bool,
    can_undo: bool,
    can_redo: bool,
    can_previous: bool,
    can_next: bool,
    all_shoots_visible: bool,
    history_visible: bool,
    keyboard_focus: bool,
}

impl NativeMenuState {
    fn allows(self, action: NativeMenuAction) -> bool {
        match action {
            NativeMenuAction::NewCatalog | NativeMenuAction::OpenCatalog => !self.modal_open,
            NativeMenuAction::ImportPhotos => !self.modal_open && !self.history_visible,
            NativeMenuAction::MoveCatalog => !self.modal_open && self.catalog_movable,
            NativeMenuAction::ExportPhotos => {
                !self.modal_open
                    && self.selection_present
                    && !self.all_shoots_visible
                    && !self.history_visible
            }
            NativeMenuAction::Undo => !self.modal_open && self.can_undo && !self.keyboard_focus,
            NativeMenuAction::Redo => !self.modal_open && self.can_redo && !self.keyboard_focus,
            NativeMenuAction::ToggleAllShoots => !self.modal_open && self.has_photos,
            NativeMenuAction::PreviousPhoto => {
                !self.modal_open
                    && self.can_previous
                    && !self.all_shoots_visible
                    && !self.history_visible
                    && !self.keyboard_focus
            }
            NativeMenuAction::NextPhoto => {
                !self.modal_open
                    && self.can_next
                    && !self.all_shoots_visible
                    && !self.history_visible
                    && !self.keyboard_focus
            }
            NativeMenuAction::ToggleHistory => {
                !self.modal_open
                    && self.selection_present
                    && !self.all_shoots_visible
                    && !self.keyboard_focus
            }
            NativeMenuAction::FitPhoto | NativeMenuAction::ZoomIn | NativeMenuAction::ZoomOut => {
                !self.modal_open
                    && self.selection_present
                    && !self.all_shoots_visible
                    && !self.history_visible
            }
        }
    }

    fn title(self, action: NativeMenuAction) -> Option<&'static str> {
        match action {
            NativeMenuAction::ToggleAllShoots => Some(if self.all_shoots_visible {
                "Back to Photos"
            } else {
                "Show All Shoots"
            }),
            NativeMenuAction::ToggleHistory => Some(if self.history_visible {
                "Hide History"
            } else {
                "Show History"
            }),
            _ => None,
        }
    }
}

pub(super) struct NativeMenuBridge {
    receiver: Receiver<NativeMenuAction>,
    last_state: Option<NativeMenuState>,
}

impl NativeMenuBridge {
    fn new(receiver: Receiver<NativeMenuAction>) -> Self {
        Self {
            receiver,
            last_state: None,
        }
    }
}

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
        let _ = unsafe { url.startAccessingSecurityScopedResource() };
        if let Some(path) = url.to_file_path() {
            queued |= sender.send(path).is_ok();
        }
    }
    if queued {
        request_app_repaint();
    }
}

unsafe extern "C-unwind" fn perform_native_menu_action(
    _delegate: *mut AnyObject,
    _selector: Sel,
    item: *mut NSMenuItem,
) {
    let Some(action) =
        (unsafe { item.as_ref() }).and_then(|item| NativeMenuAction::from_tag(item.tag()))
    else {
        return;
    };
    if let Some(sender) = NATIVE_MENU_SENDER.get()
        && sender.send(action).is_ok()
    {
        request_app_repaint();
    }
}

fn request_app_repaint() {
    if let Some(context) = APP_REPAINT.get() {
        context.request_repaint();
    }
}

fn install_app_integration(
    open_document_sender: Sender<PathBuf>,
    native_menu_sender: Sender<NativeMenuAction>,
) {
    let _ = OPEN_DOCUMENT_SENDER.set(open_document_sender);
    let _ = NATIVE_MENU_SENDER.set(native_menu_sender);
    let marker = MainThreadMarker::new().expect("Lumen starts on the macOS main thread");
    let application = NSApplication::sharedApplication(marker);
    let delegate = application
        .delegate()
        .expect("winit configures an application delegate while creating its event loop");
    let delegate_protocol: &ProtocolObject<dyn NSApplicationDelegate> = &delegate;
    let delegate_object: &AnyObject = delegate_protocol.as_ref();
    let class = delegate_object.class();

    let open_implementation: Imp = unsafe {
        std::mem::transmute(application_open_urls as unsafe extern "C-unwind" fn(_, _, _, _))
    };
    let open_added = unsafe {
        ffi::class_addMethod(
            class as *const _ as *mut _,
            sel!(application:openURLs:),
            open_implementation,
            c"v@:@@".as_ptr(),
        )
    };
    assert!(
        open_added.as_bool(),
        "could not install Lumen's macOS open-document handler"
    );

    let menu_implementation: Imp = unsafe {
        std::mem::transmute(perform_native_menu_action as unsafe extern "C-unwind" fn(_, _, _))
    };
    let menu_added = unsafe {
        ffi::class_addMethod(
            class as *const _ as *mut _,
            sel!(performLumenMenuAction:),
            menu_implementation,
            c"v@:@".as_ptr(),
        )
    };
    assert!(
        menu_added.as_bool(),
        "could not install Lumen's macOS menu action handler"
    );
    install_main_menu(&application, delegate_object, marker);
}

pub(super) fn run(initial_catalog: Option<PathBuf>) -> eframe::Result {
    let (open_document_sender, open_document_receiver) = mpsc::channel();
    let (native_menu_sender, native_menu_receiver) = mpsc::channel();
    let mut event_loop_builder =
        winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event();
    event_loop_builder.with_default_menu(false);
    let event_loop = event_loop_builder.build()?;
    install_app_integration(open_document_sender, native_menu_sender);
    let mut application = eframe::create_native(
        "Lumen",
        native_options(),
        Box::new(move |creation| {
            let _ = APP_REPAINT.set(creation.egui_ctx.clone());
            Ok(Box::new(LumenApp::new(
                creation,
                initial_catalog.clone(),
                open_document_receiver,
                NativeMenuBridge::new(native_menu_receiver),
            )))
        }),
        &event_loop,
    );
    event_loop.run_app(&mut application)?;
    Ok(())
}

#[derive(Clone, Copy)]
struct KeyEquivalent {
    key: &'static str,
    modifiers: NSEventModifierFlags,
}

fn key(key: &'static str) -> KeyEquivalent {
    KeyEquivalent {
        key,
        modifiers: NSEventModifierFlags::Command,
    }
}

impl From<ActionKeyEquivalent> for KeyEquivalent {
    fn from(value: ActionKeyEquivalent) -> Self {
        let modifiers = match value.modifiers {
            KeyModifiers::Command => NSEventModifierFlags::Command,
            KeyModifiers::CommandShift => {
                NSEventModifierFlags::Command | NSEventModifierFlags::Shift
            }
        };
        Self {
            key: value.key,
            modifiers,
        }
    }
}

fn menu_item(
    marker: MainThreadMarker,
    title: &str,
    selector: Option<Sel>,
    equivalent: Option<KeyEquivalent>,
) -> Retained<NSMenuItem> {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            marker.alloc(),
            &NSString::from_str(title),
            selector,
            &NSString::from_str(equivalent.map_or("", |value| value.key)),
        )
    };
    if let Some(equivalent) = equivalent {
        item.setKeyEquivalentModifierMask(equivalent.modifiers);
    }
    item
}

fn action_item(
    marker: MainThreadMarker,
    target: &AnyObject,
    title: &str,
    action: NativeMenuAction,
    equivalent: Option<ActionKeyEquivalent>,
) -> Retained<NSMenuItem> {
    let item = menu_item(
        marker,
        title,
        Some(sel!(performLumenMenuAction:)),
        equivalent.map(Into::into),
    );
    item.setTag(action as isize);
    unsafe { item.setTarget(Some(target)) };
    item
}

fn submenu(marker: MainThreadMarker, menu_bar: &NSMenu, title: &str) -> Retained<NSMenu> {
    let root = menu_item(marker, title, None, None);
    let menu = NSMenu::initWithTitle(marker.alloc(), &NSString::from_str(title));
    root.setSubmenu(Some(&menu));
    menu_bar.addItem(&root);
    menu
}

fn action_submenu(
    marker: MainThreadMarker,
    menu_bar: &NSMenu,
    target: &AnyObject,
    section: NativeMenuSection,
) {
    let menu = submenu(marker, menu_bar, section.title());
    menu.setAutoenablesItems(false);
    for spec in ACTION_MENU_ITEMS
        .into_iter()
        .filter(|spec| spec.section == section)
    {
        if spec.separator_before {
            menu.addItem(&NSMenuItem::separatorItem(marker));
        }
        menu.addItem(&action_item(
            marker,
            target,
            spec.title,
            spec.action,
            spec.equivalent,
        ));
    }
}

fn install_main_menu(application: &NSApplication, target: &AnyObject, marker: MainThreadMarker) {
    let menu_bar = NSMenu::new(marker);
    let app_menu = submenu(marker, &menu_bar, "Lumen");
    app_menu.addItem(&menu_item(
        marker,
        "About Lumen",
        Some(sel!(orderFrontStandardAboutPanel:)),
        None,
    ));
    app_menu.addItem(&NSMenuItem::separatorItem(marker));
    let services_menu = NSMenu::new(marker);
    let services_item = menu_item(marker, "Services", None, None);
    services_item.setSubmenu(Some(&services_menu));
    app_menu.addItem(&services_item);
    app_menu.addItem(&NSMenuItem::separatorItem(marker));
    app_menu.addItem(&menu_item(marker, "Hide Lumen", Some(sel!(hide:)), None));
    app_menu.addItem(&menu_item(
        marker,
        "Hide Others",
        Some(sel!(hideOtherApplications:)),
        Some(KeyEquivalent {
            key: "h",
            modifiers: NSEventModifierFlags::Command | NSEventModifierFlags::Option,
        }),
    ));
    app_menu.addItem(&menu_item(
        marker,
        "Show All",
        Some(sel!(unhideAllApplications:)),
        None,
    ));
    app_menu.addItem(&NSMenuItem::separatorItem(marker));
    app_menu.addItem(&menu_item(
        marker,
        "Quit Lumen",
        Some(sel!(terminate:)),
        Some(key("q")),
    ));
    application.setServicesMenu(Some(&services_menu));

    action_submenu(marker, &menu_bar, target, NativeMenuSection::File);
    action_submenu(marker, &menu_bar, target, NativeMenuSection::Edit);
    action_submenu(marker, &menu_bar, target, NativeMenuSection::View);

    let window_menu = submenu(marker, &menu_bar, "Window");
    window_menu.addItem(&menu_item(
        marker,
        "Minimize",
        Some(sel!(performMiniaturize:)),
        Some(key("m")),
    ));
    window_menu.addItem(&menu_item(marker, "Zoom", Some(sel!(performZoom:)), None));
    window_menu.addItem(&NSMenuItem::separatorItem(marker));
    window_menu.addItem(&menu_item(
        marker,
        "Bring All to Front",
        Some(sel!(arrangeInFront:)),
        None,
    ));
    application.setWindowsMenu(Some(&window_menu));
    application.setMainMenu(Some(&menu_bar));
    update_menu_state(NativeMenuState::default());
}

fn find_item(menu: &NSMenu, action: NativeMenuAction) -> Option<Retained<NSMenuItem>> {
    for item in &menu.itemArray() {
        if item.tag() == action as isize {
            return Some(item);
        }
        if let Some(submenu) = item.submenu()
            && let Some(found) = find_item(&submenu, action)
        {
            return Some(found);
        }
    }
    None
}

fn update_menu_state(state: NativeMenuState) {
    let marker = MainThreadMarker::new().expect("Lumen updates menus on the macOS main thread");
    let application = NSApplication::sharedApplication(marker);
    let Some(menu) = application.mainMenu() else {
        return;
    };
    for action in NativeMenuAction::ALL {
        let Some(item) = find_item(&menu, action) else {
            continue;
        };
        item.setEnabled(state.allows(action));
        if let Some(title) = state.title(action) {
            item.setTitle(&NSString::from_str(title));
        }
    }
}

impl LumenApp {
    fn native_menu_state(&self, context: &egui::Context) -> NativeMenuState {
        let visible = self.visible_photo_ids();
        let selected_index = self
            .workspace
            .project
            .selected
            .and_then(|id| visible.iter().position(|visible_id| *visible_id == id));
        NativeMenuState {
            modal_open: self.reset_confirmation
                || self.remove_confirmation
                || self.pending_catalog_switch.is_some()
                || self.rename_batch.is_some()
                || self.export_open,
            catalog_movable: self.workspace.catalog_path.is_some(),
            has_photos: !self.workspace.project.photos.is_empty(),
            selection_present: self.workspace.project.selected.is_some(),
            can_undo: self.workspace.can_undo(),
            can_redo: self.workspace.can_redo(),
            can_previous: selected_index.is_some_and(|index| index > 0),
            can_next: selected_index.is_some_and(|index| index + 1 < visible.len()),
            all_shoots_visible: self.library_mode,
            history_visible: self.history_open,
            keyboard_focus: context.egui_wants_keyboard_input(),
        }
    }

    pub(super) fn process_native_menu_actions(&mut self, context: &egui::Context) {
        let actions: Vec<_> = self.native_menu.receiver.try_iter().collect();
        for action in actions {
            if !self.native_menu_state(context).allows(action) {
                continue;
            }
            match action {
                NativeMenuAction::NewCatalog => self.new_catalog(),
                NativeMenuAction::OpenCatalog => self.open_catalog(),
                NativeMenuAction::ImportPhotos => self.import_dialog(),
                NativeMenuAction::MoveCatalog => self.move_project(),
                NativeMenuAction::ExportPhotos => self.open_export(),
                NativeMenuAction::Undo | NativeMenuAction::Redo => {
                    let command = if action == NativeMenuAction::Undo {
                        Command::Undo
                    } else {
                        Command::Redo
                    };
                    if self.execute(command) {
                        self.draft_id = None;
                        self.sync_draft();
                    }
                }
                NativeMenuAction::ToggleAllShoots => {
                    if self.library_mode {
                        self.return_to_photo_view();
                    } else {
                        self.library_mode = true;
                    }
                }
                NativeMenuAction::PreviousPhoto => self.select_relative(-1),
                NativeMenuAction::NextPhoto => self.select_relative(1),
                NativeMenuAction::ToggleHistory => self.toggle_history(),
                NativeMenuAction::FitPhoto => {
                    self.zoom = 1.0;
                    self.pan = Vec2::ZERO;
                }
                NativeMenuAction::ZoomIn => self.zoom = (self.zoom * 1.25).min(8.0),
                NativeMenuAction::ZoomOut => self.zoom = (self.zoom / 1.25).max(0.25),
            }
        }
    }

    pub(super) fn sync_native_menu_state(&mut self, context: &egui::Context) {
        let state = self.native_menu_state(context);
        if self.native_menu.last_state == Some(state) {
            return;
        }
        update_menu_state(state);
        self.native_menu.last_state = Some(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_actions_follow_photo_and_modal_context() {
        let photo = NativeMenuState {
            catalog_movable: true,
            has_photos: true,
            selection_present: true,
            can_undo: true,
            can_previous: true,
            ..Default::default()
        };
        assert!(photo.allows(NativeMenuAction::ImportPhotos));
        assert!(photo.allows(NativeMenuAction::ExportPhotos));
        assert!(photo.allows(NativeMenuAction::Undo));
        assert!(photo.allows(NativeMenuAction::ToggleAllShoots));
        assert!(photo.allows(NativeMenuAction::PreviousPhoto));

        let modal = NativeMenuState {
            modal_open: true,
            ..photo
        };
        assert!(
            NativeMenuAction::ALL
                .into_iter()
                .all(|action| !modal.allows(action))
        );
    }

    #[test]
    fn all_shoots_mode_offers_an_explicit_return() {
        let state = NativeMenuState {
            has_photos: true,
            all_shoots_visible: true,
            ..Default::default()
        };
        assert_eq!(
            state.title(NativeMenuAction::ToggleAllShoots),
            Some("Back to Photos")
        );
        assert!(!state.allows(NativeMenuAction::ExportPhotos));
        assert!(!state.allows(NativeMenuAction::PreviousPhoto));
    }
}
