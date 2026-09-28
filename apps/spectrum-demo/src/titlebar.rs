//! Window moving from our own title areas. On macOS the top band of a
//! transparent title bar belongs to the system: a press there moves the
//! window and a quick second click zooms it, even over our buttons. The view
//! opts out of that, and only drag areas move the window.
use gpui::{MouseDownEvent, Window};

/// Call once on the new window.
pub fn install(window: &Window) {
    #[cfg(target_os = "macos")]
    mac::install(window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

/// A press in a drag area. On macOS this moves the window, or zooms it on a
/// double-click, and returns true; elsewhere the caller waits for movement.
pub fn press(event: &MouseDownEvent, window: &mut Window) -> bool {
    if event.click_count >= 2 {
        if cfg!(target_os = "macos") {
            window.titlebar_double_click();
        } else {
            window.zoom_window();
        }
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        mac::drag();
        true
    }
    #[cfg(not(target_os = "macos"))]
    false
}

#[cfg(target_os = "macos")]
mod mac {
    use gpui::Window;
    use objc::{
        class, msg_send,
        runtime::{BOOL, NO, Object, Sel, class_addMethod, object_getClass},
        sel, sel_impl,
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    extern "C" fn cannot_move_window(_: &Object, _: Sel) -> BOOL {
        NO
    }

    pub fn install(window: &Window) {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return;
        };
        unsafe {
            let view = handle.ns_view.as_ptr() as *const Object;
            let class = object_getClass(view) as *mut _;
            let imp: extern "C" fn(&Object, Sel) -> BOOL = cannot_move_window;
            class_addMethod(
                class,
                sel!(mouseDownCanMoveWindow),
                std::mem::transmute::<extern "C" fn(&Object, Sel) -> BOOL, objc::runtime::Imp>(imp),
                c"c@:".as_ptr(),
            );
        }
    }

    /// Hands the mouse-down being handled to AppKit's window drag.
    pub fn drag() {
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let event: *mut Object = msg_send![app, currentEvent];
            if event.is_null() {
                return;
            }
            let window: *mut Object = msg_send![event, window];
            if !window.is_null() {
                let _: () = msg_send![window, performWindowDragWithEvent: event];
            }
        }
    }
}
