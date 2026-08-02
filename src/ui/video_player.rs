// SPDX-License-Identifier: MPL-2.0

//! A video player driven by Hangar's own GStreamer pipeline.
//!
//! The widget is a self-contained `GtkBox`: picture, transport controls, and an
//! error page it switches to when the pipeline gives up. Keeping the chrome
//! here rather than in the dialog is what will let a post row embed the same
//! player inline later without any of this being rewritten.

use crate::media::{self, MediaError};
use crate::state::{AppSettings, VideoVolume};
use gst::prelude::*;
use gstreamer as gst;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// How often the position slider and time label are refreshed.
const TICK: Duration = Duration::from_millis(250);

/// How long a slider drag has to settle before the pipeline is actually sought.
///
/// Every seek on an HLS stream is a flushing one that costs a segment fetch, so
/// following the handle sample by sample would stall the whole drag.
const SEEK_SETTLE: Duration = Duration::from_millis(150);

/// How long the volume has to sit still before it is written to disk.
///
/// A slider drag emits a value per pixel; the level is app-wide, so it is worth
/// persisting, but not worth a settings write per frame.
const VOLUME_SETTLE: Duration = Duration::from_millis(500);

/// How far the arrow keys move through a video.
const KEY_SEEK_SECONDS: f64 = 5.0;

/// How far short of the duration a seek is allowed to land.
///
/// Dragging the scrubber fully right asks for exactly the duration, because the
/// adjustment's upper bound *is* the duration. Under the KEY_UNIT flags this
/// player used to pass, the demuxer refuses that seek, and a refused flushing
/// seek leaves playbin3 wedged in ASYNC with a pending PAUSED that nothing ever
/// resolves: no EOS, no bus message, a frozen frame, a Pause icon that lies,
/// and no way out but closing the dialog.
///
/// ACCURATE does not rescue the exact-duration case: measured with the clamp
/// skipped, an ACCURATE seek to exactly the duration wedged the pipeline
/// permanently, 2 runs out of 2 — `Ok(Async)` with a pending PAUSED that never
/// resolved, position frozen, no EOS. This epsilon is the whole fix, not belt
/// and braces. Stopping a frame short of the end is invisible either way.
const SEEK_END_EPSILON: f64 = 0.05;

/// How many ticks a seek may stay outstanding before the handle is handed back.
///
/// ASYNC_DONE is the real signal that a seek has landed. This only covers the
/// case where that message never arrives: without it a lost ASYNC_DONE would
/// pin the handle to a position the pipeline is not at for as long as the
/// dialog stays open. Eight ticks is two seconds, against a worst measured
/// seek of 437ms on a 10-minute HLS stream, so it cannot fire on a seek that
/// is merely slow.
const SEEK_LOST_TICKS: u32 = 8;

/// Modifiers that mean a keypress belongs to somebody else.
///
/// Shift is deliberately absent: `M` is Shift+m.
const FOREIGN_MODIFIERS: gdk::ModifierType = gdk::ModifierType::CONTROL_MASK
    .union(gdk::ModifierType::ALT_MASK)
    .union(gdk::ModifierType::SUPER_MASK)
    .union(gdk::ModifierType::META_MASK)
    .union(gdk::ModifierType::HYPER_MASK);

/// Everything the player needs to know about one video embed.
#[derive(Debug, Clone)]
pub struct VideoSource {
    /// The HLS playlist Bluesky serves for this video.
    pub playlist_url: String,
    pub alt: Option<String>,
    /// Where to send someone when playback fails: the post on the web, never
    /// the playlist, which a browser renders as a page of raw text.
    ///
    /// `None` when no post URL is known. The error page then offers no browser
    /// button at all, which is honest, rather than quietly handing over the
    /// playlist and reproducing the raw-text bug this player exists to fix.
    pub fallback_url: Option<String>,
}

impl VideoSource {
    pub fn from_embed(video: &crate::atproto::VideoEmbed, fallback_url: Option<String>) -> Self {
        Self {
            playlist_url: video.playlist.clone(),
            alt: video.alt.clone(),
            fallback_url,
        }
    }
}

