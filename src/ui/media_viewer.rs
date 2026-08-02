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

/// Play a video embed in a dialog.
///
/// Bluesky serves video as an HLS playlist. Handing that URL to the browser is
/// what produced a page of raw playlist text rather than a video, so play it
/// here instead. GStreamer does the decoding, and HLS support lives in
/// gst-plugins-bad, which is not guaranteed to be installed. When the pipeline
/// reports an error the dialog closes and the post opens on the web, so the
/// video is still reachable rather than silently doing nothing.
pub fn show_video(parent: &impl IsA<gtk4::Widget>, playlist_url: String, fallback_url: String) {
    let root = match parent
        .as_ref()
        .root()
        .and_then(|r| r.downcast::<gtk4::Window>().ok())
    {
        Some(w) => w,
        None => return,
    };

    let dialog = adw::Dialog::new();
    dialog.set_title("Video");
    dialog.set_content_width(900);
    dialog.set_content_height(620);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());

    let media = gtk4::MediaFile::for_file(&gtk4::gio::File::for_uri(&playlist_url));
    let video = gtk4::Video::new();
    video.set_hexpand(true);
    video.set_vexpand(true);
    video.set_autoplay(true);
    video.set_media_stream(Some(&media));

    // MediaStream reports failures through the error property rather than by
    // returning one, so watch it and fall back to the browser.
    media.connect_error_notify({
        let dialog = dialog.clone();
        let fallback_url = fallback_url.clone();
        move |media| {
            if let Some(e) = media.error() {
                eprintln!("hangar: could not play video in app ({e}); opening in browser");
                dialog.close();
                let _ = open::that(&fallback_url);
            }
        }
    });

    toolbar.set_content(Some(&video));
    dialog.set_child(Some(&toolbar));

    // Stop decoding when the dialog goes away, otherwise audio keeps playing.
    dialog.connect_closed({
        let media = media.clone();
        move |_| media.set_playing(false)
    });

    dialog.present(Some(&root));
}

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
