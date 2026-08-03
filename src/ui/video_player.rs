// SPDX-License-Identifier: MPL-2.0

//! A video player driven by Hangar's own GStreamer pipeline.
//!
//! The widget is a self-contained `GtkBox`: picture, transport controls, and an
//! error page it switches to when the pipeline gives up. Keeping the chrome
//! here rather than in the dialog is what lets a post row embed the same player
//! inline without any of this being rewritten.
//!
//! [`Chrome`] is the one axis that differs between the two. A dialog is 900x620
//! and can hold a transport bar and an `AdwStatusPage`; a timeline row is as
//! wide as the window and sits in a scroller with no horizontal valve, so its
//! transport has to be an *unmeasured* overlay and its failures have to be
//! handed back to the row rather than drawn here. See [`Self::build_controls`].

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

/// What is being played, which decides how the player behaves around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// A Bluesky video: audio, a scrubber, and it stops at the end.
    Video,
    /// A GIF that reached us as an MP4. Silent, looping, no transport chrome —
    /// anything else would be a video player wrapped around a reaction image.
    Gif,
}

impl MediaKind {
    fn noun(self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Gif => "GIF",
        }
    }
}

/// Everything the player needs to know about one embed.
#[derive(Debug, Clone)]
pub struct VideoSource {
    /// What to hand `playbin3`: the HLS playlist for a video, an MP4 or WebM
    /// for a GIF.
    pub playlist_url: String,
    pub alt: Option<String>,
    /// Where to send someone when playback fails: the post on the web, never
    /// the playlist, which a browser renders as a page of raw text.
    ///
    /// `None` when no post URL is known. The error page then offers no browser
    /// button at all, which is honest, rather than quietly handing over the
    /// playlist and reproducing the raw-text bug this player exists to fix.
    pub fallback_url: Option<String>,
    pub kind: MediaKind,
}

impl VideoSource {
    pub fn from_embed(video: &crate::atproto::VideoEmbed, fallback_url: Option<String>) -> Self {
        Self {
            playlist_url: video.playlist.clone(),
            alt: video.alt.clone(),
            fallback_url,
            kind: MediaKind::Video,
        }
    }

    /// A GIF, which has no HLS playlist and no audio track.
    ///
    /// Separate from [`Self::from_embed`] because a GIF is not a
    /// [`crate::atproto::VideoEmbed`] at all — it arrives as an external link
    /// card and is recognised by [`crate::atproto::gif::detect`].
    pub fn from_gif(
        gif: &crate::atproto::GifEmbed,
        alt: Option<String>,
        fallback_url: Option<String>,
    ) -> Self {
        Self {
            playlist_url: gif.video_url.clone(),
            alt,
            fallback_url,
            kind: MediaKind::Gif,
        }
    }
}

/// Which chrome a player wears, and so how it lays out and how it fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chrome {
    /// A dialog of its own: transport bar under the picture, `AdwStatusPage` on
    /// failure, keyboard shortcuts if the caller installs them.
    Dialog,
    /// A timeline row: transport as an overlay strip, failure handed to the
    /// owner through [`PlayerConfig::on_fail`], never any key controller.
    Inline,
}

/// Everything about a player that is not the stream itself.
pub struct PlayerConfig {
    pub chrome: Chrome,
    /// Perceptual 0..=1. Passed in rather than read from disk here: inline
    /// players are built on scroll, and a settings file read per pipeline is a
    /// disk read per video somebody scrolled past.
    pub volume: f64,
    /// Applied before the pipeline ever reaches PLAYING, so a muted start
    /// cannot leak a burst of audio.
    pub muted: bool,
    /// Seconds to resume from, once the stream admits to having a duration.
    /// `0.0` starts at the beginning and issues no seek at all.
    pub start_at: f64,
    /// Called with a one-line message instead of showing an error page.
    ///
    /// How an inline player fails: the row drops the player, puts its thumbnail
    /// back and shows a compact banner. An `AdwStatusPage` has a minimum width
    /// well over 300px, and a row's normal child *is* measured, so rendering
    /// one inline would widen the whole window through a scroller that has no
    /// horizontal valve.
    ///
    /// The callback may drop the player. Every path into [`VideoPlayer::fail`]
    /// holds a strong `Rc` across the call for exactly that reason — the local
    /// upgrade in the bus-error idle, and the constructor's own binding when a
    /// pipeline fails to build. Do not call `fail` from anywhere that does not.
    pub on_fail: Option<Box<dyn Fn(String)>>,
    /// Called with the current position when the fullscreen button is pressed.
    /// `None` builds no such button, which is what the dialog wants — it is
    /// already the thing an inline player hands off *to*.
    pub on_expand: Option<Box<dyn Fn(f64)>>,
}