pub struct VideoPlayer {
    root: gtk4::Box,
    stack: gtk4::Stack,
    picture: gtk4::Picture,
    spinner: gtk4::Spinner,
    play_btn: gtk4::Button,
    position: gtk4::Scale,
    time_label: gtk4::Label,
    mute_btn: gtk4::ToggleButton,
    volume: gtk4::Scale,
    error_page: adw::StatusPage,
    error_detail: gtk4::Label,
    source: VideoSource,
    /// `None` before the pipeline is built and after it is torn down.
    playbin: RefCell<Option<gst::Element>>,
    /// Dropping this removes the bus watch, so it has to outlive the pipeline.
    bus_watch: RefCell<Option<gst::bus::BusWatchGuard>>,
    tick: RefCell<Option<glib::SourceId>>,
    seek_settle: RefCell<Option<glib::SourceId>>,
    /// The bus ERROR path reports from an idle callback, and that callback
    /// touches widgets, so `stop` has to be able to cancel it.
    error_report: RefCell<Option<glib::SourceId>>,
    pending_seek: Cell<Option<f64>>,
    /// Where the handle belongs while somebody other than the pipeline owns it.
    ///
    /// Between asking for a seek and the pipeline arriving, a position query
    /// still answers with the old time. Writing that back into the scale is
    /// what pulled the handle out from under the cursor mid-drag, so the tick
    /// shows this instead until the seek lands.
    seek_target: Cell<Option<f64>>,
    /// Whether a seek has been handed to the pipeline and not yet come back.
    ///
    /// Set when `seek_simple` is accepted, cleared by the ASYNC_DONE that
    /// belongs to it. ASYNC_DONE is also how preroll and every
    /// buffering-driven PAUSED/PLAYING transition announce themselves, so
    /// without this there is no way to tell a seek arriving from either of
    /// those, and the handle gets yanked to the playhead mid-drag.
    seek_inflight: Cell<bool>,
    /// Ticks spent waiting for that ASYNC_DONE, for [`SEEK_LOST_TICKS`].
    seek_waited: Cell<u32>,
    volume_save: RefCell<Option<glib::SourceId>>,
    /// Volume waiting to be written to disk, if any.
    pending_volume: Cell<Option<f64>>,
    /// What the person last asked for. Buffering only resumes PLAYING if set,
    /// so a rebuffer never restarts a video someone deliberately paused.
    wants_playing: Cell<bool>,
    duration: Cell<Option<f64>>,
}

impl VideoPlayer {
    /// Build a player and start loading `source`.
    ///
    /// Must be called on the GTK main thread: the sink hands out its paintable
    /// only there.
    pub fn new(source: VideoSource) -> Rc<Self> {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);

        let picture = gtk4::Picture::new();
        picture.set_content_fit(gtk4::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        if let Some(alt) = source
            .alt
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
        {
            picture.set_alternative_text(Some(alt));
        }

        let spinner = gtk4::Spinner::new();
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::Center);
        spinner.set_size_request(32, 32);
        spinner.start();

        let overlay = gtk4::Overlay::new();
        overlay.add_css_class("video-surface");
        overlay.set_child(Some(&picture));
        overlay.add_overlay(&spinner);
        overlay.set_hexpand(true);
        overlay.set_vexpand(true);

        let play_btn = gtk4::Button::from_icon_name("media-playback-pause-symbolic");
        play_btn.add_css_class("flat");
        play_btn.add_css_class("circular");
        play_btn.set_tooltip_text(Some("Pause"));

