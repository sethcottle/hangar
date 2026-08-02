// SPDX-License-Identifier: MPL-2.0

//! Full-size image and video viewers.
//!
//! Post embeds render thumbnails, which are small and often cropped. This
//! presents the fullsize variant in a dialog so a post's images can actually
//! be read, with the alt text alongside where the author supplied one.

use crate::atproto::ImageEmbed;
use crate::ui::avatar_cache;
use crate::ui::external;
use crate::ui::video_player::{VideoPlayer, VideoSource};
use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// One notch of zoom.
///
/// Small enough that holding a key does not shoot past what you were aiming
/// at, large enough that a single press is worth pressing.
const ZOOM_STEP: f64 = 1.25;

/// Past eight screen pixels per image pixel there is nothing left to see, and
/// the cap keeps a held key from asking GTK for a widget a million pixels wide.
///
/// There is no matching minimum: zooming out stops at the fitted view, which
/// is as far out as the whole image being visible.
const ZOOM_MAX: f64 = 8.0;

/// Modifiers that mean a keypress belongs to somebody else.
///
/// Control is deliberately absent, unlike the video player's list: Ctrl+plus,
/// Ctrl+minus and Ctrl+0 are the conventional zoom bindings, so Control on
/// those keys is ours to answer.
const FOREIGN_MODIFIERS: gdk::ModifierType = gdk::ModifierType::ALT_MASK
    .union(gdk::ModifierType::SUPER_MASK)
    .union(gdk::ModifierType::META_MASK)
    .union(gdk::ModifierType::HYPER_MASK);

/// Play a video embed in a dialog.
///
/// Bluesky serves video as an HLS playlist. Handing that URL to the browser is
/// what produced a page of raw playlist text rather than a video, so play it
/// here instead, through Hangar's own GStreamer pipeline rather than
/// `GtkMediaFile`, which cannot decode these streams.
///
/// A failure keeps the dialog open and offers the post on the web behind a
/// button. Opening a browser without being asked is what the old code did, from
/// a timer that could not tell a slow stream from a broken one.
pub fn show_video(parent: &impl IsA<gtk4::Widget>, source: VideoSource) {
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

    let player = VideoPlayer::new(source);
    player.install_shortcuts(&dialog);

    toolbar.set_content(Some(player.widget()));

    // The dialog covers the window, so the error page's "Open in Browser"
    // button cannot reach the window's toast overlay. Give it one of its own.
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&toolbar));
    dialog.set_child(Some(&toasts));

    // Tear the pipeline down with the dialog, otherwise it keeps fetching
    // segments and playing audio into a window nobody can see.
    dialog.connect_closed({
        let player = player.clone();
        move |_| player.stop()
    });

    dialog.present(Some(&root));
}

/// What the picture is showing, which is a different question from whether it
/// has a paintable on it.
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

/// A full-size image, plus everything you can do with the one on screen.
///
/// The actions arrived after the viewer was a single function passing five
/// cloned widgets into every closure. One struct behind an `Rc`, the way
/// `VideoPlayer` does it, is what stops the next action from adding a sixth.
struct ImageViewer {
    images: Vec<ImageEmbed>,
    index: Cell<usize>,
    /// `None` fits the image to the viewport, which is how the viewer opens.
    /// `Some(z)` is z screen pixels per image pixel, and the scroller pans
    /// around whatever that comes to.
    zoom: Cell<Option<f64>>,
    /// Whether the picture is showing `images[index]` yet.
    ///
    /// One `Picture` serves every image in the gallery, so "there is a
    /// paintable on the widget" answers a question nobody asked: it stays true
    /// across a page to an image that has not loaded. Every action in the menu
    /// is about `images[index]`, so they all read this instead.
    load_state: Cell<LoadState>,
    /// The load the picture is waiting on, held so it can be cancelled.
    ///
    /// Dropping it is what guarantees the previous image can no longer paint
    /// itself over the current one, and what stops its poll running on after
    /// the dialog is closed.
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
        // Alt text is arbitrary text off the wire and nothing sits between this
        // label and the dialog to absorb it. Plain `Word` wrapping makes a
        // label's minimum width its longest unbreakable run, so one long URL in
        // somebody's alt text would set the dialog's minimum width to the width
        // of that URL. `WordChar` breaks mid-token when a token cannot fit,
        // which puts the minimum back at one character.
        alt_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        // And a cap on the natural width, so a paragraph of alt text sets its
        // own comfortable measure instead of stretching across a 1000px dialog.
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