impl PlayerConfig {
    /// The preset the media viewer's dialog has always used.
    pub fn dialog() -> Self {
        Self {
            chrome: Chrome::Dialog,
            volume: AppSettings::load().video_volume.value(),
            muted: false,
            start_at: 0.0,
            on_fail: None,
            on_expand: None,
        }
    }
}

/// The assembled transport, and the two controls only inline builds.
struct Transport {
    controls: gtk4::Box,
    volume_menu: Option<gtk4::MenuButton>,
    expand_btn: Option<gtk4::Button>,
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
    /// The inline transport's volume control: a popover, so its slider's ~110px
    /// minimum width never lands in the strip. `None` under [`Chrome::Dialog`].
    volume_menu: Option<gtk4::MenuButton>,
    /// Hands the stream to the full viewer. `None` under [`Chrome::Dialog`].
    expand_btn: Option<gtk4::Button>,
    error_page: adw::StatusPage,
    error_detail: gtk4::Label,
    chrome: Chrome,
    on_fail: Option<Box<dyn Fn(String)>>,
    on_expand: Option<Box<dyn Fn(f64)>>,
    source: VideoSource,
    /// `None` before the pipeline is built and after it is torn down.
    playbin: RefCell<Option<gst::Element>>,
    /// Alive for exactly as long as the pipeline, so `media::live_pipelines`
    /// counts what is actually running. Taken in [`Self::stop`] beside the
    /// playbin it belongs to.
    pipeline_token: RefCell<Option<media::PipelineToken>>,
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
    /// Where to resume from, until the first duration arrives and it is used.
    ///
    /// Seeking before that point asks an HLS stream to seek before its demuxer
    /// has the playlist, which answers "unseekable" — so the resume has to wait
    /// for the one moment the stream is known to be ready, not for a timer.
    pending_start: Cell<Option<f64>>,
}

