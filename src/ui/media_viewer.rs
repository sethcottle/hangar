// SPDX-License-Identifier: MPL-2.0

//! Full-size image and video viewers.
//!
//! Post embeds render the thumbnail, which is small and usually cropped. This
//! shows the fullsize variant in a dialog, with the alt text where the author
//! supplied one.

use crate::atproto::ImageEmbed;
use crate::ui::avatar_cache;
use crate::ui::external;
use crate::ui::video_player::{MediaKind, PlayerConfig, VideoPlayer, VideoSource};
use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// One notch of zoom.
const ZOOM_STEP: f64 = 1.25;

/// 8 screen pixels per image pixel. Also stops a held key asking GTK for a
/// million-pixel widget. No minimum - zooming out stops at fitted.
const ZOOM_MAX: f64 = 8.0;

/// Modifiers that mean a keypress belongs to somebody else.
///
/// Control is absent, unlike the video player's list: Ctrl+plus, Ctrl+minus and
/// Ctrl+0 are the conventional zoom bindings.
const FOREIGN_MODIFIERS: gdk::ModifierType = gdk::ModifierType::ALT_MASK
    .union(gdk::ModifierType::SUPER_MASK)
    .union(gdk::ModifierType::META_MASK)
    .union(gdk::ModifierType::HYPER_MASK);

/// Play a video embed in a dialog.
///
/// Bluesky serves HLS, which a browser renders as raw playlist text and
/// `GtkMediaFile` cannot decode. A failure keeps the dialog open with a browser
/// button.
pub fn show_video(parent: &impl IsA<gtk4::Widget>, source: VideoSource) {
    show_video_at(parent, source, 0.0);
}

/// Play a video embed in a dialog, resuming `start_at` seconds in, for the
/// inline fullscreen button.
///
/// The seek waits for the first duration the stream reports; see
/// `VideoPlayer::refresh_position`.
pub fn show_video_at(parent: &impl IsA<gtk4::Widget>, source: VideoSource, start_at: f64) {
    let root = match parent
        .as_ref()
        .root()
        .and_then(|r| r.downcast::<gtk4::Window>().ok())
    {
        Some(w) => w,
        None => return,
    };

    // One pipeline per process: stop whatever the feed is playing. The hold is
    // a count, so this and the `dialog_closed` below must pair - a quoted video
    // and a GIF both reach here from any page.
    let director = crate::ui::inline_video::director();
    if let Some(director) = director.as_ref() {
        director.dialog_opened();
    }

    let dialog = adw::Dialog::new();
    dialog.set_title(match source.kind {
        MediaKind::Video => "Video",
        MediaKind::Gif => "GIF",
    });
    dialog.set_content_width(900);
    dialog.set_content_height(620);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());

    let player = VideoPlayer::new_with(
        source,
        PlayerConfig {
            start_at,
            ..PlayerConfig::dialog()
        },
    );
    // The dialog has exactly one video, so shortcuts are unambiguous. See
    // `install_shortcuts`.
    player.install_shortcuts(&dialog);

    toolbar.set_content(Some(player.widget()));

    // The dialog covers the window, so the error page's "Open in Browser"
    // button cannot reach the window's toast overlay. Give it one of its own.
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&toolbar));
    dialog.set_child(Some(&toasts));

    // Tear the pipeline down with the dialog, or it keeps fetching segments and
    // playing audio.
    dialog.connect_closed({
        let player = player.clone();
        // `closed` is emitted again if the dialog is presented again, and a
        // second `dialog_closed` would let go of another dialog's hold.
        let released = Cell::new(false);
        move |_| {
            if released.replace(true) {
                return;
            }
            player.stop();
            // The dialog may have written a new volume.
            if let Some(director) = director.as_ref() {
                director.refresh_volume();
                director.dialog_closed();
            }
        }
    });

    dialog.present(Some(&root));
}

/// Whether the picture is showing the current image.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LoadState {
    /// The current image's bytes have not arrived. The picture is blank.
    Loading,
    /// The picture is showing the current image.
    Shown,
    /// The current image could not be fetched or decoded. The picture is blank
    /// and waiting will not change that.
    Failed,
}

