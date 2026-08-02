// SPDX-License-Identifier: MPL-2.0

//! Full-size image viewer.
//!
//! Post embeds render thumbnails, which are small and often cropped. This
//! presents the fullsize variant in a dialog so a post's images can actually
//! be read, with the alt text alongside where the author supplied one.

use crate::atproto::ImageEmbed;
use crate::ui::avatar_cache;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

/// Present `images` at full size, starting on `start`.
///
/// `parent` only needs to be some widget inside the window; the dialog finds
/// its own root from there.
pub fn show_images(parent: &impl IsA<gtk4::Widget>, images: Vec<ImageEmbed>, start: usize) {
    if images.is_empty() {
        return;
    }

    let root = match parent
        .as_ref()
        .root()
        .and_then(|r| r.downcast::<gtk4::Window>().ok())
    {
        Some(w) => w,
        None => return,
    };

    let total = images.len();
    let index = Rc::new(Cell::new(start.min(total - 1)));
    let images = Rc::new(images);

    let dialog = adw::Dialog::new();
    dialog.set_title("Image");
    dialog.set_content_width(1000);
    dialog.set_content_height(760);

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar.add_top_bar(&header);

    let picture = gtk4::Picture::new();
    picture.set_content_fit(gtk4::ContentFit::Contain);
    picture.set_hexpand(true);
    picture.set_vexpand(true);

    // Alt text is frequently absent, so the label only takes space when the
    // author actually wrote one.
    let alt_label = gtk4::Label::new(None);
    alt_label.add_css_class("dim-label");
    alt_label.set_wrap(true);
    alt_label.set_justify(gtk4::Justification::Center);
    alt_label.set_margin_start(12);
    alt_label.set_margin_end(12);
    alt_label.set_margin_bottom(12);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    content.append(&picture);
    content.append(&alt_label);

    // Only paginate when there is something to paginate to.
    let prev_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
    let next_btn = gtk4::Button::from_icon_name("go-next-symbolic");
    if total > 1 {
        prev_btn.set_tooltip_text(Some("Previous image"));
        next_btn.set_tooltip_text(Some("Next image"));
        header.pack_start(&prev_btn);
        header.pack_end(&next_btn);
    }

    let apply = {
        let picture = picture.clone();
        let alt_label = alt_label.clone();
        let dialog = dialog.clone();
        let images = images.clone();
        let index = index.clone();
        move || {
            let i = index.get();
            let img = &images[i];
            avatar_cache::load_image_into_picture(picture.clone(), img.fullsize.clone());

            let alt = img.alt.trim();
            alt_label.set_visible(!alt.is_empty());
            alt_label.set_label(alt);

            dialog.set_title(&if images.len() > 1 {
                format!("Image {} of {}", i + 1, images.len())
            } else {
                "Image".to_string()
            });
        }
    };

    if total > 1 {
        // Wrap around: with at most four images, hitting an edge and stopping
        // is more annoying than useful.
        prev_btn.connect_clicked({
            let index = index.clone();
            let apply = apply.clone();
            move |_| {
                let i = index.get();
                index.set(if i == 0 { total - 1 } else { i - 1 });
                apply();
            }
        });
        next_btn.connect_clicked({
            let index = index.clone();
            let apply = apply.clone();
            move |_| {
                index.set((index.get() + 1) % total);
                apply();
            }
        });

        let key = gtk4::EventControllerKey::new();
        key.connect_key_pressed({
            let index = index.clone();
            let apply = apply.clone();
            move |_, keyval, _, _| match keyval {
                gtk4::gdk::Key::Left => {
                    let i = index.get();
                    index.set(if i == 0 { total - 1 } else { i - 1 });
                    apply();
                    glib::Propagation::Stop
                }
                gtk4::gdk::Key::Right => {
                    index.set((index.get() + 1) % total);
                    apply();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        dialog.add_controller(key);
    }

    apply();

    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(&root));
}
