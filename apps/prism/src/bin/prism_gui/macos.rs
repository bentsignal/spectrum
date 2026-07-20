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

use super::*;

static OPEN_DOCUMENT_SENDER: OnceLock<Sender<PathBuf>> = OnceLock::new();
static NATIVE_MENU_SENDER: OnceLock<Sender<NativeMenuAction>> = OnceLock::new();
static APP_REPAINT: OnceLock<egui::Context> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(isize)]
pub(super) enum NativeMenuAction {
    NewDocument = 44_001,
    OpenDocument,
    MoveProject,
    Export,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    ToggleHistory,
    ToggleTerminal,
    FitCanvas,
    ZoomIn,
    ZoomOut,
}

impl NativeMenuAction {
    const ALL: [Self; 14] = [
        Self::NewDocument,
        Self::OpenDocument,
        Self::MoveProject,
        Self::Export,
        Self::Undo,
        Self::Redo,
        Self::Cut,
        Self::Copy,
        Self::Paste,
        Self::ToggleHistory,
        Self::ToggleTerminal,
        Self::FitCanvas,
        Self::ZoomIn,
        Self::ZoomOut,
    ];

    fn from_tag(tag: isize) -> Option<Self> {
        Self::ALL.into_iter().find(|action| *action as isize == tag)
    }
}