/// A full-size image and the actions on it.
struct ImageViewer {
    images: Vec<ImageEmbed>,
    index: Cell<usize>,
    /// `None` fits the image to the viewport, which is how the viewer opens.
    /// `Some(z)` is z screen pixels per image pixel, and the scroller pans
    /// around whatever that comes to.
    zoom: Cell<Option<f64>>,
    /// Whether the picture is showing `images[index]` yet.
    ///
    /// One `Picture` serves the whole gallery, so a paintable on the widget can
    /// still be the previous image. Every menu action reads this.
    load_state: Cell<LoadState>,
    /// The in-flight load, held so it can be cancelled on page or close.
    load: RefCell<Option<avatar_cache::ImageLoad>>,
    picture: gtk4::Picture,
    scroller: gtk4::ScrolledWindow,
    alt_label: gtk4::Label,
    title: adw::WindowTitle,
    dialog: adw::Dialog,
}

impl ImageViewer {
    fn new(images: Vec<ImageEmbed>, start: usize) -> Rc<Self> {
        let total = images.len();

        let dialog = adw::Dialog::new();
        dialog.set_content_width(1000);
        dialog.set_content_height(760);

        let picture = gtk4::Picture::new();
        picture.set_content_fit(gtk4::ContentFit::Contain);
        picture.set_hexpand(true);
        picture.set_vexpand(true);

        // GtkPicture can shrink to nothing, so while no zoom is set the
        // viewport hands it exactly the visible area and Contain letterboxes
        // inside that -- no scrollbars until a zoom asks for room.
        let scroller = gtk4::ScrolledWindow::new();
        scroller.set_child(Some(&picture));
        scroller.set_hexpand(true);
        scroller.set_vexpand(true);
        scroller.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Automatic);

        // Alt text is frequently absent, so the label only takes space when the
        // author actually wrote one.
        let alt_label = gtk4::Label::new(None);
        alt_label.add_css_class("dim-label");
        alt_label.set_wrap(true);
        // WordChar: a long URL in alt text would otherwise set the dialog's
        // minimum width. See video_player.rs.
        alt_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        // Cap the natural width so a paragraph of alt text does not stretch
        // across a 1000px dialog.
        alt_label.set_max_width_chars(80);
        alt_label.set_justify(gtk4::Justification::Center);
        alt_label.set_margin_start(12);
        alt_label.set_margin_end(12);
        alt_label.set_margin_bottom(12);

        let viewer = Rc::new(Self {
            images,
            index: Cell::new(start.min(total - 1)),
            zoom: Cell::new(None),
            load_state: Cell::new(LoadState::Loading),
            load: RefCell::new(None),
            picture,
            scroller,
            alt_label,
            title: adw::WindowTitle::new("Image", ""),
            dialog,
        });

