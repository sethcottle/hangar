// SPDX-License-Identifier: MPL-2.0

//! Browser and clipboard hand-offs, each with a toast.

use crate::ui::HangarWindow;
use gtk4::prelude::*;
use libadwaita as adw;

/// Matches `HangarWindow::show_toast` so a toast reads the same however it was
/// raised.
const TOAST_TIMEOUT: u32 = 3;

/// Matches `HangarWindow::show_toast_with_action`.
const ACTION_TOAST_TIMEOUT: u32 = 5;

/// The toast overlay nearest `widget`.
///
/// Nearest, not the window's own: an AdwDialog draws over the window content,
/// so a toast added to the window while a dialog is up comes up behind it.
fn nearest_overlay(widget: &gtk4::Widget) -> Option<adw::ToastOverlay> {
    widget
        .ancestor(adw::ToastOverlay::static_type())
        .and_then(|w| w.downcast::<adw::ToastOverlay>().ok())
}

/// Show `message` on the toast overlay nearest `widget`.
pub fn toast(widget: &impl IsA<gtk4::Widget>, message: &str) {
    let widget = widget.as_ref();
    if let Some(overlay) = nearest_overlay(widget) {
        let toast = adw::Toast::new(message);
        toast.set_timeout(TOAST_TIMEOUT);
        overlay.add_toast(toast);
    } else if let Some(window) = widget
        .root()
        .and_then(|r| r.downcast::<HangarWindow>().ok())
    {
        window.show_toast(message);
    }
}

/// Show `message` with an action button. Same overlay rules as [`toast`].
pub fn toast_with_action(
    widget: &impl IsA<gtk4::Widget>,
    message: &str,
    button_label: &str,
    action: impl Fn() + 'static,
) {
    let widget = widget.as_ref();
    if let Some(overlay) = nearest_overlay(widget) {
        let toast = adw::Toast::new(message);
        toast.set_timeout(ACTION_TOAST_TIMEOUT);
        toast.set_button_label(Some(button_label));
        toast.connect_button_clicked(move |_| action());
        overlay.add_toast(toast);
    } else if let Some(window) = widget
        .root()
        .and_then(|r| r.downcast::<HangarWindow>().ok())
    {
        window.show_toast_with_action(message, button_label, action);
    }
}

/// Open `url` in the user's browser and report which way it went.
///
/// `what` names the destination: "post" reads as "Opened post in your browser".
pub fn open_url(widget: &impl IsA<gtk4::Widget>, url: &str, what: &str) {
    match open::that(url) {
        Ok(()) => toast(widget, &format!("Opened {what} in your browser")),
        // Otherwise no browser installed and a dead link look identical.
        Err(_) => toast(widget, &format!("Couldn't open {what} in your browser")),
    }
}

/// Copy `text` to the clipboard and report it.
///
/// `what` names what was copied, capitalised: "Link to post" reads as
/// "Link to post copied to clipboard".
pub fn copy_text(widget: &impl IsA<gtk4::Widget>, text: &str, what: &str) {
    widget.as_ref().display().clipboard().set_text(text);
    toast(widget, &format!("{what} copied to clipboard"));
}
