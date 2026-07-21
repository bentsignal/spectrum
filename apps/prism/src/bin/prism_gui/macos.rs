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
use crate::shortcuts::canvas_interaction_active;

static OPEN_DOCUMENT_SENDER: OnceLock<Sender<PathBuf>> = OnceLock::new();
static NATIVE_MENU_SENDER: OnceLock<Sender<NativeMenuAction>> = OnceLock::new();
static APP_REPAINT: OnceLock<egui::Context> = OnceLock::new();

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
    pub(super) pixel_selection_present: bool,
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
        match action {
            NativeMenuAction::Cut | NativeMenuAction::Copy | NativeMenuAction::Paste => {
                self.keyboard_focus
                    || self.terminal_visible
                    || !self.modal_open
                        && self.workspace_ready
                        && !self.history_visible
                        && !self.interaction_active
                        && (action == NativeMenuAction::Paste || self.selection_present)
            }
            NativeMenuAction::NewDocument | NativeMenuAction::OpenDocument => !self.modal_open,
            NativeMenuAction::MoveProject => {
                !self.modal_open && self.workspace_ready && self.can_move_project
            }
            NativeMenuAction::Export | NativeMenuAction::ToggleTerminal => {
                !self.modal_open && self.workspace_ready
            }
            NativeMenuAction::ToggleHistory => {
                !self.modal_open
                    && self.workspace_ready
                    && !self.keyboard_focus
                    && !self.terminal_visible
            }
            NativeMenuAction::Undo => {
                !self.modal_open
                    && self.workspace_ready
                    && self.can_undo
                    && !self.keyboard_focus
                    && !self.terminal_visible
            }
            NativeMenuAction::Redo => {
                !self.modal_open
                    && self.workspace_ready
                    && self.can_redo
                    && !self.keyboard_focus
                    && !self.terminal_visible
            }
            NativeMenuAction::SelectAll | NativeMenuAction::Deselect => {
                !self.modal_open
                    && self.workspace_ready
                    && !self.keyboard_focus
                    && !self.terminal_visible
                    && !self.history_visible
                    && !self.interaction_active
                    && (action == NativeMenuAction::SelectAll || self.pixel_selection_present)
            }
            NativeMenuAction::FitCanvas | NativeMenuAction::ZoomIn | NativeMenuAction::ZoomOut => {
                !self.modal_open
                    && self.workspace_ready
                    && !self.history_visible
                    && !self.terminal_visible
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
            NativeMenuAction::Cut
            | NativeMenuAction::Copy
            | NativeMenuAction::Paste
            | NativeMenuAction::SelectAll
            | NativeMenuAction::Deselect => None,
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
    let mut event_loop_builder =
        winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event();
    // winit otherwise replaces this process-wide menu in applicationDidFinishLaunching,
    // after `install_app_integration` has installed Prism's File/Edit/View menus.
    event_loop_builder.with_default_menu(false);
    let event_loop = event_loop_builder.build()?;
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
    equivalent: Option<ActionKeyEquivalent>,
) -> Retained<NSMenuItem> {
    let item = menu_item(
        marker,
        title,
        Some(sel!(performPrismMenuAction:)),
        equivalent.map(Into::into),
    );
    item.setTag(action as isize);
    unsafe { item.setTarget(Some(target)) };
    item
}

fn action_submenu(
    marker: MainThreadMarker,
    menu_bar: &NSMenu,
    target: &AnyObject,
    section: NativeMenuSection,
) -> Retained<NSMenu> {
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
    menu
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
            pixel_selection_present: self.workspace.document.selection.is_some(),
            interaction_active: canvas_interaction_active(
                self.drag.is_some(),
                clipboard.interaction_active,
            ),
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
                NativeMenuAction::SelectAll => self.select_all_pixels(),
                NativeMenuAction::Deselect => self.deselect_pixels(),
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
        assert!(!state.allows(NativeMenuAction::SelectAll));
        assert!(!state.allows(NativeMenuAction::Deselect));
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
    fn export_and_move_project_follow_workspace_and_path_state() {
        let untitled = NativeMenuState {
            workspace_ready: true,
            ..Default::default()
        };
        assert!(untitled.allows(NativeMenuAction::Export));
        assert!(!untitled.allows(NativeMenuAction::MoveProject));

        let saved = NativeMenuState {
            can_move_project: true,
            ..untitled
        };
        assert!(saved.allows(NativeMenuAction::Export));
        assert!(saved.allows(NativeMenuAction::MoveProject));

        let modal = NativeMenuState {
            modal_open: true,
            ..saved
        };
        assert!(!modal.allows(NativeMenuAction::Export));
        assert!(!modal.allows(NativeMenuAction::MoveProject));
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
    fn focused_text_and_terminal_own_clipboard_but_not_prism_history() {
        let focused = NativeMenuState {
            workspace_ready: true,
            can_undo: true,
            can_redo: true,
            keyboard_focus: true,
            ..Default::default()
        };
        let terminal = NativeMenuState {
            workspace_ready: true,
            can_undo: true,
            can_redo: true,
            terminal_visible: true,
            ..Default::default()
        };
        assert!(!focused.allows(NativeMenuAction::Undo));
        assert!(!focused.allows(NativeMenuAction::Redo));
        assert!(!terminal.allows(NativeMenuAction::Undo));
        assert!(!terminal.allows(NativeMenuAction::Redo));
        assert!(terminal.allows(NativeMenuAction::ToggleTerminal));
        assert!(focused.allows(NativeMenuAction::Cut));
        assert!(focused.allows(NativeMenuAction::Copy));
        assert!(focused.allows(NativeMenuAction::Paste));
        assert!(terminal.allows(NativeMenuAction::Cut));
        assert!(terminal.allows(NativeMenuAction::Copy));
        assert!(terminal.allows(NativeMenuAction::Paste));
        assert!(!focused.allows(NativeMenuAction::SelectAll));
        assert!(!focused.allows(NativeMenuAction::Deselect));
        assert!(!terminal.allows(NativeMenuAction::SelectAll));
        assert!(!terminal.allows(NativeMenuAction::Deselect));
    }

    #[test]
    fn selection_menu_actions_require_an_idle_canvas_and_deselect_requires_a_marquee() {
        let selected = NativeMenuState {
            workspace_ready: true,
            pixel_selection_present: true,
            ..Default::default()
        };
        assert!(selected.allows(NativeMenuAction::SelectAll));
        assert!(selected.allows(NativeMenuAction::Deselect));

        let empty = NativeMenuState {
            pixel_selection_present: false,
            ..selected
        };
        assert!(empty.allows(NativeMenuAction::SelectAll));
        assert!(!empty.allows(NativeMenuAction::Deselect));

        for unavailable in [
            NativeMenuState {
                history_visible: true,
                ..selected
            },
            NativeMenuState {
                interaction_active: true,
                ..selected
            },
        ] {
            assert!(!unavailable.allows(NativeMenuAction::SelectAll));
            assert!(!unavailable.allows(NativeMenuAction::Deselect));
        }
    }

    #[test]
    fn draw_drag_disables_native_selection_actions_before_a_workspace_interaction_starts() {
        let state = NativeMenuState {
            workspace_ready: true,
            pixel_selection_present: true,
            interaction_active: canvas_interaction_active(true, false),
            ..Default::default()
        };
        assert!(!state.allows(NativeMenuAction::SelectAll));
        assert!(!state.allows(NativeMenuAction::Deselect));
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

    #[test]
    fn dynamic_panel_titles_follow_visibility_and_clipboard_titles_stay_static() {
        let visible = NativeMenuState {
            history_visible: true,
            terminal_visible: true,
            ..Default::default()
        };
        assert_eq!(
            visible.title(NativeMenuAction::ToggleHistory),
            Some("Hide History")
        );
        assert_eq!(
            visible.title(NativeMenuAction::ToggleTerminal),
            Some("Hide Terminal")
        );
        let hidden = NativeMenuState::default();
        assert_eq!(
            hidden.title(NativeMenuAction::ToggleHistory),
            Some("Show History")
        );
        assert_eq!(
            hidden.title(NativeMenuAction::ToggleTerminal),
            Some("Show Terminal")
        );
        for action in [
            NativeMenuAction::Cut,
            NativeMenuAction::Copy,
            NativeMenuAction::Paste,
        ] {
            assert_eq!(hidden.title(action), None);
        }
    }
}
