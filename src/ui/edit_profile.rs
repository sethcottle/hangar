// SPDX-License-Identifier: MPL-2.0

//! The Edit Profile dialog: display name, bio, avatar, banner.
//!
//! Text fields prefill from the shown profile and always save; images only
//! ride along when a new one was picked, so an untouched avatar or banner
//! never re-uploads. The lexicon's grapheme limits gate the Save button
//! rather than letting the server bounce the write.

use crate::atproto::Profile;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

/// Lexicon limits for app.bsky.actor.profile.
const MAX_NAME_GRAPHEMES: usize = 64;
const MAX_BIO_GRAPHEMES: usize = 256;

/// A picked image: bytes plus mime type, shared with the save closure.
type PickedImage = Rc<RefCell<Option<(Vec<u8>, String)>>>;

/// What Save hands back. Images are bytes plus mime type.
pub struct ProfileEdits {
    pub display_name: String,
    pub description: String,
    pub avatar: Option<(Vec<u8>, String)>,
    pub banner: Option<(Vec<u8>, String)>,
}

/// The pieces a test needs to drive the dialog without a pointer.
pub(crate) struct EditProfileParts {
    pub dialog: adw::Dialog,
    pub name_entry: gtk4::Entry,
    pub bio_view: gtk4::TextView,
    pub save_button: gtk4::Button,
    pub picked_avatar: PickedImage,
    pub picked_banner: PickedImage,
}

/// Mime type from a path's extension, matching what compose attaches.
fn mime_for(path: &std::path::Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("png") => "image/png".to_string(),
        Some(ext) if ext.eq_ignore_ascii_case("gif") => "image/gif".to_string(),
        Some(ext) if ext.eq_ignore_ascii_case("webp") => "image/webp".to_string(),
        _ => "image/jpeg".to_string(),
    }
}

/// A picker that reads the chosen image and hands bytes, mime, and a
/// texture for the preview to `picked`.
fn pick_image(
    parent: &gtk4::Widget,
    picked: impl Fn(Vec<u8>, String, gtk4::gdk::Texture) + 'static,
) {
    let filter = gtk4::FileFilter::new();
    filter.set_name(Some("Images"));
    filter.add_mime_type("image/jpeg");
    filter.add_mime_type("image/png");
    filter.add_mime_type("image/gif");
    filter.add_mime_type("image/webp");
    let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
    filters.append(&filter);

    let dialog = gtk4::FileDialog::builder()
        .title("Choose an Image")
        .filters(&filters)
        .build();

    let window = parent
        .root()
        .and_then(|root| root.downcast::<gtk4::Window>().ok());
    dialog.open(
        window.as_ref(),
        None::<&gtk4::gio::Cancellable>,
        move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(path) = file.path() else {
                return;
            };
            let Ok(data) = std::fs::read(&path) else {
                return;
            };
            let Ok(texture) = gtk4::gdk::Texture::from_filename(&path) else {
                return;
            };
            picked(data, mime_for(&path), texture);
        },
    );
}