        viewer.build_chrome();
        viewer.install_shortcuts();
        viewer.apply();
        viewer
    }

    /// Assemble the header, the actions and the dialog around the picture.
    fn build_chrome(self: &Rc<Self>) {
        let total = self.images.len();

        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&self.title));

        let zoom_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        zoom_box.add_css_class("linked");
        zoom_box.append(
            &self.zoom_button("zoom-out-symbolic", "Zoom out", |viewer| {
                viewer.zoom_by(1.0 / ZOOM_STEP)
            }),
        );
        // Otherwise the only way back to fitted is the keyboard shortcut.
        zoom_box.append(
            &self.zoom_button("zoom-fit-best-symbolic", "Fit to window", |viewer| {
                viewer.zoom_reset()
            }),
        );
        zoom_box.append(&self.zoom_button("zoom-in-symbolic", "Zoom in", |viewer| {
            viewer.zoom_by(ZOOM_STEP)
        }));
        header.pack_start(&zoom_box);

        // Both arrows sit on the innermost side of their group, flanking the
        // title: previous goes in after the zoom cluster so it ends up nearest
        // the title, mirroring next on the other side.
        if total > 1 {
            let prev_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
            prev_btn.set_tooltip_text(Some("Previous image"));
            prev_btn.update_property(&[gtk4::accessible::Property::Label("Previous image")]);
            prev_btn.connect_clicked({
                let weak = Rc::downgrade(self);
                move |_| {
                    if let Some(viewer) = weak.upgrade() {
                        viewer.go(-1);
                    }
                }
            });
            header.pack_start(&prev_btn);
        }

        header.pack_end(&self.build_menu());

        if total > 1 {
            let next_btn = gtk4::Button::from_icon_name("go-next-symbolic");
            next_btn.set_tooltip_text(Some("Next image"));
            next_btn.update_property(&[gtk4::accessible::Property::Label("Next image")]);
            next_btn.connect_clicked({
                let weak = Rc::downgrade(self);
                move |_| {
                    if let Some(viewer) = weak.upgrade() {
                        viewer.go(1);
                    }
                }
            });
            header.pack_end(&next_btn);
        }

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        content.append(&self.scroller);
        content.append(&self.alt_label);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);

        // The dialog covers the window's toast overlay, so give it one of its
        // own.
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&content));
        toolbar.set_content(Some(&toasts));

        self.dialog.set_child(Some(&toolbar));

        // Cancel the in-flight load; do not rely on the viewer being the last
        // strong ref.
        self.dialog.connect_closed({
            let weak = Rc::downgrade(self);
            move |_| {
                if let Some(viewer) = weak.upgrade() {
                    viewer.load.replace(None);
                }
            }
        });
    }

    fn zoom_button(
        self: &Rc<Self>,
        icon_name: &str,
        label: &str,
        action: fn(&Rc<ImageViewer>),
    ) -> gtk4::Button {
        let button = gtk4::Button::from_icon_name(icon_name);
        button.set_tooltip_text(Some(label));
        button.update_property(&[gtk4::accessible::Property::Label(label)]);
        button.connect_clicked({
            let weak = Rc::downgrade(self);
            move |_| {
                if let Some(viewer) = weak.upgrade() {
                    action(&viewer);
                }
            }
        });
        button
    }

    /// The overflow menu for the image on screen.
    ///
    /// No Share item: xdg-desktop-portal has no share interface, and neither
    /// GTK 4.14 nor libadwaita 1.5 has a share sheet.
    fn build_menu(self: &Rc<Self>) -> gtk4::MenuButton {
        let popover_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        popover_box.set_margin_top(6);
        popover_box.set_margin_bottom(6);
        popover_box.set_margin_start(6);
        popover_box.set_margin_end(6);

        let popover = gtk4::Popover::new();
        popover.add_css_class("menu");
        popover.set_has_arrow(false);

        let copy_image = menu_item("image-x-generic-symbolic", "Copy Image");
        let copy_link = menu_item("edit-copy-symbolic", "Copy Image Link");
        let save_as = menu_item("document-save-symbolic", "Save Image As\u{2026}");
        popover_box.append(&copy_image);
        popover_box.append(&copy_link);
        popover_box.append(&save_as);

        let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        separator.set_margin_top(4);
        separator.set_margin_bottom(4);
        popover_box.append(&separator);

        let open_browser = menu_item("web-browser-symbolic", "Open Image in Browser");
        popover_box.append(&open_browser);

        self.connect_menu_item(&copy_image, &popover, |viewer| viewer.copy_image());
        self.connect_menu_item(&copy_link, &popover, |viewer| viewer.copy_url());
        self.connect_menu_item(&save_as, &popover, |viewer| viewer.save_image());
        self.connect_menu_item(&open_browser, &popover, |viewer| viewer.open_in_browser());

        popover.set_child(Some(&popover_box));

        let menu_btn = gtk4::MenuButton::new();
        menu_btn.set_icon_name("view-more-symbolic");
        menu_btn.add_css_class("flat");
        menu_btn.set_tooltip_text(Some("Image actions"));
        menu_btn.update_property(&[gtk4::accessible::Property::Label("Image actions")]);
        menu_btn.set_popover(Some(&popover));
        menu_btn
    }

    fn connect_menu_item(
        self: &Rc<Self>,
        item: &gtk4::Button,
        popover: &gtk4::Popover,
        action: fn(&Rc<ImageViewer>),
    ) {
        let popover = popover.clone();
        let weak = Rc::downgrade(self);
        item.connect_clicked(move |_| {
            // The menu covers the image these act on.
            popover.popdown();
            if let Some(viewer) = weak.upgrade() {
                action(&viewer);
            }
        });
    }

    /// Keyboard zoom and pagination, plus Ctrl+scroll.
    fn install_shortcuts(self: &Rc<Self>) {
        let total = self.images.len();

        let key = gtk4::EventControllerKey::new();
        // Capture, not bubble: a focused header button claims Left and Right
        // for focus navigation before a bubbling controller sees them.
        key.set_propagation_phase(gtk4::PropagationPhase::Capture);
        key.connect_key_pressed({
            let weak = Rc::downgrade(self);
            move |_, keyval, _, state| {
                let Some(viewer) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                if state.intersects(FOREIGN_MODIFIERS) {
                    return glib::Propagation::Proceed;
                }
                match keyval {
                    // `equal` because unshifted `+` is `=` on most layouts.
                    gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => {
                        viewer.zoom_by(ZOOM_STEP);
                        glib::Propagation::Stop
                    }
                    gdk::Key::minus | gdk::Key::KP_Subtract => {
                        viewer.zoom_by(1.0 / ZOOM_STEP);
                        glib::Propagation::Stop
                    }
                    gdk::Key::_0 | gdk::Key::KP_0 => {
                        viewer.zoom_reset();
                        glib::Propagation::Stop
                    }
                    gdk::Key::Left if total > 1 => {
                        viewer.go(-1);
                        glib::Propagation::Stop
                    }
                    gdk::Key::Right if total > 1 => {
                        viewer.go(1);
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Proceed,
                }
            }
        });
        self.dialog.add_controller(key);

        let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
        // Capture, so a Ctrl+scroll zooms instead of being spent scrolling the
        // window it happened over.
        scroll.set_propagation_phase(gtk4::PropagationPhase::Capture);
        scroll.connect_scroll({
            let weak = Rc::downgrade(self);
            move |controller, _dx, dy| {
                if !controller
                    .current_event_state()
                    .contains(gdk::ModifierType::CONTROL_MASK)
                {
                    // Plain scrolling still pans a zoomed-in image.
                    return glib::Propagation::Proceed;
                }
                if dy == 0.0 || !dy.is_finite() {
                    return glib::Propagation::Proceed;
                }
                // Scale by how far the wheel or fingers moved. A wheel reports
                // whole notches, so dy of -1 is one ZOOM_STEP. A touchpad
                // reports fractions dozens of times per gesture, and taking a
                // full step off the sign alone ran one flick to ZOOM_MAX.
                let factor = ZOOM_STEP.powf(-dy);
                if let Some(viewer) = weak.upgrade() {
                    viewer.zoom_by(factor);
                }
                glib::Propagation::Stop
            }
        });
        self.scroller.add_controller(scroll);
    }

    fn current(&self) -> &ImageEmbed {
        &self.images[self.index.get()]
    }

    /// The decoded current image, or `None` while the picture still holds an
    /// earlier one. [`Self::not_ready_message`] says which case it was.
    fn texture(&self) -> Option<gdk::Texture> {
        if self.load_state.get() != LoadState::Shown {
            return None;
        }
        self.picture
            .paintable()
            .and_then(|paintable| paintable.downcast::<gdk::Texture>().ok())
    }

    /// Why there is nothing to act on, in a sentence.
    fn not_ready_message(&self) -> &'static str {
        match self.load_state.get() {
            LoadState::Failed => "Image couldn't be loaded",
            _ => "Image is still loading",
        }
    }

    /// Show the image at `index`, with its alt text and title.
    fn apply(self: &Rc<Self>) {
        // Drop the old load and blank the picture, so `texture` is None until
        // the new bytes land.
        self.load.replace(None);
        self.picture.set_paintable(gdk::Paintable::NONE);
        self.load_state.set(LoadState::Loading);

        let image = self.current();

        let alt = image.alt.trim();
        self.alt_label.set_visible(!alt.is_empty());
        self.alt_label.set_label(alt);

        let title = if self.images.len() > 1 {
            format!("Image {} of {}", self.index.get() + 1, self.images.len())
        } else {
            "Image".to_string()
        };
        self.dialog.set_title(&title);
        self.title.set_title(&title);

        // A new image opens fitted.
        self.zoom.set(None);
        self.apply_zoom();

        // Started last: a cached image finishes inside this call, so everything
        // the callback reads must already be set.
        let handle = avatar_cache::load_image_into_picture_tracked(
            self.picture.clone(),
            image.fullsize.clone(),
            {
                let weak = Rc::downgrade(self);
                move |result| {
                    let Some(viewer) = weak.upgrade() else {
                        return;
                    };
                    match result {
                        avatar_cache::ImageLoadResult::Shown => {
                            viewer.load_state.set(LoadState::Shown);
                            // There is a texture now, so the fitted scale and
                            // the zoom subtitle can finally be worked out.
                            viewer.apply_zoom();
                        }
                        avatar_cache::ImageLoadResult::Failed => {
                            // A blank picture on its own looks like a slow
                            // load, so say the image failed.
                            viewer.load_state.set(LoadState::Failed);
                            external::toast(&viewer.picture, "Couldn't load image");
                        }
                    }
                }
            },
        );
        self.load.replace(Some(handle));
    }

    /// Step `delta` images. Wraps; there are at most four.
    fn go(self: &Rc<Self>, delta: isize) {
        let total = self.images.len() as isize;
        if total < 2 {
            return;
        }
        let next = (self.index.get() as isize + delta).rem_euclid(total);
        self.index.set(next as usize);
        self.apply();
    }

    /// Fitted scale, and the floor for zooming out - the viewport never hands
    /// the picture less than the visible area.
    fn fit_scale(&self) -> Option<f64> {
        let texture = self.texture()?;
        let (width, height) = (texture.width() as f64, texture.height() as f64);
        // The page size is the visible area with the scrollbars already taken
        // off, which is the box the image has to fit inside.
        let page_width = self.scroller.hadjustment().page_size();
        let page_height = self.scroller.vadjustment().page_size();
        if width <= 0.0 || height <= 0.0 || page_width <= 0.0 || page_height <= 0.0 {
            return None;
        }
        Some((page_width / width).min(page_height / height))
    }

    fn zoom_by(&self, factor: f64) {
        let Some(fit) = self.fit_scale() else {
            return;
        };
        let target = (self.zoom.get().unwrap_or(fit) * factor).min(ZOOM_MAX);
        if target <= fit {
            // All the way out is the fitted state, not an equivalent number.
            self.zoom_reset();
            return;
        }
        self.zoom.set(Some(target));
        self.apply_zoom();
    }

    fn zoom_reset(&self) {
        self.zoom.set(None);
        self.apply_zoom();
    }

    fn apply_zoom(&self) {
        match (self.zoom.get(), self.texture()) {
            (Some(zoom), Some(texture)) => {
                self.picture.set_size_request(
                    (texture.width() as f64 * zoom).round() as i32,
                    (texture.height() as f64 * zoom).round() as i32,
                );
                // Centre while zoomed: the viewport rounds the request up to
                // the visible area in whichever direction the image does not
                // fill, and centring leaves that slack as margin. Contain draws
                // inside it at the zoom asked for; Fill would stretch.
                self.picture.set_halign(gtk4::Align::Center);
                self.picture.set_valign(gtk4::Align::Center);
            }
            _ => {
                // Fill, so a picture smaller than the window is scaled up to it
                // the way the viewer has always opened.
                self.picture.set_size_request(-1, -1);
                self.picture.set_halign(gtk4::Align::Fill);
                self.picture.set_valign(gtk4::Align::Fill);
            }
        }

        // The zoom level belongs in the subtitle, not in a toast: it changes on
        // every scroll notch, and a toast per notch would bury the image.
        self.title.set_subtitle(&match self.zoom.get() {
            Some(zoom) => format!("{}%", (zoom * 100.0).round()),
            None => String::new(),
        });
    }

    fn copy_url(&self) {
        // The embed's own URL, not avatar_cache's `@jpeg`-normalised cache key.
        // This is the link a person pastes somewhere else.
        external::copy_text(&self.picture, &self.current().fullsize, "Image link");
    }

    fn copy_image(&self) {
        let Some(texture) = self.texture() else {
            external::toast(&self.picture, self.not_ready_message());
            return;
        };
        // GTK assembles the union content provider behind this, so the paste
        // side is offered PNG, JPEG, TIFF and GdkPixbuf without us building one.
        // The clipboard is served lazily by this process, so the copy does not
        // outlive Hangar -- nothing persists it on Wayland once we quit.
        self.picture.display().clipboard().set_texture(&texture);
        external::toast(&self.picture, "Image copied to clipboard");
    }

    fn open_in_browser(&self) {
        external::open_url(&self.picture, &self.current().fullsize, "image");
    }

    fn save_image(self: &Rc<Self>) {
        // Prefer the bytes the CDN served over a re-encode: no generation loss,
        // and no PNG bloat on what was already a JPEG. Both arms are keyed on
        // the current image.
        let bytes = match avatar_cache::cached_bytes(&self.current().fullsize) {
            Some(bytes) => bytes,
            None => match self.texture() {
                Some(texture) => texture.save_to_png_bytes().to_vec(),
                None => {
                    external::toast(&self.picture, self.not_ready_message());
                    return;
                }
            },
        };

        // A save dialog rather than a silent write into ~/Pictures. The Flatpak
        // manifest grants no filesystem access at all, so a plain write would
        // fail there; the file chosen here comes back already granted through
        // the document portal, with no new sandbox permission to justify.
        let file_dialog = gtk4::FileDialog::builder()
            .title("Save Image")
            .accept_label("Save")
            .initial_name(suggested_filename(&self.current().fullsize))
            .modal(true)
            .build();
        if let Some(pictures) = glib::user_special_dir(glib::UserDirectory::Pictures) {
            file_dialog.set_initial_folder(Some(&gio::File::for_path(pictures)));
        }

        let window = self
            .dialog
            .root()
            .and_then(|root| root.downcast::<gtk4::Window>().ok());
        let weak = Rc::downgrade(self);
        file_dialog.save(window.as_ref(), gio::Cancellable::NONE, move |result| {
            let Some(viewer) = weak.upgrade() else {
                return;
            };
            match result {
                Ok(file) => viewer.write_image(file, bytes),
                // Closing the picker is ordinary behaviour, not a failure.
                Err(e) if e.matches(gtk4::DialogError::Dismissed) => {}
                Err(e) => {
                    eprintln!("Save dialog failed: {e}");
                    external::toast(&viewer.picture, "Couldn't save image");
                }
            }
        });
    }

    /// Write `bytes` into `file` and say how it went.
    ///
    /// Async so the write lands on a GIO worker: the image is already in
    /// memory, but a slow or remote destination would otherwise freeze the
    /// window mid-save.
    fn write_image(&self, file: gio::File, bytes: Vec<u8>) {
        let picture = self.picture.clone();
        // A save outlives its dialog, so hold the window (weakly) for the toast.
        let window = self.root_window();
        let saved = file.clone();
        file.replace_contents_async(
            bytes,
            None,
            false,
            gio::FileCreateFlags::REPLACE_DESTINATION,
            gio::Cancellable::NONE,
            // The buffer is handed back either way; it has done its job.
            move |result| {
                // Log first; the toast may have no overlay left.
                if let Err((_, e)) = &result {
                    eprintln!("Failed to save image: {e}");
                }
                let Some(target) = toast_target(&picture, &window) else {
                    return;
                };
                match result {
                    Ok(_) => external::toast_with_action(
                        &target,
                        "Image saved",
                        "Show in Files",
                        move || show_in_files(&saved, &picture, &window),
                    ),
                    Err(_) => external::toast(&target, "Couldn't save image"),
                }
            },
        );
    }

    /// The window the dialog is presented over, weakly.
    fn root_window(&self) -> glib::WeakRef<gtk4::Window> {
        self.dialog
            .root()
            .and_then(|root| root.downcast::<gtk4::Window>().ok())
            .map(|window| window.downgrade())
            .unwrap_or_default()
    }
}