impl VideoPlayer {
    /// Build a player wearing `config`'s chrome and start loading `source`.
    ///
    /// Must be called on the GTK main thread: the sink hands out its paintable
    /// only there.
    pub fn new_with(source: VideoSource, config: PlayerConfig) -> Rc<Self> {
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

        let volume = gtk4::Scale::with_range(
            gtk4::Orientation::Horizontal,
            VideoVolume::MIN,
            VideoVolume::MAX,
            VideoVolume::STEP,
        );
        volume.set_draw_value(false);
        volume.set_size_request(110, -1);
        volume.set_value(config.volume);
        volume.set_tooltip_text(Some("Volume"));

        let chrome = config.chrome;
        let transport = Self::build_transport(
            chrome,
            &play_btn,
            &position,
            &time_label,
            &mute_btn,
            &volume,
            config.on_expand.is_some(),
        );
        // A GIF has nothing to scrub, nothing to mute and nowhere to stop, so
        // it gets no transport. The widgets are still built and still wired up
        // — the keyboard shortcuts and the tick both go through them, and a
        // hidden widget is a great deal less trouble than a set of Options.
        transport
            .controls
            .set_visible(source.kind != MediaKind::Gif);

        let video_page: gtk4::Widget = match chrome {
            Chrome::Dialog => {
                let page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                page.append(&overlay);
                page.append(&transport.controls);
                page.upcast()
            }
            Chrome::Inline => {
                // The transport goes *over* the picture, not under it: the row
                // already committed to a height derived from the video's aspect
                // ratio before this player existed, and a 40px bar beneath
                // would grow the embed past it.
                //
                // It is also what keeps the strip's width off the window.
                // `GtkOverlay` does not measure its overlay children —
                // `measure-overlay` defaults to false — so the strip's minimum
                // width never reaches the row, the `AdwClamp` or the
                // (Never, Automatic) scroller above it. Never call
                // `set_measure_overlay(true)` here.
                let page = gtk4::Overlay::new();
                page.set_hexpand(true);
                page.set_vexpand(true);
                page.set_child(Some(&overlay));
                page.add_overlay(&transport.controls);
                page.upcast()
            }
        };

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
        error_page.set_title(&format!("{} Couldn't Be Played", source.kind.noun()));
        error_page.set_child(Some(&error_body));

        let stack = gtk4::Stack::new();
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        stack.add_named(&video_page, Some("video"));
        // The status page is only ever put in a dialog's tree. An inline player
        // hands its failure back through `on_fail` instead, so the widget whose
        // minimum width would widen the window is not merely hidden inline —
        // it is not there. See `PlayerConfig::on_fail`.
        if chrome == Chrome::Dialog {
            stack.add_named(&error_page, Some("error"));
        }
        root.append(&stack);

        // A GIF is silent by definition; an autoplayed video starts muted when
        // the setting says so.
        let start_muted = config.muted || source.kind == MediaKind::Gif;

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
            volume_menu: transport.volume_menu,
            expand_btn: transport.expand_btn,
            error_page,
            error_detail,
            chrome,
            on_fail: config.on_fail,
            on_expand: config.on_expand,
            source,
            playbin: RefCell::new(None),
            pipeline_token: RefCell::new(None),
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
            pending_start: Cell::new((config.start_at > 0.0).then_some(config.start_at)),
        });

        player.connect_controls();
        // Setting the toggle rather than only muting the pipeline keeps the
        // button, the icon and the `m` shortcut agreeing with what is actually
        // coming out of the speakers.
        if start_muted {
            player.mute_btn.set_active(true);
        }
        // The stored level may be low or zero, and the button was built with
        // the loud icon.
        player.sync_volume_icon();
        player.start(config.volume, start_muted);
        player
    }

    /// Assemble the transport for `chrome` out of controls the caller owns.
    ///
    /// The two layouts differ only in arrangement, never in behaviour: the same
    /// `play_btn`, the same position `Scale` with its settle-coalesced seeks,
    /// the same mute toggle. What inline changes is width. Budgeted against the
    /// 495px content column measured in a 700px window: 34 (play) + ~40 (scale
    /// minimum) + ~70 (time) + 34 (volume) + 34 (fullscreen) + spacing, which
    /// is why the volume slider moves into a popover — inline it would have
    /// been 110px of the strip on its own.
    fn build_transport(
        chrome: Chrome,
        play_btn: &gtk4::Button,
        position: &gtk4::Scale,
        time_label: &gtk4::Label,
        mute_btn: &gtk4::ToggleButton,
        volume: &gtk4::Scale,
        expandable: bool,
    ) -> Transport {
        let controls = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);

        if chrome == Chrome::Dialog {
            controls.add_css_class("video-controls");
            controls.append(play_btn);
            controls.append(position);
            controls.append(time_label);
            controls.append(mute_btn);
            controls.append(volume);
            return Transport {
                controls,
                volume_menu: None,
                expand_btn: None,
            };
        }

        controls.add_css_class("video-inline-controls");
        controls.add_css_class("osd");
        controls.set_valign(gtk4::Align::End);
        controls.set_halign(gtk4::Align::Fill);
        // Clicking the strip's padding must not reach the row body, which would
        // open the thread. A bubble-phase gesture on the strip is walked
        // descendants-first, so it claims before the row's own gesture sees the
        // release — the same mechanism the quote card relies on.
        let claim = gtk4::GestureClick::new();
        claim.connect_released(|gesture, _, _, _| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
        });
        controls.add_controller(claim);

        // Bluesky caps video at three minutes, so "0:12 / 2:58" never truncates
        // — only a hypothetical hour-long stream would, and an ellipsis beats
        // an eleventh character of clock pushing the window wider.
        time_label.set_max_width_chars(11);
        time_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);

        let volume_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        volume_box.set_margin_top(6);
        volume_box.set_margin_bottom(6);
        volume_box.set_margin_start(6);
        volume_box.set_margin_end(6);
        volume_box.append(mute_btn);
        volume_box.append(volume);

        let popover = gtk4::Popover::new();
        popover.set_child(Some(&volume_box));

        let volume_menu = gtk4::MenuButton::new();
        volume_menu.set_icon_name("audio-volume-high-symbolic");
        volume_menu.add_css_class("flat");
        volume_menu.add_css_class("circular");
        volume_menu.set_popover(Some(&popover));
        volume_menu.set_tooltip_text(Some("Volume"));
        volume_menu.update_property(&[gtk4::accessible::Property::Label("Volume")]);

        // Only the scrubber expands; everything else keeps its natural width so
        // the strip's total stays the sum budgeted above.
        play_btn.set_hexpand(false);
        position.set_hexpand(true);
        time_label.set_hexpand(false);
        volume_menu.set_hexpand(false);

        controls.append(play_btn);
        controls.append(position);
        controls.append(time_label);
        controls.append(&volume_menu);

        let expand_btn = expandable.then(|| {
            let btn = gtk4::Button::from_icon_name("view-fullscreen-symbolic");
            btn.add_css_class("flat");
            btn.add_css_class("circular");
            btn.set_hexpand(false);
            btn.set_tooltip_text(Some("Open in Viewer"));
            btn.update_property(&[gtk4::accessible::Property::Label("Open in viewer")]);
            controls.append(&btn);
            btn
        });

        Transport {
            controls,
            volume_menu: Some(volume_menu),
            expand_btn,
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Attach Space / arrow / M shortcuts to `widget`.
    ///
    /// Left to the caller rather than wired up in `new`, because a player sat
    /// in a feed must not swallow the keys the feed itself uses. An inline
    /// player never calls this and installs no key controller at all: `Space`
    /// already activates whichever button or quote card has focus, `Left` and
    /// `Right` belong to the list and to the focused scrubber, and with several
    /// videos realized at once `M` has no unambiguous target. Everything an
    /// inline transport needs from the keyboard falls out of ordinary focusable
    /// widgets — Tab walks play, scrubber, volume, fullscreen; arrows scrub the
    /// focused `GtkScale` through the same `request_seek` path a drag uses.
    ///
    /// [`crate::ui::media_viewer::show_video`] is the only caller.
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
        // After NULL and after the bus watch, so the count only drops once the
        // pipeline really has nothing left running.
        self.bus_watch.borrow_mut().take();
        self.pipeline_token.borrow_mut().take();
        self.pending_start.set(None);
        self.picture.set_paintable(gdk::Paintable::NONE);
    }

    /// Whether the audio is currently off.
    ///
    /// The director reads this as a video is torn down: someone who unmuted an
    /// autoplay has said what they want the next one to do.
    pub fn is_muted(&self) -> bool {
        self.mute_btn.is_active()
    }

    /// The perceptual 0..=1 level the slider is sat at.
    ///
    /// Lets the director keep its cached volume current without a settings read
    /// per video, which is the whole reason the level is passed in rather than
    /// loaded here.
    pub fn volume_level(&self) -> f64 {
        self.volume.value()
    }

    /// Where the stream is now, in seconds. `0.0` when there is no pipeline.
    ///
    /// For handing a position to the full viewer, which is the only reason a
    /// caller outside this file needs it.
    pub fn position(&self) -> f64 {
        self.playbin()
            .and_then(|p| p.query_position::<gst::ClockTime>())
            .map(clock_seconds)
            .unwrap_or(0.0)
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

    /// `muted` is applied before the pipeline ever reaches PLAYING, so neither a
    /// GIF nor a muted autoplay can leak a burst of audio.
    fn start(self: &Rc<Self>, volume: f64, muted: bool) {
        let pipeline = match media::build_pipeline(
            &self.source.playlist_url,
            media::linear_volume(volume),
            muted,
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
        // Beside the playbin, and taken beside it in `stop`: that pairing is
        // the whole reason `media::live_pipelines` can be trusted.
        self.pipeline_token.replace(Some(pipeline.token));
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
                if self.source.kind == MediaKind::Gif {
                    // Loop, because a GIF that stops after one pass is not a
                    // GIF. `playbin3` has no loop property, so the only way is
                    // to seek back on EOS; `wants_playing` stays true so the
                    // seek is followed by PLAYING rather than parking.
                    //
                    // No timer involved, deliberately. This is the pipeline's
                    // own bus watch, which `stop` already drops with the
                    // player, so there is nothing here that can outlive the
                    // widget the way a `glib::timeout` would.
                    self.seek_to(0.0);
                    let _ = playbin.set_state(gst::State::Playing);
                } else {
                    // Park at the start rather than looping: the dialog is
                    // somewhere people go to watch one video, not a feed.
                    self.wants_playing.set(false);
                    let _ = playbin.set_state(gst::State::Paused);
                    self.seek_to(0.0);
                    self.sync_transport();
                }
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

        if let Some(btn) = self.expand_btn.as_ref() {
            btn.connect_clicked({
                let weak = Rc::downgrade(self);
                move |_| {
                    let Some(player) = weak.upgrade() else {
                        return;
                    };
                    // Read the position before handing over: the callback is
                    // going to tear this player down.
                    let at = player.position();
                    if let Some(cb) = player.on_expand.as_ref() {
                        cb(at);
                    }
                }
            });
        }
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
        // Inline, the toggle is inside a popover nobody has open. The button
        // that stands in for it has to say the same thing.
        if let Some(menu) = self.volume_menu.as_ref() {
            menu.set_icon_name(icon);
        }
    }

    fn sync_transport(&self) {
        let playing = self.wants_playing.get();
        self.play_btn.set_icon_name(if playing {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        });
        let label = if playing { "Pause" } else { "Play" };
        self.play_btn.set_tooltip_text(Some(label));
        self.play_btn
            .update_property(&[gtk4::accessible::Property::Label(label)]);
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
            // The first duration is the first moment the demuxer has the
            // playlist, and so the first moment a seek can land. Resuming any
            // earlier gets "unseekable" back from HLS and starts at zero
            // anyway; resuming from a timer would be the fifth instance of the
            // bug this file has had four times.
            if let Some(at) = self.pending_start.take() {
                self.seek_to(at);
            }
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
        // Inline, the owner is told and this player is on its way out; the
        // row puts its thumbnail back and says so in one line. Only a dialog
        // has room for the status page and its technical details.
        if let Some(cb) = self.on_fail.as_ref() {
            cb(error.user_message());
            return;
        }
        debug_assert_eq!(
            self.chrome,
            Chrome::Dialog,
            "an inline player has no error page to switch to"
        );
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
