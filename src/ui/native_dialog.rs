use std::cell::Cell;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    WindowHandle,
};

#[derive(Clone, Copy)]
struct DialogParent {
    window: RawWindowHandle,
    display: RawDisplayHandle,
}

impl HasWindowHandle for DialogParent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: The raw handle comes from the live eframe main window and this
        // private wrapper is used only while processing that window's UI frame.
        Ok(unsafe { WindowHandle::borrow_raw(self.window) })
    }
}

impl HasDisplayHandle for DialogParent {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        // SAFETY: The display handle has the same lifetime and usage boundary as
        // the associated live eframe main window handle above.
        Ok(unsafe { DisplayHandle::borrow_raw(self.display) })
    }
}

thread_local! {
    static DIALOG_PARENT: Cell<Option<DialogParent>> = const { Cell::new(None) };
}

/// Register the main application window as the modal parent for native dialogs.
pub fn set_native_dialog_parent(frame: &eframe::Frame) {
    let parent = frame
        .window_handle()
        .ok()
        .zip(frame.display_handle().ok())
        .map(|(window, display)| DialogParent {
            window: window.as_raw(),
            display: display.as_raw(),
        });
    DIALOG_PARENT.set(parent);
}

pub fn file_dialog() -> rfd::FileDialog {
    DIALOG_PARENT.with(|parent| {
        let dialog = rfd::FileDialog::new();
        match parent.get() {
            Some(parent) => dialog.set_parent(&parent),
            None => dialog,
        }
    })
}

pub fn message_dialog() -> rfd::MessageDialog {
    DIALOG_PARENT.with(|parent| {
        let dialog = rfd::MessageDialog::new();
        match parent.get() {
            Some(parent) => dialog.set_parent(&parent),
            None => dialog,
        }
    })
}