/// Where to raise a toast about work that can outlive the dialog.
///
/// The picture while the dialog is up, the window after. A toast queued on an
/// unmapped tree is never shown.
fn toast_target(
    picture: &gtk4::Picture,
    window: &glib::WeakRef<gtk4::Window>,
) -> Option<gtk4::Widget> {
    if picture.is_mapped() {
        return Some(picture.clone().upcast());
    }
    window.upgrade().map(|window| window.upcast())
}

/// Reveal `file` in the file manager, and say so when that does not work.
///
/// Under Flatpak this is a portal call: no file manager, a denied request and an
/// unresolvable path all come back as an error. The window goes in as the parent
/// so the portal has something to be modal to.
fn show_in_files(file: &gio::File, picture: &gtk4::Picture, window: &glib::WeakRef<gtk4::Window>) {
    let parent = window.upgrade();
    let picture = picture.clone();
    let window = window.clone();
    gtk4::FileLauncher::new(Some(file)).open_containing_folder(
        parent.as_ref(),
        gio::Cancellable::NONE,
        move |result| {
            if let Err(e) = result {
                eprintln!("Failed to show image in file manager: {e}");
                if let Some(target) = toast_target(&picture, &window) {
                    external::toast(&target, "Couldn't open the containing folder");
                }
            }
        },
    );
}