        // Only paginate when there is somewhere to paginate to.
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

        let zoom_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        zoom_box.add_css_class("linked");
        zoom_box.append(
            &self.zoom_button("zoom-out-symbolic", "Zoom out", |viewer| {
                viewer.zoom_by(1.0 / ZOOM_STEP)
            }),
        );
        // A visible way back to fitted: without it, the only exit from a zoom
        // is a keyboard shortcut nobody was told about.
        zoom_box.append(
            &self.zoom_button("zoom-fit-best-symbolic", "Fit to window", |viewer| {
                viewer.zoom_reset()
            }),
        );
        zoom_box.append(&self.zoom_button("zoom-in-symbolic", "Zoom in", |viewer| {
            viewer.zoom_by(ZOOM_STEP)
        }));
        header.pack_start(&zoom_box);

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

        // The dialog covers the window, so a toast raised from in here would
        // come up behind it on the window's own overlay. Same reasoning as the
        // video dialog above.
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&content));
        toolbar.set_content(Some(&toasts));

        self.dialog.set_child(Some(&toolbar));

        // A load in flight is a timer holding this dialog's picture, and the
        // picture holds the rest of the tree. Dropping the viewer would take it
        // with it, but only for as long as nothing else holds a strong
        // reference; cancelling here does not depend on that being true.
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

    /// The overflow menu of everything you can do with the image on screen.
    ///
    /// There is no Share item because there is nothing honest to put behind
    /// one: xdg-desktop-portal has no share interface, and neither GTK 4.14
    /// nor libadwaita 1.5 offers a share sheet. Sharing on this platform means
    /// handing the link to the clipboard or to a browser, so the menu says
    /// exactly that instead of naming a picker that will never appear.
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
            // Close the menu first: it sits over the image, and every one of
            // these actions is about the image underneath it.
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
        // Capture rather than bubble. Now that the header has buttons, a
        // focused button claims Left and Right for focus navigation before a
        // bubbling controller on the dialog ever sees them, which is exactly
        // how the existing prev/next keys would have stopped working.
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
                // Scale by how far the wheel or the fingers actually moved. A
                // mouse wheel reports whole notches, so `dy` of -1 is exactly
                // one `ZOOM_STEP` in, same as the keyboard. A touchpad reports
                // fractions of a notch dozens of times per gesture: taking a
                // full step off the sign alone ran one flick straight to
                // `ZOOM_MAX`, and one flick the other way straight back to
                // fitted, with nothing usable in between.
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

    /// The decoded image on screen, if what is on screen is the current image.
    ///
    /// Copy, Save and the zoom arithmetic all read this, and all of them are
    /// about `images[index]`, so it deliberately answers `None` while the
    /// picture still holds an earlier image or nothing at all. Ask
    /// [`Self::not_ready_message`] which of those it was.
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
        // Whatever the outgoing image's load was doing, it is not about this
        // image. Dropping the handle stops its poll before it can paint over
        // the new one; blanking the picture is what makes `texture` tell the
        // truth in the gap before the new bytes arrive, and what stops a failed
        // load leaving the reader looking at the image they just paged away
        // from while every menu item acts on this one.
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

        // A new image opens fitted. Carrying the last one's pixel zoom over
        // would drop the reader somewhere in the middle of a picture they have
        // not seen the whole of yet.
        self.zoom.set(None);
        self.apply_zoom();

        // Started last, and the handle stored last, because a cached image
        // finishes inside this call: everything the callback reads has to be
        // settled before it can run.
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
                            // The picture stays blank, which on its own reads as
                            // an image that is taking a while. Say that it is
                            // not coming.
                            viewer.load_state.set(LoadState::Failed);
                            external::toast(&viewer.picture, "Couldn't load image");
                        }
                    }
                }
            },
        );
        self.load.replace(Some(handle));
    }

    /// Step `delta` images, wrapping around.
    ///
    /// With at most four images, stopping dead at an edge is more annoying
    /// than useful.
    fn go(self: &Rc<Self>, delta: isize) {
        let total = self.images.len() as isize;
        if total < 2 {
            return;
        }
        let next = (self.index.get() as isize + delta).rem_euclid(total);
        self.index.set(next as usize);
        self.apply();
    }

    /// Screen pixels per image pixel once the image is fitted to the viewport.
    ///
    /// Also the floor for zooming out. The viewport never hands the picture
    /// less than the visible area, so a smaller number could not be drawn and
    /// would only make the subtitle lie.
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
            // All the way back out is just the fitted view. Landing on the
            // real state rather than an equivalent number keeps the subtitle
            // and the "Fit to window" button agreeing with each other.
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
                // Centre while zoomed. The viewport rounds the request up to
                // the visible area in whichever direction the image does not
                // fill, and up to the image's own size below 1:1; centring
                // leaves that slack as margin. Contain -- set once, never
                // changed -- draws inside it at the smaller of the two ratios,
                // which is the zoom that was asked for. Fill would stretch the
                // image into the slack instead, badly on a tall or wide one.
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
        // and no PNG bloat on what was already a JPEG.
        // Both arms are keyed on the current image: the cache lookup by its own
        // URL, the fallback by `texture`, which is `None` unless the picture is
        // showing this image. Neither can hand back the one before it.
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
        // A save outlives the dialog it was started from, and a closed dialog's
        // overlay cannot show anything ever again. Hold the window too, so both
        // the confirmation and the error still have somewhere to land. Weakly:
        // a toast that outlived the window has nobody left to tell.
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
                // Logged before anything else: the toast needs somewhere to go
                // and may not find one, but the log always takes it.
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