        let position = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 1.0, 1.0);
        position.set_hexpand(true);
        position.set_draw_value(false);
        // Nothing to scrub through until a duration comes back from the demuxer.
        position.set_sensitive(false);

        let time_label = gtk4::Label::new(Some("0:00 / 0:00"));
        time_label.add_css_class("dim-label");
        time_label.add_css_class("numeric");

        let mute_btn = gtk4::ToggleButton::new();
        mute_btn.set_icon_name("audio-volume-high-symbolic");
        mute_btn.add_css_class("flat");
        mute_btn.add_css_class("circular");
        mute_btn.set_tooltip_text(Some("Mute"));

        let stored_volume = AppSettings::load().video_volume.value();
        let volume = gtk4::Scale::with_range(
            gtk4::Orientation::Horizontal,
            VideoVolume::MIN,
            VideoVolume::MAX,
            VideoVolume::STEP,
        );
        volume.set_draw_value(false);
        volume.set_size_request(110, -1);
        volume.set_value(stored_volume);
        volume.set_tooltip_text(Some("Volume"));

        let controls = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        controls.add_css_class("video-controls");
        controls.append(&play_btn);
        controls.append(&position);
        controls.append(&time_label);
        controls.append(&mute_btn);
        controls.append(&volume);

        let video_page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        video_page.append(&overlay);
        video_page.append(&controls);

        let error_detail = gtk4::Label::new(None);
        error_detail.add_css_class("dim-label");
        error_detail.add_css_class("monospace");
        error_detail.add_css_class("caption");
        error_detail.set_wrap(true);
        // GStreamer's debug string is one long run of element paths joined by
        // ":" and "_", and Pango's default `Word` mode has nowhere to break in
        // that — the label's minimum width becomes the width of the whole run,
        // and past about 84 characters it drags the dialog wider than the
        // window it is presented over. `WordChar` breaks mid-token when a token
        // cannot fit; the character cap keeps the natural width to a readable
        // monospace measure so the details do not stretch the dialog either.
        error_detail.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        error_detail.set_max_width_chars(64);
        error_detail.set_selectable(true);
        error_detail.set_xalign(0.0);
        error_detail.set_margin_top(8);

        // The browser is offered, never taken: an earlier version launched one
        // from a timeout and people got tabs they had not asked for.
        let open_btn = gtk4::Button::with_label("Open in Browser");
        open_btn.add_css_class("suggested-action");
        open_btn.add_css_class("pill");
        open_btn.set_halign(gtk4::Align::Center);
        if let Some(url) = source.fallback_url.clone() {
            open_btn.connect_clicked(move |btn| {
                crate::ui::external::open_url(btn, &url, "post");
            });
        } else {
            // Nothing worth opening, so do not offer to open anything.
            open_btn.set_visible(false);
        }

        let details = gtk4::Expander::new(Some("Technical details"));
        details.set_child(Some(&error_detail));
        details.set_margin_top(12);

        let error_body = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        error_body.set_margin_start(12);
        error_body.set_margin_end(12);
        error_body.append(&open_btn);
        error_body.append(&details);

        let error_page = adw::StatusPage::new();
        error_page.set_icon_name(Some("dialog-warning-symbolic"));
        error_page.set_title("Video Couldn't Be Played");
        error_page.set_child(Some(&error_body));

        let stack = gtk4::Stack::new();
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        stack.add_named(&video_page, Some("video"));
        stack.add_named(&error_page, Some("error"));
        root.append(&stack);

        let player = Rc::new(Self {
            root,
            stack,
            picture,
            spinner,
            play_btn,
            position,
            time_label,
            mute_btn,
            volume,
            error_page,
            error_detail,
            source,
            playbin: RefCell::new(None),
            bus_watch: RefCell::new(None),
            tick: RefCell::new(None),
            seek_settle: RefCell::new(None),
            error_report: RefCell::new(None),
            pending_seek: Cell::new(None),
            seek_target: Cell::new(None),
            seek_inflight: Cell::new(false),
            seek_waited: Cell::new(0),
            volume_save: RefCell::new(None),
            pending_volume: Cell::new(None),
            wants_playing: Cell::new(false),
            duration: Cell::new(None),
        });

        player.connect_controls();
        // The stored level may be low or zero, and the button was built with
        // the loud icon.
        player.sync_volume_icon();
        player.start(stored_volume);
        player
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Attach Space / arrow / M shortcuts to `widget`.
    ///
    /// Left to the caller rather than wired up in `new`, because a player sat
    /// in a feed must not swallow the keys the feed itself uses.
    pub fn install_shortcuts(self: &Rc<Self>, widget: &impl IsA<gtk4::Widget>) {
        let key = gtk4::EventControllerKey::new();
        key.connect_key_pressed({
            let weak = Rc::downgrade(self);
            move |_, keyval, _, state| {
                let Some(player) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                // Once the pipeline has given up, the error page owns the
                // keyboard: swallowing Left and Right there would leave no way
                // to move focus between its button and its expander.
                if player.stack.visible_child_name().as_deref() != Some("video") {
                    return glib::Propagation::Proceed;
                }
                // Ctrl+M, Alt+Left and friends belong to somebody else.
                if state.intersects(FOREIGN_MODIFIERS) {
                    return glib::Propagation::Proceed;
                }
                match keyval {
                    gdk::Key::space => {
                        player.toggle_playback();
                        glib::Propagation::Stop
                    }
                    gdk::Key::Left => {
                        player.seek_relative(-KEY_SEEK_SECONDS);
                        glib::Propagation::Stop
                    }
                    gdk::Key::Right => {
                        player.seek_relative(KEY_SEEK_SECONDS);
                        glib::Propagation::Stop
                    }
                    gdk::Key::m | gdk::Key::M => {
                        player.toggle_mute();
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Proceed,
                }
            }
        });
        widget.as_ref().add_controller(key);
    }

    /// Tear the pipeline down. Idempotent, and safe to call from a dialog's
    /// close handler; without it the stream keeps pulling segments and playing
    /// audio after its window is gone.
    pub fn stop(&self) {
        self.wants_playing.set(false);
        if let Some(id) = self.tick.borrow_mut().take() {
            id.remove();
        }
        if let Some(id) = self.seek_settle.borrow_mut().take() {
            id.remove();
        }
        // Every source this player owns has to go, not just the timers: the
        // error report is an idle, and an idle that outlives the widget tree
        // sets a visible child on a stack nobody is showing any more.
        if let Some(id) = self.error_report.borrow_mut().take() {
            id.remove();
        }
        self.pending_seek.set(None);
        self.release_seek_hold();
        // The volume timer belongs to this player, so it goes with it — and a
        // level someone set a moment before closing is still worth keeping.
        if let Some(id) = self.volume_save.borrow_mut().take() {
            id.remove();
        }
        self.flush_volume();
        if let Some(playbin) = self.playbin.borrow_mut().take() {
            let _ = playbin.set_state(gst::State::Null);
        }
        self.bus_watch.borrow_mut().take();
        self.picture.set_paintable(gdk::Paintable::NONE);
    }

    pub fn toggle_playback(&self) {
        if self.wants_playing.get() {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn toggle_mute(&self) {
        self.mute_btn.set_active(!self.mute_btn.is_active());
    }

    /// Move `delta` seconds through the video, for keyboard shortcuts.
    pub fn seek_relative(&self, delta: f64) {
        let Some(playbin) = self.playbin() else {
            return;
        };
        let now = playbin
            .query_position::<gst::ClockTime>()
            .map(clock_seconds)
            .unwrap_or(0.0);
        self.seek_to(now + delta);
    }

    fn play(&self) {
        let Some(playbin) = self.playbin() else {
            return;
        };
        self.wants_playing.set(true);
        let _ = playbin.set_state(gst::State::Playing);
        self.sync_transport();
    }

    fn pause(&self) {
        let Some(playbin) = self.playbin() else {
            return;
        };
        self.wants_playing.set(false);
        let _ = playbin.set_state(gst::State::Paused);
        self.sync_transport();
    }

    fn playbin(&self) -> Option<gst::Element> {
        self.playbin.borrow().clone()
    }

    fn start(self: &Rc<Self>, volume: f64) {
        let pipeline = match media::build_pipeline(
            &self.source.playlist_url,
            media::linear_volume(volume),
            false,
        ) {
            Ok(p) => p,
            Err(e) => return self.fail(e),
        };

        self.picture.set_paintable(Some(&pipeline.paintable));

        let bus = pipeline
            .playbin
            .bus()
            .expect("playbin3 is a GstPipeline and always has a bus");
        let watch = bus.add_watch_local({
            let weak = Rc::downgrade(self);
            move |_, msg| match weak.upgrade() {
                Some(player) => player.on_bus_message(msg),
                None => glib::ControlFlow::Break,
            }
        });
        match watch {
            Ok(guard) => {
                self.bus_watch.replace(Some(guard));
            }
            Err(e) => return self.fail(MediaError::Init(e.to_string())),
        }

        self.playbin.replace(Some(pipeline.playbin));
        self.start_tick();
        self.play();
    }

    fn on_bus_message(self: &Rc<Self>, msg: &gst::Message) -> glib::ControlFlow {
        use gst::MessageView;

        let Some(playbin) = self.playbin() else {
            return glib::ControlFlow::Break;
        };

        match msg.view() {
            MessageView::Error(err) => {
                let error = MediaError::Pipeline {
                    message: err.error().to_string(),
                    debug: err.debug().map(|d| d.to_string()),
                };
                let _ = playbin.set_state(gst::State::Null);
                // Report from an idle callback rather than here: this closure is
                // the bus watch, and tearing down would drop its own guard
                // partway through a dispatch.
                let weak = Rc::downgrade(self);
                let id = glib::idle_add_local_once(move || {
                    if let Some(player) = weak.upgrade() {
                        // Forget the id before reporting: `fail` calls `stop`,
                        // and this source is mid-dispatch, so leaving it there
                        // would have `stop` remove a source that is already on
                        // its way out.
                        player.error_report.replace(None);
                        player.fail(error);
                    }
                });
                self.error_report.replace(Some(id));
                return glib::ControlFlow::Break;
            }
            MessageView::Warning(w) => {
                eprintln!("hangar: gstreamer warning: {} ({:?})", w.error(), w.debug());
            }
            MessageView::Buffering(b) => {
                // Percent runs backwards across an adaptive-bitrate switch and
                // arrives long before PLAYING is ever reached, so this gates on
                // the buffering mode and on what the person actually asked for
                // rather than trusting 100% to mean "go".
                if b.mode() == gst::BufferingMode::Stream {
                    let filling = b.percent() < 100;
                    self.set_busy(filling);
                    if filling {
                        let _ = playbin.set_state(gst::State::Paused);
                    } else if self.wants_playing.get() {
                        let _ = playbin.set_state(gst::State::Playing);
                    }
                }
            }
            MessageView::Eos(..) => {
                // Park at the start rather than looping: the dialog is somewhere
                // people go to watch one video, not a feed.
                self.wants_playing.set(false);
                let _ = playbin.set_state(gst::State::Paused);
                self.seek_to(0.0);
                self.sync_transport();
            }
            MessageView::StateChanged(sc) => {
                if sc.src().map(|s| s == &playbin).unwrap_or(false) {
                    if sc.current() == gst::State::Playing {
                        self.set_busy(false);
                    }
                    self.sync_transport();
                }
            }
            MessageView::DurationChanged(..) => {
                // The demuxer has revised its answer, so the cached one is
                // stale. This says nothing at all about any seek.
                self.duration.set(None);
                self.refresh_position();
            }
            MessageView::AsyncDone(..) => {
                // A flushing seek finishes with ASYNC_DONE, which is the point
                // the pipeline can answer for its new position again.
                //
                // Only for a seek, though. Preroll ends with one too — landing
                // at the moment the scrubber first becomes sensitive — and so
                // does every buffering-driven trip through PAUSED and back.
                // Releasing the handle for those is how a drag lost it to the
                // playhead partway through.
                //
                // A drag still in progress keeps the handle regardless. Pausing
                // mid-drag to look at a frame is long enough for the settle
                // timer to fire, so the drag resumes with that seek in flight;
                // its own ASYNC_DONE would then hand the scrubber back to the
                // playhead under the user's cursor. Measured at up to 3.5s of
                // travel lost, for any pause between roughly 150ms and 580ms.
                let dragging = self.seek_settle.borrow().is_some();
                if self.seek_inflight.get() && !dragging {
                    self.release_seek_hold();
                }
                self.duration.set(None);
                if !dragging {
                    self.refresh_position();
                }
            }
            MessageView::ClockLost(..) => {
                // The only documented recovery: drop to PAUSED so a new clock is
                // picked up on the way back to PLAYING.
                let _ = playbin.set_state(gst::State::Paused);
                if self.wants_playing.get() {
                    let _ = playbin.set_state(gst::State::Playing);
                }
            }
            _ => {}
        }

        glib::ControlFlow::Continue
    }

    fn connect_controls(self: &Rc<Self>) {
        self.play_btn.connect_clicked({
            let weak = Rc::downgrade(self);
            move |_| {
                if let Some(player) = weak.upgrade() {
                    player.toggle_playback();
                }
            }
        });

        self.position.connect_change_value({
            let weak = Rc::downgrade(self);
            move |_, _, value| {
                if let Some(player) = weak.upgrade() {
                    player.request_seek(value);
                }
                glib::Propagation::Proceed
            }
        });

        self.mute_btn.connect_toggled({
            let weak = Rc::downgrade(self);
            move |btn| {
                if let Some(player) = weak.upgrade() {
                    player.apply_mute(btn.is_active());
                }
            }
        });

        self.volume.connect_value_changed({
            let weak = Rc::downgrade(self);
            move |scale| {
                if let Some(player) = weak.upgrade() {
                    player.apply_volume(scale.value());
                }
            }
        });
    }

    fn apply_volume(self: &Rc<Self>, slider: f64) {
        if let Some(playbin) = self.playbin() {
            playbin.set_property("volume", media::linear_volume(slider));
        }
        // Reaching for the slider is how people unmute, so treat it that way.
        if slider > 0.0 && self.mute_btn.is_active() {
            self.mute_btn.set_active(false);
        }
        self.sync_volume_icon();
        self.persist_volume(slider);
    }

    /// Save the app-wide volume once the slider has been still for a moment.
    fn persist_volume(self: &Rc<Self>, slider: f64) {
        self.pending_volume.set(Some(slider));
        if let Some(id) = self.volume_save.borrow_mut().take() {
            id.remove();
        }
        let id = glib::timeout_add_local_once(VOLUME_SETTLE, {
            let weak = Rc::downgrade(self);
            move || {
                if let Some(player) = weak.upgrade() {
                    player.volume_save.replace(None);
                    player.flush_volume();
                }
            }
        });
        self.volume_save.replace(Some(id));
    }

    /// Write the coalesced volume out now, if one is still waiting.
    fn flush_volume(&self) {
        let Some(slider) = self.pending_volume.take() else {
            return;
        };
        let mut settings = AppSettings::load();
        settings.video_volume = VideoVolume(slider);
        if let Err(e) = settings.save() {
            eprintln!("hangar: could not save video volume: {e}");
        }
    }

    fn apply_mute(&self, muted: bool) {
        if let Some(playbin) = self.playbin() {
            playbin.set_property("mute", muted);
        }
        self.mute_btn
            .set_tooltip_text(Some(if muted { "Unmute" } else { "Mute" }));
        self.sync_volume_icon();
    }

    fn sync_volume_icon(&self) {
        let level = self.volume.value();
        let icon = if self.mute_btn.is_active() || level <= 0.0 {
            "audio-volume-muted-symbolic"
        } else if level < 0.34 {
            "audio-volume-low-symbolic"
        } else if level < 0.67 {
            "audio-volume-medium-symbolic"
        } else {
            "audio-volume-high-symbolic"
        };
        self.mute_btn.set_icon_name(icon);
    }

    fn sync_transport(&self) {
        let playing = self.wants_playing.get();
        self.play_btn.set_icon_name(if playing {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        });
        self.play_btn
            .set_tooltip_text(Some(if playing { "Pause" } else { "Play" }));
    }

    fn set_busy(&self, busy: bool) {
        self.spinner.set_visible(busy);
        if busy {
            self.spinner.start();
        } else {
            self.spinner.stop();
        }
    }

    fn start_tick(self: &Rc<Self>) {
        if self.tick.borrow().is_some() {
            return;
        }
        let id = glib::timeout_add_local(TICK, {
            let weak = Rc::downgrade(self);
            move || match weak.upgrade() {
                Some(player) => {
                    player.refresh_position();
                    glib::ControlFlow::Continue
                }
                None => glib::ControlFlow::Break,
            }
        });
        self.tick.replace(Some(id));
    }

    fn refresh_position(&self) {
        let Some(playbin) = self.playbin() else {
            return;
        };

        // HLS reports no duration until the demuxer has the playlist, so this
        // keeps asking instead of assuming the stream is unseekable.
        if self.duration.get().is_none()
            && let Some(total) = playbin
                .query_duration::<gst::ClockTime>()
                .map(clock_seconds)
            && total > 0.0
        {
            self.duration.set(Some(total));
            self.position.set_range(0.0, total);
            self.position.set_sensitive(true);
        }

        let actual = playbin
            .query_position::<gst::ClockTime>()
            .map(clock_seconds)
            .unwrap_or(0.0);
        let total = self.duration.get().unwrap_or(0.0);

        // Who owns the handle right now: the person, or the pipeline?
        //
        // The person owns it while the settle timer is armed, which is exactly
        // as long as change-value keeps arriving, and then on until the seek
        // that drag produced has landed. How *far* the handle is from the
        // playhead says nothing about either: a drag begins with the handle sat
        // on the playhead, and a drag that travels back past it is briefly
        // indistinguishable from a seek arriving. Reading proximity as arrival
        // is what snapped the handle back at the start of every drag.
        let dragging = self.seek_settle.borrow().is_some();
        if dragging {
            self.seek_waited.set(0);
        } else if self.seek_inflight.get() {
            self.seek_waited.set(self.seek_waited.get() + 1);
            if self.seek_waited.get() >= SEEK_LOST_TICKS {
                // No ASYNC_DONE, and none coming. One visible jump beats a
                // handle pinned somewhere the pipeline never reached.
                self.release_seek_hold();
            }
        }

        let position = match self.seek_target.get() {
            Some(target) if dragging || self.seek_inflight.get() => target,
            // Nothing left to hold it for.
            Some(_) => {
                self.release_seek_hold();
                actual
            }
            None => actual,
        };

        // Close to the end a position query can answer past the duration, and
        // "10:01 / 10:00" reads as a bug even though nothing is wrong.
        let shown = if total > 0.0 {
            position.clamp(0.0, total)
        } else {
            position.max(0.0)
        };

        if total > 0.0 {
            self.position.set_value(shown);
        }
        self.time_label.set_label(&format!(
            "{} / {}",
            format_clock(shown),
            format_clock(total)
        ));
    }

    /// Hand the handle back to the pipeline: nothing held, nothing outstanding.
    fn release_seek_hold(&self) {
        self.seek_target.set(None);
        self.seek_inflight.set(false);
        self.seek_waited.set(0);
    }

    fn request_seek(self: &Rc<Self>, seconds: f64) {
        self.pending_seek.set(Some(seconds));
        // Hold the handle where the drag put it, so the tick has something
        // truthful to draw while the pipeline is still somewhere else. Nothing
        // has been asked of the pipeline yet, so this is not a seek in flight;
        // the armed settle timer below is what keeps the tick off the handle.
        self.seek_target.set(Some(seconds));
        // Restart the clock on every sample. Keeping the first timer instead
        // throttled the drag to one flushing seek every SEEK_SETTLE, each one
        // costing a segment fetch, when the whole point is to issue exactly one
        // seek after the handle stops moving.
        if let Some(id) = self.seek_settle.borrow_mut().take() {
            id.remove();
        }
        let id = glib::timeout_add_local_once(SEEK_SETTLE, {
            let weak = Rc::downgrade(self);
            move || {
                let Some(player) = weak.upgrade() else {
                    return;
                };
                player.seek_settle.replace(None);
                if let Some(target) = player.pending_seek.take() {
                    player.seek_to(target);
                }
            }
        });
        self.seek_settle.replace(Some(id));
    }

    fn seek_to(&self, seconds: f64) {
        let Some(playbin) = self.playbin() else {
            return;
        };

        // Strictly inside the stream: see SEEK_END_EPSILON. Without a duration
        // yet there is nothing to clamp against, so only the floor applies.
        let target = match self.duration.get() {
            Some(total) if total > 0.0 => seconds.clamp(0.0, (total - SEEK_END_EPSILON).max(0.0)),
            _ => seconds.max(0.0),
        };

        // ACCURATE rather than KEY_UNIT. KEY_UNIT alone snaps to the keyframe
        // *before* the target -- measured on a 6-second-keyframe stream, asking
        // for 10s landed at 5.75s, so the handle jumped backwards after every
        // drag and a 5-second arrow press could rewind. Adding SNAP_NEAREST
        // fixes that but makes the end far worse: for any target past the
        // middle of the last segment it snaps *forward* onto the end of the
        // stream, which the demuxer refuses, and a refused flushing seek wedges
        // the pipeline permanently. ACCURATE never snaps, so it has neither
        // problem: measured landings are exact to the millisecond.
        let result = playbin.seek_simple(
            gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
            gst::ClockTime::from_nseconds((target * 1_000_000_000.0) as u64),
        );

        match result {
            Ok(()) => {
                self.seek_target.set(Some(target));
                // Held until the ASYNC_DONE that belongs to *this* seek, or
                // until SEEK_LOST_TICKS gives up on one ever arriving.
                self.seek_inflight.set(true);
                self.seek_waited.set(0);
            }
            Err(e) => {
                // Nothing moved, so stop claiming it did and let the next tick
                // put the handle back where the pipeline actually is.
                eprintln!("hangar: could not seek to {target:.3}s: {e}");
                self.release_seek_hold();
                self.refresh_position();
            }
        }
    }

    fn fail(&self, error: MediaError) {
        eprintln!("hangar: video playback failed: {}", error.detail());
        self.stop();
        self.set_busy(false);
        self.error_page.set_description(Some(&error.user_message()));
        self.error_detail.set_label(&error.detail());
        self.stack.set_visible_child_name("error");
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn clock_seconds(time: gst::ClockTime) -> f64 {
    time.nseconds() as f64 / 1_000_000_000.0
}

fn format_clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (hours, minutes, secs) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}