fn clipboard_viewport_command(action: NativeMenuAction) -> Option<egui::ViewportCommand> {
    match action {
        NativeMenuAction::Cut => Some(egui::ViewportCommand::RequestCut),
        NativeMenuAction::Copy => Some(egui::ViewportCommand::RequestCopy),
        NativeMenuAction::Paste => Some(egui::ViewportCommand::RequestPaste),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct NativeMenuState {
    pub(super) modal_open: bool,
    pub(super) workspace_ready: bool,
    pub(super) can_move_project: bool,
    pub(super) can_undo: bool,
    pub(super) can_redo: bool,
    pub(super) history_visible: bool,
    pub(super) terminal_visible: bool,
    pub(super) keyboard_focus: bool,
    pub(super) selection_present: bool,
    pub(super) interaction_active: bool,
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

impl NativeMenuState {
    fn allows(self, action: NativeMenuAction) -> bool {
        if matches!(
            action,
            NativeMenuAction::Cut | NativeMenuAction::Copy | NativeMenuAction::Paste
        ) {
            if self.keyboard_focus || self.terminal_visible {
                return true;
            }
            return !self.modal_open
                && self.workspace_ready
                && !self.history_visible
                && !self.interaction_active
                && (action == NativeMenuAction::Paste || self.selection_present);
        }
        if self.modal_open {
            return false;
        }
        match action {
            NativeMenuAction::NewDocument | NativeMenuAction::OpenDocument => true,
            NativeMenuAction::MoveProject => self.workspace_ready && self.can_move_project,
            NativeMenuAction::Export | NativeMenuAction::ToggleTerminal => self.workspace_ready,
            NativeMenuAction::ToggleHistory => {
                self.workspace_ready && !self.keyboard_focus && !self.terminal_visible
            }
            NativeMenuAction::Undo => {
                self.workspace_ready
                    && self.can_undo
                    && !self.keyboard_focus
                    && !self.terminal_visible
            }
            NativeMenuAction::Redo => {
                self.workspace_ready
                    && self.can_redo
                    && !self.keyboard_focus
                    && !self.terminal_visible
            }
            NativeMenuAction::FitCanvas | NativeMenuAction::ZoomIn | NativeMenuAction::ZoomOut => {
                self.workspace_ready && !self.history_visible && !self.terminal_visible
            }
        }
    }

    fn title(self, action: NativeMenuAction) -> Option<&'static str> {
        match action {
            NativeMenuAction::ToggleHistory => Some(if self.history_visible {
                "Hide History"
            } else {
                "Show History"
            }),
            NativeMenuAction::ToggleTerminal => Some(if self.terminal_visible {
                "Hide Terminal"
            } else {
                "Show Terminal"
            }),
            _ => None,
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
        // Launch Services may hand Prism a security-scoped URL for protected folders such as
        // Downloads. Keep that access for the process lifetime because the live revision store
        // needs to continue reading and writing the project after this callback returns.
        let _ = unsafe { url.startAccessingSecurityScopedResource() };
        if let Some(path) = url.to_file_path() {
            queued |= sender.send(path).is_ok();
        }
    }
    if queued {
        request_app_repaint();
    }
}

fn install_app_repaint(context: egui::Context) {
    let _ = APP_REPAINT.set(context);
}

fn request_app_repaint() {
    if let Some(context) = APP_REPAINT.get() {
        context.request_repaint();
    }
}

fn queue_and_wake<T>(sender: &Sender<T>, value: T, wake: impl FnOnce()) -> bool {
    if sender.send(value).is_err() {
        return false;
    }
    wake();
    true
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
    if let Some(sender) = NATIVE_MENU_SENDER.get() {
        queue_and_wake(sender, action, request_app_repaint);
    }
}

fn install_app_integration(
    open_document_sender: Sender<PathBuf>,
    native_menu_sender: Sender<NativeMenuAction>,
) {
    let _ = OPEN_DOCUMENT_SENDER.set(open_document_sender);
    let _ = NATIVE_MENU_SENDER.set(native_menu_sender);
    let marker = MainThreadMarker::new().expect("Prism starts on the macOS main thread");
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
        "could not install Prism's macOS open-document handler"
    );

    let menu_implementation: Imp = unsafe {
        std::mem::transmute(perform_native_menu_action as unsafe extern "C-unwind" fn(_, _, _))
    };
    let menu_added = unsafe {
        ffi::class_addMethod(
            class as *const _ as *mut _,
            sel!(performPrismMenuAction:),
            menu_implementation,
            c"v@:@".as_ptr(),
        )
    };
    assert!(
        menu_added.as_bool(),
        "could not install Prism's macOS menu action handler"
    );

    install_main_menu(&application, delegate_object, marker);
}

pub(super) fn run(initial_project: Option<PathBuf>) -> eframe::Result {
    let (open_document_sender, open_document_receiver) = mpsc::channel();
    let (native_menu_sender, native_menu_receiver) = mpsc::channel();
    let event_loop =
        winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event().build()?;
    install_app_integration(open_document_sender, native_menu_sender);
    let mut application = eframe::create_native(
        "Prism",
        native_options(),
        Box::new(move |creation| {
            install_app_repaint(creation.egui_ctx.clone());
            Ok(Box::new(PrismApp::new(
                creation,
                initial_project.as_deref(),
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

fn shifted_key(key: &'static str) -> KeyEquivalent {
    KeyEquivalent {
        key,
        modifiers: NSEventModifierFlags::Command | NSEventModifierFlags::Shift,
    }
}

fn menu_item(
    marker: MainThreadMarker,
    title: &str,
    selector: Option<Sel>,
    equivalent: Option<KeyEquivalent>,
) -> Retained<NSMenuItem> {
    let key_equivalent = NSString::from_str(equivalent.map_or("", |value| value.key));
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            marker.alloc(),
            &NSString::from_str(title),
            selector,
            &key_equivalent,
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
    equivalent: Option<KeyEquivalent>,
) -> Retained<NSMenuItem> {
    let item = menu_item(
        marker,
        title,
        Some(sel!(performPrismMenuAction:)),
        equivalent,
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

fn install_main_menu(application: &NSApplication, target: &AnyObject, marker: MainThreadMarker) {
    let menu_bar = NSMenu::new(marker);

    let app_menu = submenu(marker, &menu_bar, "Prism");
    app_menu.addItem(&menu_item(
        marker,
        "About Prism",
        Some(sel!(orderFrontStandardAboutPanel:)),
        None,
    ));
    app_menu.addItem(&NSMenuItem::separatorItem(marker));
    let services_menu = NSMenu::new(marker);
    let services_item = menu_item(marker, "Services", None, None);
    services_item.setSubmenu(Some(&services_menu));
    app_menu.addItem(&services_item);
    app_menu.addItem(&NSMenuItem::separatorItem(marker));
    app_menu.addItem(&menu_item(
        marker,
        "Hide Prism",
        Some(sel!(hide:)),
        Some(key("h")),
    ));
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
        "Quit Prism",
        Some(sel!(terminate:)),
        Some(key("q")),
    ));
    application.setServicesMenu(Some(&services_menu));

    let file_menu = submenu(marker, &menu_bar, "File");
    file_menu.setAutoenablesItems(false);
    file_menu.addItem(&action_item(
        marker,
        target,
        "New Document",
        NativeMenuAction::NewDocument,
        Some(key("n")),
    ));
    file_menu.addItem(&action_item(
        marker,
        target,
        "Open…",
        NativeMenuAction::OpenDocument,
        Some(key("o")),
    ));
    file_menu.addItem(&NSMenuItem::separatorItem(marker));
    file_menu.addItem(&action_item(
        marker,
        target,
        "Move Project…",
        NativeMenuAction::MoveProject,
        None,
    ));
    file_menu.addItem(&action_item(
        marker,
        target,
        "Export…",
        NativeMenuAction::Export,
        Some(key("e")),
    ));

    let edit_menu = submenu(marker, &menu_bar, "Edit");
    edit_menu.setAutoenablesItems(false);
    edit_menu.addItem(&action_item(
        marker,
        target,
        "Undo",
        NativeMenuAction::Undo,
        Some(key("z")),
    ));
    edit_menu.addItem(&action_item(
        marker,
        target,
        "Redo",
        NativeMenuAction::Redo,
        Some(shifted_key("z")),
    ));
    edit_menu.addItem(&NSMenuItem::separatorItem(marker));
    edit_menu.addItem(&action_item(
        marker,
        target,
        "Cut",
        NativeMenuAction::Cut,
        Some(key("x")),
    ));
    edit_menu.addItem(&action_item(
        marker,
        target,
        "Copy",
        NativeMenuAction::Copy,
        Some(key("c")),
    ));
    edit_menu.addItem(&action_item(
        marker,
        target,
        "Paste",
        NativeMenuAction::Paste,
        Some(key("v")),
    ));

    let view_menu = submenu(marker, &menu_bar, "View");
    view_menu.setAutoenablesItems(false);
    view_menu.addItem(&action_item(
        marker,
        target,
        "Show History",
        NativeMenuAction::ToggleHistory,
        Some(shifted_key("h")),
    ));
    view_menu.addItem(&action_item(
        marker,
        target,
        "Show Terminal",
        NativeMenuAction::ToggleTerminal,
        Some(key("j")),
    ));
    view_menu.addItem(&NSMenuItem::separatorItem(marker));
    view_menu.addItem(&action_item(
        marker,
        target,
        "Fit Canvas",
        NativeMenuAction::FitCanvas,
        Some(key("0")),
    ));
    view_menu.addItem(&action_item(
        marker,
        target,
        "Zoom In",
        NativeMenuAction::ZoomIn,
        Some(key("+")),
    ));
    view_menu.addItem(&action_item(
        marker,
        target,
        "Zoom Out",
        NativeMenuAction::ZoomOut,
        Some(key("-")),
    ));

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

pub(super) fn update_menu_state(state: NativeMenuState) {
    let marker = MainThreadMarker::new().expect("Prism updates menus on the macOS main thread");
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

impl PrismApp {
    fn native_menu_state(&self, context: &egui::Context) -> NativeMenuState {
        let clipboard = self.layer_clipboard_state(context);
        NativeMenuState {
            modal_open: self.has_modal_surface(),
            workspace_ready: self.workspace_initialized,
            can_move_project: self.workspace.project_path.is_some(),
            can_undo: self.workspace.can_undo(),
            can_redo: self.workspace.can_redo(),
            history_visible: self.history.visible,
            terminal_visible: self.terminal.visible(),
            keyboard_focus: clipboard.keyboard_focus,
            selection_present: clipboard.selection_present,
            interaction_active: clipboard.interaction_active,
        }
    }

    pub(super) fn process_native_menu_actions(&mut self, context: &egui::Context) {
        let actions: Vec<_> = self.native_menu.receiver.try_iter().collect();
        for action in actions {
            if !self.native_menu_state(context).allows(action) {
                continue;
            }
            match action {
                NativeMenuAction::NewDocument => {
                    self.new_dialog = Some(NewDocumentDialog::default());
                }
                NativeMenuAction::OpenDocument => self.open_project_dialog(),
                NativeMenuAction::MoveProject => self.begin_move_project(),
                NativeMenuAction::Export => self.export(),
                NativeMenuAction::Undo => {
                    self.execute(Command::Undo);
                }
                NativeMenuAction::Redo => {
                    self.execute(Command::Redo);
                }
                action @ (NativeMenuAction::Cut
                | NativeMenuAction::Copy
                | NativeMenuAction::Paste) => {
                    context.send_viewport_cmd(
                        clipboard_viewport_command(action)
                            .expect("matched actions always have a clipboard command"),
                    );
                }
                NativeMenuAction::ToggleHistory => self.toggle_history(),
                NativeMenuAction::ToggleTerminal => self.toggle_terminal(),
                NativeMenuAction::FitCanvas => {
                    self.zoom = 1.0;
                    self.pan = Vec2::ZERO;
                    self.fit_requested = false;
                }
                NativeMenuAction::ZoomIn => {
                    self.zoom = (self.zoom * 1.25).clamp(0.1, 16.0);
                    self.fit_requested = false;
                }
                NativeMenuAction::ZoomOut => {
                    self.zoom = (self.zoom / 1.25).clamp(0.1, 16.0);
                    self.fit_requested = false;
                }
            }
            context.request_repaint();
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
    fn unfocused_modal_surfaces_disable_every_prism_menu_action() {
        let state = NativeMenuState {
            modal_open: true,
            workspace_ready: true,
            can_move_project: true,
            can_undo: true,
            can_redo: true,
            ..Default::default()
        };
        assert!(
            NativeMenuAction::ALL
                .into_iter()
                .all(|action| !state.allows(action))
        );
    }

    #[test]
    fn edit_and_canvas_actions_follow_live_document_state() {
        let state = NativeMenuState {
            workspace_ready: true,
            selection_present: true,
            can_undo: true,
            can_redo: false,
            history_visible: true,
            ..Default::default()
        };
        assert!(state.allows(NativeMenuAction::Undo));
        assert!(!state.allows(NativeMenuAction::Redo));
        assert!(!state.allows(NativeMenuAction::FitCanvas));
        assert!(state.allows(NativeMenuAction::ToggleHistory));
        assert!(!state.allows(NativeMenuAction::Copy));
        assert_eq!(
            state.title(NativeMenuAction::ToggleHistory),
            Some("Hide History")
        );
    }

    #[test]
    fn only_new_and_open_are_available_before_workspace_startup() {
        let state = NativeMenuState::default();
        assert!(state.allows(NativeMenuAction::NewDocument));
        assert!(state.allows(NativeMenuAction::OpenDocument));
        assert!(NativeMenuAction::ALL.into_iter().all(|action| matches!(
            action,
            NativeMenuAction::NewDocument | NativeMenuAction::OpenDocument
        ) || !state.allows(action)));
    }

    #[test]
    fn queued_actions_wake_the_ui_after_successful_delivery() {
        let (sender, receiver) = mpsc::channel();
        let mut woke = false;
        assert!(queue_and_wake(&sender, NativeMenuAction::Export, || woke = true));
        assert!(woke);
        assert_eq!(receiver.try_recv(), Ok(NativeMenuAction::Export));
    }

    #[test]
    fn focused_editors_and_terminal_keep_undo_shortcuts() {
        let focused = NativeMenuState {
            workspace_ready: true,
            can_undo: true,
            keyboard_focus: true,
            ..Default::default()
        };
        let terminal = NativeMenuState {
            workspace_ready: true,
            can_undo: true,
            terminal_visible: true,
            ..Default::default()
        };
        assert!(!focused.allows(NativeMenuAction::Undo));
        assert!(!terminal.allows(NativeMenuAction::Undo));
        assert!(terminal.allows(NativeMenuAction::ToggleTerminal));
        assert!(focused.allows(NativeMenuAction::Cut));
        assert!(focused.allows(NativeMenuAction::Copy));
        assert!(focused.allows(NativeMenuAction::Paste));
        assert!(terminal.allows(NativeMenuAction::Cut));
        assert!(terminal.allows(NativeMenuAction::Copy));
        assert!(terminal.allows(NativeMenuAction::Paste));
    }

    #[test]
    fn canvas_clipboard_menu_state_requires_selection_and_an_idle_canvas() {
        let available = NativeMenuState {
            workspace_ready: true,
            selection_present: true,
            ..Default::default()
        };
        assert!(available.allows(NativeMenuAction::Cut));
        assert!(available.allows(NativeMenuAction::Copy));
        assert!(available.allows(NativeMenuAction::Paste));

        let no_selection = NativeMenuState {
            selection_present: false,
            ..available
        };
        assert!(!no_selection.allows(NativeMenuAction::Cut));
        assert!(!no_selection.allows(NativeMenuAction::Copy));
        assert!(no_selection.allows(NativeMenuAction::Paste));

        for unavailable in [
            NativeMenuState {
                modal_open: true,
                ..available
            },
            NativeMenuState {
                history_visible: true,
                ..available
            },
            NativeMenuState {
                interaction_active: true,
                ..available
            },
        ] {
            assert!(!unavailable.allows(NativeMenuAction::Cut));
            assert!(!unavailable.allows(NativeMenuAction::Copy));
            assert!(!unavailable.allows(NativeMenuAction::Paste));
        }
    }

    #[test]
    fn native_clipboard_actions_request_the_matching_egui_event() {
        assert!(matches!(
            clipboard_viewport_command(NativeMenuAction::Cut),
            Some(egui::ViewportCommand::RequestCut)
        ));
        assert!(matches!(
            clipboard_viewport_command(NativeMenuAction::Copy),
            Some(egui::ViewportCommand::RequestCopy)
        ));
        assert!(matches!(
            clipboard_viewport_command(NativeMenuAction::Paste),
            Some(egui::ViewportCommand::RequestPaste)
        ));
        assert!(clipboard_viewport_command(NativeMenuAction::Undo).is_none());
    }
}