/// Where a toast about work that can outlive the dialog should be raised.
///
/// The picture while the viewer is up, so the toast comes up on the dialog's
/// own overlay rather than behind it. Once the dialog is closed the picture is
/// unmapped and a toast added there is queued against a tree that will never be
/// shown again — which is how a save that finished a moment too late used to
/// report nothing at all, success or failure. The window outlives the dialog,
/// so fall back to that.
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
/// Under Flatpak this is a portal call: no file manager installed, a denied
/// request, and a path the portal cannot resolve all come back as an error, and
/// throwing that error away made all three indistinguishable from a button that
/// does nothing. The window goes in as the parent so the portal has something
/// to be modal to rather than floating loose.
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
/// which no file manager understands. The CID in front of it is at least
/// unique, so it beats offering to save every image as "image".
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

    // Every control in the viewer holds only a weak reference back to it, so
    // that the widgets it owns and the closures they carry do not pin each
    // other alive. That leaves the one strong reference with nowhere to live
    // once this function returns -- the dialog would come up with every button
    // dead. Park it on `closed`, which both keeps it for as long as the dialog
    // is up and lets go the moment it is not.
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

    /// A solid PNG of a given size. Size is the identity: which image is on
    /// screen is read straight off the texture's width.
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
                // One thread per request: the point of the slow image is that
                // it is still outstanding while another one is asked for.
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

    /// Everything the viewer says about the image on screen has to be about the
    /// image the reader is actually on.
    ///
    /// All three phases share one test because GTK may only be used from the
    /// thread that initialised it, and the test harness gives each `#[test]` a
    /// thread of its own.
    #[test]
    fn viewer_never_shows_or_acts_on_the_previous_image() {
        // Headless CI has no display, and a GTK widget cannot be built without
        // one. Nothing to assert there rather than a failure to report.
        if gtk4::init().is_err() {
            eprintln!("skipping: no display available");
            return;
        }
        adw::init().expect("libadwaita");

        let port = start_server();
        let url = |name: &str| format!("http://127.0.0.1:{port}/{name}");

        // ---- Phase 1: page out to a slow image and back before it lands ----
        //
        // Open image 1, press Right, press Left. Image 2's fetch is still
        // running and finishes last. Before the fix its poll was still
        // installed, still holding this same Picture, and called set_paintable
        // unconditionally — so the dialog ended up showing image 2 while the
        // header said "Image 1 of 2" and every menu action used image 1's URL.
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
        // No race needed for this one. Before the fix the picture kept the
        // previous image, and `texture()` — which is what the "still loading"
        // guards in Copy Image and Save Image As read — happily handed it over.
        // Copy Image copied the wrong image; Save Image As wrote the wrong
        // image's pixels into a file named after this one's CID.
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
        // The third instance of this bug in the tree: the poll held a strong
        // clone of the Picture and was tied to nothing but the fetch, so it
        // kept the whole widget chain alive past the dialog and then painted
        // into it.
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