pub(crate) fn build(profile: &Profile) -> EditProfileParts {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let header = adw::HeaderBar::new();
    let title = gtk4::Label::new(Some("Edit Profile"));
    title.add_css_class("title");
    header.set_title_widget(Some(&title));
    let save_button = gtk4::Button::with_label("Save");
    save_button.add_css_class("suggested-action");
    header.pack_end(&save_button);
    content.append(&header);

    // Banner on top with the avatar straddling its edge, the same shape
    // the profile pages draw.
    let banner_frame = gtk4::Frame::new(None);
    banner_frame.set_overflow(gtk4::Overflow::Hidden);
    banner_frame.add_css_class("profile-banner");
    banner_frame.set_size_request(-1, 100);
    let banner_picture = gtk4::Picture::new();
    banner_picture.set_hexpand(true);
    banner_picture.set_vexpand(true);
    banner_picture.set_can_shrink(true);
    banner_picture.set_content_fit(gtk4::ContentFit::Cover);
    banner_frame.set_child(Some(&banner_picture));
    if let Some(url) = &profile.banner {
        crate::ui::avatar_cache::load_image_into_picture(banner_picture.clone(), url.clone());
    }

    let banner_button = gtk4::Button::new();
    banner_button.set_child(Some(&banner_frame));
    banner_button.add_css_class("flat");
    banner_button.set_tooltip_text(Some("Change banner"));
    banner_button.update_property(&[gtk4::accessible::Property::Label("Change banner")]);

    let avatar = adw::Avatar::new(72, profile.display_name.as_deref(), true);
    if let Some(url) = &profile.avatar {
        crate::ui::avatar_cache::load_avatar(avatar.clone(), url.clone());
    }
    let avatar_button = gtk4::Button::new();
    avatar_button.set_child(Some(&avatar));
    avatar_button.add_css_class("circular");
    avatar_button.add_css_class("flat");
    avatar_button.set_halign(gtk4::Align::Center);
    avatar_button.set_valign(gtk4::Align::Start);
    avatar_button.set_margin_top(60);
    avatar_button.set_tooltip_text(Some("Change avatar"));
    avatar_button.update_property(&[gtk4::accessible::Property::Label("Change avatar")]);

    let images_overlay = gtk4::Overlay::new();
    images_overlay.set_child(Some(&banner_button));
    images_overlay.add_overlay(&avatar_button);
    // The avatar hangs below the banner; give it room.
    images_overlay.set_margin_bottom(36);
    content.append(&images_overlay);

    let picked_avatar: PickedImage = Rc::new(RefCell::new(None));
    let picked_banner: PickedImage = Rc::new(RefCell::new(None));

    let picked = picked_avatar.clone();
    let avatar_ref = avatar.clone();
    avatar_button.connect_clicked(move |btn| {
        let picked = picked.clone();
        let avatar_ref = avatar_ref.clone();
        pick_image(btn.upcast_ref(), move |data, mime, texture| {
            avatar_ref.set_custom_image(Some(&texture));
            picked.replace(Some((data, mime)));
        });
    });

    let picked = picked_banner.clone();
    let banner_ref = banner_picture.clone();
    banner_button.connect_clicked(move |btn| {
        let picked = picked.clone();
        let banner_ref = banner_ref.clone();
        pick_image(btn.upcast_ref(), move |data, mime, texture| {
            banner_ref.set_paintable(Some(&texture));
            picked.replace(Some((data, mime)));
        });
    });

    // Name and bio, prefilled.
    let form = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    form.set_margin_start(16);
    form.set_margin_end(16);
    form.set_margin_bottom(16);

    let name_label = gtk4::Label::new(Some("Display Name"));
    name_label.add_css_class("heading");
    name_label.set_halign(gtk4::Align::Start);
    form.append(&name_label);

    let name_entry = gtk4::Entry::new();
    name_entry.set_text(profile.display_name.as_deref().unwrap_or(""));
    form.append(&name_entry);

    let bio_label = gtk4::Label::new(Some("Description"));
    bio_label.add_css_class("heading");
    bio_label.set_halign(gtk4::Align::Start);
    bio_label.set_margin_top(8);
    form.append(&bio_label);

    let bio_frame = gtk4::Frame::new(None);
    let bio_view = gtk4::TextView::new();
    bio_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    bio_view.set_top_margin(8);
    bio_view.set_bottom_margin(8);
    bio_view.set_left_margin(8);
    bio_view.set_right_margin(8);
    bio_view.set_size_request(-1, 96);
    bio_view
        .buffer()
        .set_text(profile.description.as_deref().unwrap_or(""));
    bio_frame.set_child(Some(&bio_view));
    form.append(&bio_frame);

    content.append(&form);

    // The lexicon limits gate Save instead of the server bouncing it.
    let gate = {
        let name_entry = name_entry.clone();
        let bio_view = bio_view.clone();
        let save_button = save_button.clone();
        move || {
            let name_ok = name_entry.text().graphemes(true).count() <= MAX_NAME_GRAPHEMES;
            let buffer = bio_view.buffer();
            let bio = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let bio_ok = bio.graphemes(true).count() <= MAX_BIO_GRAPHEMES;
            save_button.set_sensitive(name_ok && bio_ok);
            save_button.set_tooltip_text(if name_ok && bio_ok {
                None
            } else {
                Some("Too long: names cap at 64 characters, bios at 256")
            });
        }
    };
    let g = gate.clone();
    name_entry.connect_changed(move |_| g());
    let g = gate.clone();
    bio_view.buffer().connect_changed(move |_| g());
    gate();

    let dialog = adw::Dialog::builder()
        .title("Edit Profile")
        .content_width(420)
        .child(&content)
        .build();

    EditProfileParts {
        dialog,
        name_entry,
        bio_view,
        save_button,
        picked_avatar,
        picked_banner,
    }
}

/// Show the dialog over `parent`; Save hands the edits over and closes.
pub fn present(
    parent: &impl IsA<gtk4::Widget>,
    profile: &Profile,
    on_save: impl Fn(ProfileEdits) + 'static,
) {
    let parts = build(profile);
    let dialog = parts.dialog.clone();
    let name_entry = parts.name_entry.clone();
    let bio_view = parts.bio_view.clone();
    let picked_avatar = parts.picked_avatar.clone();
    let picked_banner = parts.picked_banner.clone();

    parts.save_button.connect_clicked(move |_| {
        let buffer = bio_view.buffer();
        let description = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .trim()
            .to_string();
        let edits = ProfileEdits {
            display_name: name_entry.text().trim().to_string(),
            description,
            avatar: picked_avatar.borrow_mut().take(),
            banner: picked_banner.borrow_mut().take(),
        };
        on_save(edits);
        dialog.close();
    });

    parts.dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prefills come from the profile, edits come back out, and the
    /// grapheme limits gate Save.
    #[test]
    fn the_dialog_prefills_and_gates_on_limits() {
        crate::ui::with_gtk(the_dialog_prefills_and_gates_on_limits_body);
    }

    fn the_dialog_prefills_and_gates_on_limits_body() {
        let mut profile = Profile::minimal(
            "did:plc:me".into(),
            "me.bsky.social".into(),
            Some("Old Name".into()),
            None,
        );
        profile.description = Some("Old bio".into());

        let parts = build(&profile);
        assert_eq!(parts.name_entry.text(), "Old Name");
        let buffer = parts.bio_view.buffer();
        assert_eq!(
            buffer.text(&buffer.start_iter(), &buffer.end_iter(), false),
            "Old bio"
        );
        assert!(parts.save_button.is_sensitive());

        parts.name_entry.set_text(&"x".repeat(65));
        assert!(
            !parts.save_button.is_sensitive(),
            "a 65 grapheme name cannot save"
        );
        parts.name_entry.set_text("New Name");
        assert!(parts.save_button.is_sensitive());

        buffer.set_text(&"b".repeat(257));
        assert!(
            !parts.save_button.is_sensitive(),
            "a 257 grapheme bio cannot save"
        );
        buffer.set_text("New bio");
        assert!(parts.save_button.is_sensitive());

        assert!(
            parts.picked_avatar.borrow().is_none() && parts.picked_banner.borrow().is_none(),
            "untouched images stay home"
        );
    }
}