/// One row of the overflow menu, matching the post menu's flat icon+label rows.
fn menu_item(icon_name: &str, label: &str) -> gtk4::Button {
    let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    content.append(&gtk4::Image::from_icon_name(icon_name));
    content.append(&gtk4::Label::new(Some(label)));

    let button = gtk4::Button::new();
    button.set_child(Some(&content));
    button.add_css_class("flat");
    button
}

/// `…/bafkrei…@jpeg` → `bafkrei….jpg`.
///
/// Bluesky's CDN names the format after an `@` rather than with an extension,
/// which no file manager understands.
fn suggested_filename(url: &str) -> String {
    let last = url.rsplit('/').next().unwrap_or_default();
    let (stem, format) = last.split_once('@').unwrap_or((last, "jpeg"));
    let extension = match format {
        "png" => "png",
        "webp" => "webp",
        _ => "jpg",
    };
    let stem = if stem.is_empty() { "image" } else { stem };
    format!("{stem}.{extension}")
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

    let viewer = ImageViewer::new(images, start);

    // Every control holds a Weak back to the viewer, so the one strong ref
    // needs somewhere to live once this returns. Park it on `closed`.
    let held = RefCell::new(Some(Rc::clone(&viewer)));
    viewer.dialog.connect_closed(move |_| {
        held.borrow_mut().take();
    });

    viewer.dialog.present(Some(&root));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atproto::ImageEmbed;
    use std::time::{Duration, Instant};

    /// Run the GTK main loop for `ms`, the way sitting in front of the dialog
    /// would. The loads under test are 16ms polls driven by a fetch on another
    /// runtime, so nothing here happens without the loop turning.
    fn pump(ms: u64) {
        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < deadline {
            while context.iteration(false) {}
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// A solid PNG of a given size. Which image is on screen is read off the
    /// texture's width.
    fn png(size: i32) -> Vec<u8> {
        let pixbuf =
            gdk::gdk_pixbuf::Pixbuf::new(gdk::gdk_pixbuf::Colorspace::Rgb, false, 8, size, size)
                .expect("allocate pixbuf");
        pixbuf.fill(0x00_88_44_ff);
        pixbuf
            .save_to_bufferv("png", &[])
            .expect("encode png")
            .to_vec()
    }

    /// Serve three images: one prompt, one slow enough to page away from, and
    /// one that is never coming.
    fn start_server() -> u16 {
        let prompt = png(8);
        let slow = png(24);

        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind test server");
        let port = server
            .server_addr()
            .to_ip()
            .expect("test server address")
            .port();

        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let (prompt, slow) = (prompt.clone(), slow.clone());
                // One thread per request, so the slow image is still
                // outstanding while another is asked for.
                std::thread::spawn(move || {
                    // The query string is only there to give each phase of the
                    // test its own key in the process-wide image cache.
                    let path = request
                        .url()
                        .split('?')
                        .next()
                        .unwrap_or_default()
                        .to_string();
                    let response = match path.as_str() {
                        "/prompt.png" => {
                            std::thread::sleep(Duration::from_millis(120));
                            tiny_http::Response::from_data(prompt).boxed()
                        }
                        "/slow.png" => {
                            std::thread::sleep(Duration::from_millis(700));
                            tiny_http::Response::from_data(slow).boxed()
                        }
                        _ => tiny_http::Response::empty(404).boxed(),
                    };
                    let _ = request.respond(response);
                });
            }
        });

        port
    }

    fn embed(url: String) -> ImageEmbed {
        ImageEmbed {
            thumb: url.clone(),
            fullsize: url,
            alt: String::new(),
            aspect_ratio: None,
        }
    }

    /// The picture and every menu action must be about the current image.
    ///
    /// All three phases share one test because GTK may only be used from the
    /// thread that initialised it.
    #[test]
    fn viewer_never_shows_or_acts_on_the_previous_image() {
        // On the GTK thread, or skipped where there is no display: see
        // `crate::ui::with_gtk`. This one also turns the main loop, so it has
        // to be the thread that owns it.
        crate::ui::with_gtk(viewer_never_shows_or_acts_on_the_previous_image_body);
    }

    fn viewer_never_shows_or_acts_on_the_previous_image_body() {
        let port = start_server();
        let url = |name: &str| format!("http://127.0.0.1:{port}/{name}");

        // ---- Phase 1: page out to a slow image and back before it lands ----
        //
        // Right then Left. Image 2's fetch finishes last, and its poll used to
        // call set_paintable unconditionally on the same Picture: the dialog
        // showed image 2 under the header "Image 1 of 2".
        let images = vec![embed(url("prompt.png?a")), embed(url("slow.png?b"))];
        let viewer = ImageViewer::new(images, 0);

        viewer.go(1);
        viewer.go(-1);
        assert_eq!(viewer.index.get(), 0, "back on the first image");

        pump(1400);

        let texture = viewer.texture().expect("the first image should be showing");
        assert_eq!(
            texture.width(),
            8,
            "the picture is showing the slow second image, which the reader paged away from"
        );
        assert!(viewer.load_state.get() == LoadState::Shown);

        // ---- Phase 2: page to an image that will never arrive ----
        //
        // The picture kept the previous image and `texture()` handed it to Copy
        // and Save, which wrote the wrong pixels under this image's CID.
        let images = vec![embed(url("prompt.png?c")), embed(url("gone.png?d"))];
        let viewer = ImageViewer::new(images, 0);
        pump(600);
        assert!(viewer.texture().is_some(), "first image loaded");

        viewer.go(1);
        pump(600);

        assert!(
            viewer.texture().is_none(),
            "a failed load left the previous image on offer to Copy and Save"
        );
        assert!(
            viewer.picture.paintable().is_none(),
            "picture was not cleared"
        );
        assert_eq!(viewer.not_ready_message(), "Image couldn't be loaded");

        // ---- Phase 3: the load does not outlive the viewer ----
        //
        // The poll held a strong clone of the Picture, so it kept the widget
        // chain alive past the dialog and painted into it.
        let images = vec![embed(url("slow.png?e"))];
        let viewer = ImageViewer::new(images, 0);
        let picture = viewer.picture.clone();
        drop(viewer);

        pump(1400);
        assert!(
            picture.paintable().is_none(),
            "a load outlived the viewer and painted into its picture"
        );
    }
}
