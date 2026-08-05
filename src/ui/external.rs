// SPDX-License-Identifier: MPL-2.0

//! Browser and clipboard hand-offs, each with a toast.

use crate::ui::HangarWindow;
use gtk4::prelude::*;
use libadwaita as adw;
use url::Url;

/// Matches `HangarWindow::show_toast` so a toast reads the same however it was
/// raised.
const TOAST_TIMEOUT: u32 = 3;

/// Matches `HangarWindow::show_toast_with_action`.
const ACTION_TOAST_TIMEOUT: u32 = 5;

/// The toast overlay nearest `widget`.
///
/// Uses the nearest overlay because an AdwDialog draws over the window
/// content, so a toast added to the window while a dialog is up comes up
/// behind it.
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

/// Whether a uri is one Hangar will hand to the desktop.
///
/// Link cards carry a `uri` copied verbatim off the wire, and a click would
/// otherwise run the host's handler for whatever scheme it names with an
/// attacker-chosen argument: `steam://`, `apt://`, an `smb://` mount against
/// their server, `file:///home/you/.ssh/id_rsa` in your editor. Same rule
/// [`crate::atproto::gif`] applies to the uris it hands GStreamer.
fn is_web_uri(uri: &str) -> bool {
    Url::parse(uri).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

/// Open `url` in the user's browser and report which way it went.
///
/// Anything that is not http or https is not dispatched at all: the uri is
/// shown as-is with the offer to copy it, so the person can decide.
///
/// `what` names the destination: "post" reads as "Opened post in your browser".
///
/// The launch is asynchronous. The blocking `open::that` waits on the launcher
/// child, and on desktops xdg-open treats as generic that child is the browser
/// itself, so a cold start froze the whole window until the browser exited.
pub fn open_url(widget: &impl IsA<gtk4::Widget>, url: &str, what: &str) {
    let widget = widget.as_ref();

    if !is_web_uri(url) {
        let weak = widget.downgrade();
        let uri = url.to_string();
        // A toast title is Pango markup, and a raw uri is full of ampersands.
        let shown = gtk4::glib::markup_escape_text(url);
        toast_with_action(
            widget,
            &format!("Hangar only opens web links. This one is {shown}"),
            "Copy",
            move || {
                if let Some(widget) = weak.upgrade() {
                    copy_text(&widget, &uri, "Link");
                }
            },
        );
        return;
    }

    // The parent gives the portal a real window handle, so the browser comes
    // up over Hangar rather than wherever the compositor guesses.
    let parent = widget
        .root()
        .and_then(|root| root.downcast::<gtk4::Window>().ok());
    let weak = widget.downgrade();
    let what = what.to_string();
    gtk4::UriLauncher::new(url).launch(
        parent.as_ref(),
        None::<&gtk4::gio::Cancellable>,
        move |result| {
            let Some(widget) = weak.upgrade() else {
                return;
            };
            match result {
                Ok(()) => toast(&widget, &format!("Opened {what} in your browser")),
                // Dismissing the app chooser is a decision, not a failure.
                Err(e) if e.matches(gtk4::DialogError::Dismissed) => {}
                // Otherwise no browser installed and a dead link look identical.
                Err(_) => toast(&widget, &format!("Couldn't open {what} in your browser")),
            }
        },
    );
}

/// Copy `text` to the clipboard and report it.
///
/// `what` names what was copied, capitalised: "Link to post" reads as
/// "Link to post copied to clipboard".
pub fn copy_text(widget: &impl IsA<gtk4::Widget>, text: &str, what: &str) {
    widget.as_ref().display().clipboard().set_text(text);
    toast(widget, &format!("{what} copied to clipboard"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_card_can_only_send_the_desktop_a_web_link() {
        // Every one of these is reachable from an ExternalEmbed.uri, which is
        // a bare string off the wire. open_url shows them instead of
        // dispatching them.
        for hostile in [
            "file:///home/you/.ssh/id_rsa",
            "smb://attacker.example/share",
            "sftp://attacker.example/",
            "davs://attacker.example/",
            "steam://run/440",
            "apt://anything",
            "zoommtg://zoom.us/join?confno=1",
            "obsidian://open?vault=x",
            "javascript:alert(1)",
            "data:text/html,<script>",
            "ms-msdt:/id",
            // No scheme at all is not a uri to hand anybody.
            "bsky.app/profile/alice",
            "",
        ] {
            assert!(!is_web_uri(hostile), "{hostile} should not be dispatched");
        }
    }

    #[test]
    fn ordinary_links_still_open() {
        for ok in [
            "https://bsky.app/profile/alice.bsky.social",
            "http://example.com/a?b=c#d",
            // The scheme is compared after url normalises its case.
            "HTTPS://example.com/",
        ] {
            assert!(is_web_uri(ok), "{ok} should open");
        }
    }
}
