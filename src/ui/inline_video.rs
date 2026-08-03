// SPDX-License-Identifier: MPL-2.0

//! One video playing in the feed, and the single timer that decides which one.
//!
//! # Invariants
//!
//! These are the whole design. Breaking any of them reintroduces a bug this
//! codebase has already had, so they are written down rather than implied.
//!
//! 1. **At most one GStreamer pipeline exists in the process at any time.**
//!    Inline playback and the media viewer's dialog both go through
//!    [`VideoDirector`], and [`VideoDirector::arm`] tears the previous one down
//!    *before* building the next. [`crate::media::live_pipelines`] counts them,
//!    and the tests at the bottom of this file assert the bound while forty
//!    videos are scrolled past.
//!
//! 2. **The director owns exactly one [`glib::SourceId`].** Every trigger —
//!    scroll, model change, slot registration, setting change, page switch —
//!    re-arms that same source rather than adding another.
//!    [`VideoDirector::drop`] removes it, and its closure holds a `Weak` so it
//!    can do nothing at all if the window is gone before it fires.
//!
//! 3. **Inline chrome adds zero timers.** No hover fade, no auto-hide. The
//!    transport is visible whenever a video is armed and hidden when it is not.
//!
//! 4. **Nothing else here arms a timeout.** The `TICK`, `SEEK_SETTLE` and
//!    `VOLUME_SETTLE` sources inside [`VideoPlayer`] are already weak and
//!    already cancelled by its idempotent `stop`, which its `Drop` calls.
//!
//! 5. **Only a mapped widget's rectangle is believed.** `compute_bounds`
//!    answers from a widget's last allocation, so every row of a page the
//!    `GtkStack` has hidden goes on reporting the rectangle it had when that
//!    page was on screen. Believing those had a dialog closing over another
//!    page start a video nobody could see, at a visible fraction of 1.00 that
//!    the release hysteresis could never take back.
//!    [`VideoDirector::geometry_of`] is the only place any of this is
//!    measured, and it treats an unmapped row as what it is: not visible.
//!
//!    A row `GtkListView` is holding in its pool needs no special case, and it
//!    is worth knowing why rather than adding one: a pooled row *is* mapped,
//!    and it does report a plausible rectangle at the top of the viewport —
//!    but its allocation is `0x0`, so [`visible_fraction`] divides into a
//!    height of zero and answers 0.0. It loses the election on its size, which
//!    is the honest reason, and the tests measure against what GTK paints so
//!    that this keeps being checked rather than assumed.
//!
//! 6. **A press of the play button is never refused for being on the wrong
//!    page.** The only thing that refuses is a dialog holding the one
//!    pipeline, and that hold is counted rather than a flag, so two dialogs
//!    cannot un-hold each other. Whether *autoplay* may run is decided by
//!    invariant 5, not by a cached idea of which page is on screen.
//!
//! With autoplay off — the default — a feed that nobody has pressed play in
//! arms no timer and builds no pipeline, so a row costs exactly what it cost
//! before this module existed: a `Picture` and a `Button`.

use crate::state::VideoAutoplay;
use crate::ui::video_player::{Chrome, PlayerConfig, VideoPlayer, VideoSource};
use gtk4::glib;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// How long scrolling has to settle before autoplay picks a video.
///
/// Short enough that stopping on a post feels immediate, long enough that
/// flinging through fifty posts builds zero pipelines. The whole point of the
/// delay is that a fast scroll must cost nothing at all.
const ELECTION_SETTLE: Duration = Duration::from_millis(200);

/// How much of a video has to be on screen before autoplay will start it.
///
/// A half-visible video playing at the edge of the viewport is noise, not a
/// feature.
const AUTOPLAY_FRACTION: f64 = 0.6;

/// How little of a video has to be left on screen before it is torn down.
///
/// Deliberately far below [`AUTOPLAY_FRACTION`]: the gap is what lets a video
/// keep playing while you scroll a little, instead of stopping and starting as
/// the wheel moves.
const RELEASE_FRACTION: f64 = 0.1;

/// Visible fractions closer than this count as equal, so the tie-break by
/// position decides rather than float noise.
const FRACTION_EPSILON: f64 = 1e-6;

/// Positions below this are not worth remembering: resuming a video half a
/// second in is indistinguishable from starting it.
const RESUME_FLOOR: f64 = 1.0;

/// Identifies one embed for the length of its slot's life.
pub type SlotId = u64;

/// Why a video is playing, which decides whether the election may take it away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// The election chose it. The election may unchoose it.
    Auto,
    /// Somebody pressed play. Scrolling a little does not undo that.
    Manual,
}

/// One slot the election is choosing between.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub id: SlotId,
    /// How much of the embed is inside the viewport, 0.0..=1.0.
    pub fraction: f64,
    /// Its top edge in viewport coordinates, for breaking ties.
    pub top: f64,
}

/// How much of a `height`-tall box starting at `top` is inside a `viewport`.
///
/// Extracted as a free function on plain numbers, not because it is long, but
/// because it is the part of the policy most likely to be wrong at an edge —
/// and a `GtkListView` cannot be persuaded to reproduce "straddling the bottom
/// while taller than the viewport" on demand.
pub fn visible_fraction(top: f64, height: f64, viewport: f64) -> f64 {
    if height <= 0.0 {
        return 0.0;
    }
    let visible = (top + height).min(viewport) - top.max(0.0);
    (visible / height).clamp(0.0, 1.0)
}

/// Pick the video autoplay should be showing, if any.
///
/// Most visible wins; equally visible, the higher one wins, because that is the
/// one being read. Below [`AUTOPLAY_FRACTION`] nothing wins at all — "no video"
/// is a perfectly good answer and is the right one for a feed scrolled to a
/// gap between two of them.
pub fn elect(candidates: &[Candidate]) -> Option<SlotId> {
    candidates
        .iter()
        .filter(|c| c.fraction >= AUTOPLAY_FRACTION)
        .reduce(|best, c| {
            if c.fraction > best.fraction + FRACTION_EPSILON {
                c
            } else if best.fraction > c.fraction + FRACTION_EPSILON {
                best
            } else if c.top < best.top {
                c
            } else {
                best
            }
        })
        .map(|c| c.id)
}

thread_local! {
    /// The one director, found by rows that have no other way to reach it.
    ///
    /// A `Weak`, never an `Rc`: the director must die with the window so that
    /// its `Drop` removes the election source. A `PostRow` is a `glib::Object`
    /// built by a factory with no window handle, and threading the director
    /// through `bind` would touch six call sites to express something that is
    /// true of the process rather than of any one row — there is one video.
    static DIRECTOR: RefCell<Weak<VideoDirector>> = const { RefCell::new(Weak::new()) };

    static NEXT_SLOT_ID: Cell<SlotId> = const { Cell::new(1) };
}

/// Make `director` the one every slot will find. The caller keeps the `Rc`.
pub fn set_director(director: &Rc<VideoDirector>) {
    DIRECTOR.with(|d| *d.borrow_mut() = Rc::downgrade(director));
}

/// The director, if a window built one and still holds it.
pub fn director() -> Option<Rc<VideoDirector>> {
    DIRECTOR.with(|d| d.borrow().upgrade())
}

fn next_slot_id() -> SlotId {
    NEXT_SLOT_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    })
}

/// One video embed in a row: a thumbnail, a play button, and — only while it is
/// the video that is playing — a [`VideoPlayer`].
///
/// The slot is owned by its `PostRow` and by nothing else. That is what makes
/// [`Drop`] a real guarantee rather than a hope: a row recycled, rebound or
/// destroyed drops its slot, and a dropped slot takes its pipeline with it even
/// if every other teardown path somehow failed to run.
pub struct VideoSlot {
    id: SlotId,
    /// The embed's outermost widget, and what visibility is measured on.
    root: gtk4::Overlay,
    /// Holds the thumbnail, and the player in its place while armed.
    frame: gtk4::Frame,
    thumb: gtk4::Picture,
    play_btn: gtk4::Button,
    error_banner: gtk4::Box,
    error_label: gtk4::Label,
    source: VideoSource,
    /// `None` unless this is the one video playing.
    player: RefCell<Option<Rc<VideoPlayer>>>,
    intent: Cell<Option<Intent>>,
    /// Set by [`Self::fail`] so [`Self::arm`] can tell a player that died
    /// during construction from one that is fine.
    failed: Cell<bool>,
}

impl VideoSlot {
    /// Take over an embed built by `PostRow::render_video`.
    ///
    /// The widgets are the caller's because the row wants them sized and
    /// clipped its own way; what this adds is the swap between thumbnail and
    /// player, the failure banner, and registration with the director.
    pub fn new(
        root: &gtk4::Overlay,
        frame: &gtk4::Frame,
        thumb: &gtk4::Picture,
        play_btn: &gtk4::Button,
        source: VideoSource,
    ) -> Rc<Self> {
        let error_label = gtk4::Label::new(None);
        // An inline failure must not widen the window. This lives in an overlay
        // child, which `GtkOverlay` does not measure, and it is capped and
        // ellipsized anyway so that stays true if it is ever moved.
        error_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        error_label.set_max_width_chars(28);

        let error_banner = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        error_banner.add_css_class("video-inline-error");
        error_banner.add_css_class("osd");
        error_banner.set_halign(gtk4::Align::Center);
        error_banner.set_valign(gtk4::Align::End);
        error_banner.set_margin_bottom(8);
        error_banner.set_visible(false);
        error_banner.append(&error_label);

        // Offered, never taken — and never the playlist, which a browser
        // renders as a page of raw text. No post URL means no button.
        if let Some(url) = source.fallback_url.clone() {
            let open = gtk4::Button::with_label("Open");
            open.add_css_class("flat");
            open.connect_clicked(move |btn| {
                crate::ui::external::open_url(btn, &url, "post");
            });
            error_banner.append(&open);
        }
        root.add_overlay(&error_banner);

        let slot = Rc::new(Self {
            id: next_slot_id(),
            root: root.clone(),
            frame: frame.clone(),
            thumb: thumb.clone(),
            play_btn: play_btn.clone(),
            error_banner,
            error_label,
            source,
            player: RefCell::new(None),
            intent: Cell::new(None),
            failed: Cell::new(false),
        });

        play_btn.connect_clicked({
            let weak = Rc::downgrade(&slot);
            move |_| {
                if let Some(slot) = weak.upgrade() {
                    slot.play_manual();
                }
            }
        });

        // The guard that covers everything unbind cannot: a `NavigationView`
        // pop, a `GtkStack` page switch, a thread page built into a plain box
        // and then thrown away. Connected once, to the slot's own root, so
        // there is no handler id to keep in step with arming.
        root.connect_unmap({
            let weak = Rc::downgrade(&slot);
            move |_| {
                if let Some(slot) = weak.upgrade() {
                    slot.release();
                }
            }
        });

        if let Some(director) = director() {
            director.register(&slot);
        }
        slot
    }

    pub fn id(&self) -> SlotId {
        self.id
    }

    pub fn intent(&self) -> Option<Intent> {
        self.intent.get()
    }

    /// Start this video because somebody asked for it.
    pub fn play_manual(self: &Rc<Self>) {
        match director() {
            Some(director) => director.arm(self, Intent::Manual),
            // No window, or a test: fall back to what this embed did before
            // inline playback existed rather than doing nothing at all.
            None => crate::ui::media_viewer::show_video(&self.root, self.source.clone()),
        }
    }

    /// Give up this video, whoever started it. Idempotent.
    pub fn release(self: &Rc<Self>) {
        if self.player.borrow().is_none() {
            return;
        }
        match director() {
            Some(director) => director.release_slot(self.id),
            None => self.disarm(),
        }
    }

    /// Where the video has got to, if it is playing.
    fn position(&self) -> Option<f64> {
        self.player.borrow().as_ref().map(|p| p.position())
    }

    /// Whether the person unmuted this one, so later autoplays can follow suit.
    fn is_muted(&self) -> bool {
        self.player
            .borrow()
            .as_ref()
            .map(|p| p.is_muted())
            .unwrap_or(true)
    }

    fn volume_level(&self) -> Option<f64> {
        self.player.borrow().as_ref().map(|p| p.volume_level())
    }

    /// Build a player, put it where the thumbnail was, and start it.
    ///
    /// Only the director calls this, and only right after releasing whatever
    /// was playing before.
    fn arm(self: &Rc<Self>, intent: Intent, volume: f64, muted: bool, start_at: f64) {
        self.disarm();
        self.error_banner.set_visible(false);
        self.failed.set(false);

        let on_fail: Box<dyn Fn(String)> = Box::new({
            let weak = Rc::downgrade(self);
            move |message| {
                if let Some(slot) = weak.upgrade() {
                    slot.fail(message);
                }
            }
        });
        let on_expand: Box<dyn Fn(f64)> = Box::new({
            let weak = Rc::downgrade(self);
            move |at| {
                if let Some(slot) = weak.upgrade() {
                    slot.expand(at);
                }
            }
        });

        let player = VideoPlayer::new_with(
            self.source.clone(),
            PlayerConfig {
                chrome: Chrome::Inline,
                volume,
                muted,
                start_at,
                on_fail: Some(on_fail),
                on_expand: Some(on_expand),
            },
        );

        // A pipeline can fail before the constructor returns — a missing
        // GStreamer element does exactly that — in which case `on_fail` has
        // already run and put the thumbnail back. Installing the player now
        // would cover that with a permanently blank frame.
        if self.failed.get() {
            return;
        }

        self.frame.set_child(Some(player.widget()));
        // The centre button has done its job and would otherwise sit on top of
        // the video it started.
        self.play_btn.set_visible(false);
        self.player.replace(Some(player));
        self.intent.set(Some(intent));
    }

    /// Put the thumbnail back and drop the pipeline. Idempotent.
    fn disarm(&self) {
        let player = self.player.borrow_mut().take();
        if let Some(player) = player {
            // `Drop` would call this anyway. Calling it here makes the teardown
            // happen at a point in the code somebody can put a breakpoint on,
            // rather than whenever the last `Rc` happens to go.
            player.stop();
        }
        self.frame.set_child(Some(&self.thumb));
        self.play_btn.set_visible(true);
        self.intent.set(None);
    }

    /// The pipeline gave up: thumbnail back, one line of explanation over it.
    fn fail(&self, message: String) {
        self.failed.set(true);
        self.disarm();
        self.error_label.set_label(&message);
        self.error_banner.set_visible(true);
        // Hand the budget back, or one broken video would be the only video
        // this session ever tried to play.
        if let Some(director) = director() {
            director.note_failed(self.id);
        }
    }

    /// Hand the stream to the full viewer, carrying the position with it.
    fn expand(self: &Rc<Self>, at: f64) {
        // Down before up: the viewer builds a pipeline of its own.
        self.release();
        crate::ui::media_viewer::show_video_at(&self.root, self.source.clone(), at);
    }
}

impl Drop for VideoSlot {
    fn drop(&mut self) {
        // The last line of defence, and the reason a leaked pipeline needs
        // *every* other guard to fail rather than any one of them: a row that
        // is recycled, rebound or destroyed drops its slot, and this runs.
        let player = self.player.borrow_mut().take();
        if let Some(player) = player {
            player.stop();
        }
        if let Some(director) = director() {
            director.forget(self.id);
        }
    }
}

/// Decides which video plays, and is the only thing in the process that may
/// start one.
///
/// Held by `HangarWindow`, found by rows through [`director`].
pub struct VideoDirector {
    slots: RefCell<Vec<(SlotId, Weak<VideoSlot>)>>,
    /// The one armed slot, if any.
    current: RefCell<Option<(SlotId, Weak<VideoSlot>)>>,
    /// Invariant 2: exactly one source, re-armed rather than added to.
    election: RefCell<Option<glib::SourceId>>,
    /// The main timeline's scroller. Autoplay is scoped to it, mirroring
    /// `hide_replies_in_feed`: threads, profiles and search wait to be asked.
    timeline: glib::WeakRef<gtk4::ScrolledWindow>,
    autoplay: Cell<VideoAutoplay>,
    /// Cached so arming does not read the settings file on every scroll.
    volume: Cell<f64>,
    /// Once you unmute an autoplayed video, later ones start unmuted too. A
    /// `Cell` and not a setting: it is a session mood, not a preference, and
    /// restarting returns to the polite default.
    unmuted_by_user: Cell<bool>,
    /// How many dialogs are holding the one pipeline right now.
    ///
    /// A count and not a flag, because the calls are not one pair: the viewer
    /// opens from a quoted video, from a GIF and from the inline transport's
    /// fullscreen button, from any page. With a flag, the first of two
    /// overlapping dialogs closing un-suspended the second one's, and the
    /// election then started an inline video underneath it.
    ///
    /// Deliberately *not* also "the feed is not on screen". That conflation
    /// is what made the play button inert everywhere except Home: refusing
    /// autoplay on a page nobody is looking at is policy, refusing a press of
    /// a button is a bug. Whether autoplay may run is now answered by
    /// measuring the rows, in [`Self::geometry_of`], which cannot be wrong
    /// about which page is on screen the way a cached flag can.
    dialog_holds: Cell<usize>,
    /// Where one video got to, so scrolling back to it does not start it over.
    /// One entry, deliberately: a map would grow for the length of the session.
    resume: RefCell<Option<(String, f64)>>,
}

impl VideoDirector {
    pub fn new(settings: &crate::state::AppSettings) -> Rc<Self> {
        Rc::new(Self {
            slots: RefCell::new(Vec::new()),
            current: RefCell::new(None),
            election: RefCell::new(None),
            timeline: glib::WeakRef::new(),
            autoplay: Cell::new(settings.video_autoplay),
            volume: Cell::new(settings.video_volume.value()),
            unmuted_by_user: Cell::new(false),
            dialog_holds: Cell::new(0),
            resume: RefCell::new(None),
        })
    }

    /// Name the scroller autoplay is allowed to elect inside.
    pub fn set_timeline_scroller(&self, scrolled: &gtk4::ScrolledWindow) {
        self.timeline.set(Some(scrolled));
    }

    pub fn autoplay(&self) -> VideoAutoplay {
        self.autoplay.get()
    }

    /// Apply a new autoplay mode. Does *not* refetch the feed: this changes
    /// behaviour, not contents, and refetching would throw away the scroll
    /// position for nothing.
    pub fn set_autoplay(self: &Rc<Self>, mode: VideoAutoplay) {
        self.autoplay.set(mode);
        if mode.enabled() {
            self.schedule_election();
            return;
        }
        // Off means off now, not after the next scroll — but a video somebody
        // pressed play on is theirs and not the setting's.
        let manual = self
            .current_slot()
            .map(|s| s.intent() == Some(Intent::Manual))
            .unwrap_or(false);
        if !manual {
            self.release_current();
        }
    }

    /// Re-read the app-wide volume, for after something else has written it.
    pub fn refresh_volume(&self) {
        self.volume
            .set(crate::state::AppSettings::load().video_volume.value());
    }

    /// A dialog is taking the one pipeline: stop, and stay stopped until it
    /// gives it back.
    ///
    /// Named for when to call it, and balanced by exactly one
    /// [`Self::dialog_closed`]. It was called `suspend`, and the page switch
    /// called it too — which is how a press of the play button came to be
    /// refused on five pages. A page that is not on screen is not a reason to
    /// refuse anything; it is a reason for the election to find nothing, which
    /// it now does by measuring.
    pub fn dialog_opened(&self) {
        self.dialog_holds.set(self.dialog_holds.get() + 1);
        self.release_current();
    }

    /// A dialog has given the pipeline back.
    ///
    /// Not "playback is fine now": the last hold coming off only removes this
    /// director's reason to refuse. What may play is a question about what is
    /// on screen, and the dialog never knew the answer, so the election is
    /// asked rather than assumed. `saturating_sub` because an unbalanced call
    /// must not wrap a counter into permanent suspension.
    pub fn dialog_closed(self: &Rc<Self>) {
        let held = self.dialog_holds.get();
        self.dialog_holds.set(held.saturating_sub(1));
        if self.dialog_holds.get() == 0 {
            self.schedule_election();
        }
    }

    /// The main stack switched pages.
    ///
    /// Which page it switched to does not matter, and that is the point: a
    /// page switch changes what is on screen without scrolling, without
    /// touching a model and without registering a slot, so nothing else would
    /// run an election at all — but the election is perfectly capable of
    /// deciding what a hidden feed may play, because every row of a hidden
    /// page is unmapped and measures as such. Leaving the feed stops its video
    /// on the way out through each slot's own `connect_unmap`, synchronously,
    /// which is a stronger guarantee than the flag this replaces: it covers a
    /// `NavigationView` push, which the stack never hears about.
    pub fn page_changed(self: &Rc<Self>) {
        self.schedule_election();
    }

    pub fn register(self: &Rc<Self>, slot: &Rc<VideoSlot>) {
        self.slots.borrow_mut().push((slot.id, Rc::downgrade(slot)));
        // So a row that has just been bound can win without waiting for a
        // scroll, which is what makes the first screen after login autoplay.
        self.schedule_election();
    }

    /// Drop a dead slot from the roll. Called from [`VideoSlot::drop`], where
    /// there is no `Rc` left to hand over.
    pub fn forget(&self, id: SlotId) {
        self.slots
            .borrow_mut()
            .retain(|(slot_id, _)| *slot_id != id);
        let current_matches = self
            .current
            .borrow()
            .as_ref()
            .map(|(current, _)| *current == id)
            .unwrap_or(false);
        if current_matches {
            // The slot is mid-drop and has already stopped its player; there is
            // nothing to release, only a claim to let go of.
            self.current.replace(None);
        }
    }

    /// Give up whichever video `id` names, if it is the one playing.
    pub fn release_slot(&self, id: SlotId) {
        if self.current_id() == Some(id) {
            self.release_current();
        }
    }

    /// A slot's pipeline died. Free the budget and let somebody else have it.
    fn note_failed(&self, id: SlotId) {
        if self.current_id() == Some(id) {
            // The slot has already torn itself down; only the claim is left.
            self.current.replace(None);
        }
    }

    /// Build a pipeline for `slot`, after taking down whatever was playing.
    pub fn arm(self: &Rc<Self>, slot: &Rc<VideoSlot>, intent: Intent) {
        if self.dialog_holds.get() > 0 {
            // A dialog owns the one pipeline. Manual is refused too, not just
            // Auto: the dialog's player is not the director's to take down, so
            // arming over it would be the two-decoder case invariant 1 exists
            // to make unreachable. A dialog covers the window, so there is no
            // play button behind it to press, and the state ends by calling
            // `dialog_closed`.
            //
            // This is the *only* refusal. A feed that is not the page on
            // screen used to land here too, which is what made the play button
            // inert on Profile, Likes, Search, Mentions and Activity: five
            // ordinary feeds of ordinary rows, all wired for inline playback,
            // where pressing play produced no pipeline, no dialog and no
            // message. Whether *autoplay* may run is decided by measuring the
            // rows, not by refusing here.
            return;
        }
        // Invariant 1, and the order is the invariant: down before up. Two
        // decoders alive at once is not slow, it is a bus ERROR that reaches
        // the user as "Video Couldn't Be Played".
        self.release_current();

        // Checked in release, not merely asserted in debug. If a pipeline ever
        // survives teardown — a teardown path added later that forgets, a slot
        // that stopped answering — this is the one moment the damage is still
        // one line long and attributable, rather than a hundred rows of scroll
        // later when the machine has run out of decode sessions.
        let survivors = crate::media::live_pipelines();
        if survivors > 0 {
            eprintln!(
                "hangar: {survivors} video pipeline(s) survived teardown; \
                 arming another risks a decoder the system will refuse"
            );
        }

        // Pressing play is unambiguous, so it is never muted. An autoplay is
        // muted only while nobody has said otherwise this session.
        let muted = intent == Intent::Auto
            && self.autoplay.get().starts_muted()
            && !self.unmuted_by_user.get();
        let start_at = self.resume_for(&slot.source.playlist_url);

        // Claimed before arming, not after: a pipeline that fails during
        // construction calls back into `note_failed`, and a claim written
        // afterwards would overwrite that with a slot that is already dead.
        self.current.replace(Some((slot.id, Rc::downgrade(slot))));
        slot.arm(intent, self.volume.get(), muted, start_at);
        if slot.intent().is_none() {
            // It failed on the way up and has already disowned itself.
            self.current.replace(None);
        }
    }

    fn current_id(&self) -> Option<SlotId> {
        self.current.borrow().as_ref().map(|(id, _)| *id)
    }

    fn current_slot(&self) -> Option<Rc<VideoSlot>> {
        let held = self.current.borrow();
        held.as_ref().and_then(|(_, weak)| weak.upgrade())
    }

    fn slot(&self, id: SlotId) -> Option<Rc<VideoSlot>> {
        let slots = self.slots.borrow();
        slots
            .iter()
            .find(|(slot_id, _)| *slot_id == id)
            .and_then(|(_, weak)| weak.upgrade())
    }

    /// Tear the current video down, remembering what it can teach the next one.
    fn release_current(&self) {
        // Taken in its own statement: `disarm` below can drop the last `Rc` to
        // a slot, whose `Drop` calls back into `forget`, which borrows this.
        let held = self.current.borrow_mut().take();
        let Some((_, weak)) = held else {
            return;
        };
        let Some(slot) = weak.upgrade() else {
            return;
        };

        if let Some(at) = slot.position().filter(|at| *at >= RESUME_FLOOR) {
            self.resume
                .replace(Some((slot.source.playlist_url.clone(), at)));
        }
        // Learn from what the person did with this one before it goes.
        if let Some(level) = slot.volume_level() {
            self.volume.set(level);
        }
        if slot.intent() == Some(Intent::Auto) && !slot.is_muted() {
            self.unmuted_by_user.set(true);
        }
        slot.disarm();
    }

    fn resume_for(&self, url: &str) -> f64 {
        let remembered = self.resume.borrow().clone();
        match remembered {
            Some((remembered_url, at)) if remembered_url == url => at,
            _ => 0.0,
        }
    }

    /// Where `slot` sits in the timeline: how much of it shows, and how far
    /// down. `None` when it is not in the timeline at all.
    ///
    /// `None` is not "invisible": it is "not something scrolling the feed has
    /// any opinion about". A video playing on a thread page, or on the profile
    /// page because somebody pressed play there, is taken down by unbind and
    /// unmap, never by an election it was never part of. Being *in* the feed
    /// and not on screen is a different answer — `Some(0.0, _)` — and the
    /// difference is what lets the release hysteresis fire.
    fn geometry_of(&self, slot: &VideoSlot) -> Option<(f64, f64)> {
        let scrolled = self.timeline.upgrade()?;
        // `compute_bounds` succeeds for any two widgets under the same window,
        // so the ancestry check is what actually scopes autoplay to the feed —
        // without it a thread page's row would compute a perfectly plausible
        // fraction against a scroller it is not in.
        if !slot.root.is_ancestor(&scrolled) {
            return None;
        }

        let viewport_height = f64::from(scrolled.height());
        // In the feed, and not on screen. Not a guess: the rows this covers
        // are exactly the ones whose rectangles cannot be believed.
        //
        // `compute_bounds` answers from the widget's last allocation, and a
        // row that is not mapped has not been allocated since it stopped being
        // visible — every row of a page the `GtkStack` has switched away from
        // keeps the rectangle it had when it *was* on screen. Ancestry stays
        // true and the numbers stay plausible, which is how a hidden page's
        // post reported itself perfectly visible, held the election against
        // everything actually on screen, and could never fall below
        // `RELEASE_FRACTION` to be let go of.
        let unseen = Some((0.0, viewport_height));
        if !scrolled.is_mapped() || !slot.root.is_mapped() {
            return unseen;
        }
        let Some(bounds) = slot.root.compute_bounds(&scrolled) else {
            return unseen;
        };
        // Checked against the viewport rather than assumed from ancestry.
        // `visible_fraction` answers for the vertical; this is what rules out
        // a rectangle that is beside the viewport rather than inside it.
        let viewport =
            gtk4::graphene::Rect::new(0.0, 0.0, scrolled.width() as f32, scrolled.height() as f32);
        if bounds.intersection(&viewport).is_none() {
            return unseen;
        }

        let top = f64::from(bounds.y());
        let fraction = visible_fraction(top, f64::from(bounds.height()), viewport_height);
        Some((fraction, top))
    }

    fn visible_fraction_of(&self, slot: &VideoSlot) -> Option<f64> {
        self.geometry_of(slot).map(|(fraction, _)| fraction)
    }

    fn candidates(&self) -> Vec<Candidate> {
        // Cloned rather than held: measuring touches widgets, and a borrow left
        // open across that is a borrow open across anything a signal decides to
        // do about it.
        let slots = self.slots.borrow().clone();
        slots
            .iter()
            .filter_map(|(id, weak)| {
                let slot = weak.upgrade()?;
                let (fraction, top) = self.geometry_of(&slot)?;
                Some(Candidate {
                    id: *id,
                    fraction,
                    top,
                })
            })
            .collect()
    }

    fn prune(&self) {
        self.slots.borrow_mut().retain(|(_, weak)| {
            // `strong_count` rather than `upgrade`: this must not resurrect a
            // slot for as long as it takes to drop the temporary.
            weak.strong_count() > 0
        });
    }

    /// Re-arm the one election source.
    ///
    /// Every trigger lands here: the timeline's `value-changed`, the model's
    /// `items-changed`, a slot registering, the autoplay setting changing, the
    /// main stack switching pages. None of them adds a source of its own.
    pub fn schedule_election(self: &Rc<Self>) {
        // With autoplay off and nothing playing there is no decision to make,
        // so scrolling arms no timer whatsoever.
        if !self.autoplay.get().enabled() && self.current.borrow().is_none() {
            return;
        }
        let previous = self.election.borrow_mut().take();
        if let Some(id) = previous {
            id.remove();
        }
        let id = glib::timeout_add_local_once(ELECTION_SETTLE, {
            let weak = Rc::downgrade(self);
            move || {
                let Some(director) = weak.upgrade() else {
                    return;
                };
                // Forget the id before running: this source is mid-dispatch and
                // finishes on its own, so leaving it there would have the next
                // `schedule_election` remove a source already on its way out.
                director.election.replace(None);
                director.run_election();
            }
        });
        self.election.replace(Some(id));
    }

    /// Decide what should be playing, and make that true.
    fn run_election(self: &Rc<Self>) {
        self.prune();
        if self.dialog_holds.get() > 0 {
            self.release_current();
            return;
        }

        if let Some(slot) = self.current_slot() {
            match self.visible_fraction_of(&slot) {
                // Not in the timeline at all: a thread page, or a video
                // somebody pressed play on while the profile page was up.
                // Not the election's to take away — unbind and unmap own it.
                // A row that *is* in the feed and not on screen answers 0.0
                // instead and falls to the release check below, which is what
                // stops a video the list has scrolled a long way past.
                None => return,
                Some(fraction) => {
                    if fraction < RELEASE_FRACTION {
                        self.release_current();
                    } else if slot.intent() == Some(Intent::Manual) {
                        // Deliberately started and still on screen. Losing that
                        // to a nudge of the wheel is what makes other clients
                        // infuriating.
                        return;
                    }
                }
            }
        }

        if !self.autoplay.get().enabled() {
            self.release_current();
            return;
        }

        let candidates = self.candidates();
        let Some(winner) = elect(&candidates) else {
            // Nothing is visible enough to *start*, which is not a reason to
            // stop what is already playing — the release check above is what
            // does that, and the gap between the two thresholds is the whole
            // hysteresis. Without this a video scrolled to half visible, with
            // nothing eligible to replace it, would be torn down and then
            // rebuilt the moment it came back.
            return;
        };
        if self.current_id() == Some(winner) {
            return;
        }
        let Some(slot) = self.slot(winner) else {
            return;
        };
        self.arm(&slot, Intent::Auto);
    }
}

impl Drop for VideoDirector {
    fn drop(&mut self) {
        // Invariant 2. The closure holds a `Weak` and would do nothing anyway,
        // but a source left on the main context after its owner is gone is the
        // exact shape of bug this codebase has had four times.
        let election = self.election.borrow_mut().take();
        if let Some(id) = election {
            id.remove();
        }
        let held = self.current.borrow_mut().take();
        if let Some((_, weak)) = held
            && let Some(slot) = weak.upgrade()
        {
            slot.disarm();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media;
    // For `AdwWindow::set_content`; the gtk4 traits come in through `super`.
    use crate::ui::video_player::MediaKind;
    use crate::ui::with_gtk;
    use libadwaita::prelude::*;

    #[test]
    fn visible_fraction_handles_every_edge() {
        // Entirely above the viewport, and entirely below it.
        assert_eq!(visible_fraction(-200.0, 200.0, 800.0), 0.0);
        assert_eq!(visible_fraction(800.0, 200.0, 800.0), 0.0);
        // Straddling the top edge, and the bottom.
        assert_eq!(visible_fraction(-50.0, 100.0, 800.0), 0.5);
        assert_eq!(visible_fraction(750.0, 100.0, 800.0), 0.5);
        // Wholly inside.
        assert_eq!(visible_fraction(100.0, 200.0, 800.0), 1.0);
        // Taller than the viewport: never more than all of it, and the fraction
        // is of the video, so a 1000px video in an 800px viewport tops out at
        // 0.8 and can never reach AUTOPLAY_FRACTION's sibling case of 1.0.
        assert_eq!(visible_fraction(-100.0, 1000.0, 800.0), 0.8);
        // A row that has not been allocated yet.
        assert_eq!(visible_fraction(0.0, 0.0, 800.0), 0.0);
        assert_eq!(visible_fraction(0.0, -10.0, 800.0), 0.0);
    }

    #[test]
    fn election_prefers_the_most_visible_candidate() {
        let winner = elect(&[
            Candidate {
                id: 1,
                fraction: 0.7,
                top: 0.0,
            },
            Candidate {
                id: 2,
                fraction: 0.95,
                top: 300.0,
            },
            Candidate {
                id: 3,
                fraction: 0.61,
                top: 600.0,
            },
        ]);
        assert_eq!(winner, Some(2));
    }

    #[test]
    fn election_ignores_below_threshold() {
        assert_eq!(
            elect(&[
                Candidate {
                    id: 1,
                    fraction: 0.59,
                    top: 0.0
                },
                Candidate {
                    id: 2,
                    fraction: 0.0,
                    top: 400.0
                },
            ]),
            None,
            "a feed scrolled to the gap between two videos should play neither"
        );
        assert_eq!(elect(&[]), None);
    }

    #[test]
    fn election_breaks_ties_by_reading_order() {
        let winner = elect(&[
            Candidate {
                id: 7,
                fraction: 1.0,
                top: 420.0,
            },
            Candidate {
                id: 9,
                fraction: 1.0,
                top: 20.0,
            },
        ]);
        assert_eq!(winner, Some(9), "the higher of two equals is the one read");
    }

    /// Build the widgets `PostRow::render_video` builds, minus the row.
    fn slot_widgets() -> (gtk4::Overlay, gtk4::Frame, gtk4::Picture, gtk4::Button) {
        let overlay = gtk4::Overlay::new();
        let frame = gtk4::Frame::new(None);
        let thumb = gtk4::Picture::new();
        thumb.set_can_shrink(true);
        frame.set_child(Some(&thumb));
        overlay.set_child(Some(&frame));
        let play_btn = gtk4::Button::from_icon_name("media-playback-start-symbolic");
        overlay.add_overlay(&play_btn);
        (overlay, frame, thumb, play_btn)
    }

    fn test_source(n: usize) -> VideoSource {
        VideoSource {
            // A URI playbin3 will accept and never play. The pipeline is real —
            // that is the point, it is what gets counted — but nothing here
            // runs a main loop, so it never gets as far as reporting the error.
            playlist_url: format!("file:///nonexistent/hangar-test-{n}.mp4"),
            alt: None,
            fallback_url: None,
            kind: MediaKind::Video,
        }
    }

    fn new_slot(n: usize) -> Rc<VideoSlot> {
        let (overlay, frame, thumb, play_btn) = slot_widgets();
        // The widgets are kept alive by the slot's clones of them.
        VideoSlot::new(&overlay, &frame, &thumb, &play_btn, test_source(n))
    }

    /// Scrolling a feed past a great many videos must never accumulate
    /// pipelines. This is the measurement, not an argument that it is fine.
    ///
    /// The counter is process-wide, but every test that can touch it runs
    /// through `with_gtk`, which serialises them onto one worker thread.
    #[test]
    fn scrolling_past_many_videos_never_accumulates_pipelines() {
        with_gtk(scrolling_past_many_videos_never_accumulates_pipelines_body);
    }

    fn scrolling_past_many_videos_never_accumulates_pipelines_body() {
        const POSTS: usize = 40;

        let settings = crate::state::AppSettings {
            video_autoplay: VideoAutoplay::Muted,
            ..Default::default()
        };
        let director = VideoDirector::new(&settings);
        set_director(&director);

        assert_eq!(
            media::live_pipelines(),
            0,
            "something left a pipeline running before this test started"
        );

        let mut peak = 0usize;
        let mut armed = 0usize;
        // One slot at a time reaching the middle of the viewport, which is what
        // scrolling a feed of video posts amounts to.
        for n in 0..POSTS {
            let slot = new_slot(n);
            director.arm(&slot, Intent::Auto);
            let live = media::live_pipelines();
            peak = peak.max(live);
            if slot.player.borrow().is_some() {
                armed += 1;
                assert_eq!(live, 1, "post {n} armed and did not leave exactly one");
            }
            assert!(live <= 1, "post {n} of {POSTS} left {live} pipelines alive");
            // The row scrolls out and the ListView hands it to the next post.
            slot.release();
            assert_eq!(
                media::live_pipelines(),
                0,
                "post {n} kept a pipeline after its row was recycled"
            );
        }

        assert!(
            armed > 0,
            "no pipeline was ever built, so this proved nothing \
             (GStreamer plugins missing?)"
        );
        eprintln!(
            "pipelines: {POSTS} posts armed and released, {armed} pipelines built, \
             peak {peak} live, {} live at the end",
            media::live_pipelines()
        );
        assert_eq!(peak, 1);
        assert_eq!(media::live_pipelines(), 0);
    }

    /// Dropping the slot is the guarantee every other guard is a shortcut for.
    #[test]
    fn a_dropped_slot_takes_its_pipeline_with_it() {
        with_gtk(a_dropped_slot_takes_its_pipeline_with_it_body);
    }

    fn a_dropped_slot_takes_its_pipeline_with_it_body() {
        let director = VideoDirector::new(&crate::state::AppSettings::default());
        set_director(&director);

        let slot = new_slot(0);
        director.arm(&slot, Intent::Manual);
        assert_eq!(media::live_pipelines(), 1, "manual play builds a pipeline");
        assert_eq!(director.current_id(), Some(slot.id()));

        // No release_video, no unbind, no unmap: just the row going away, which
        // is the case where nothing else can save us.
        drop(slot);
        assert_eq!(media::live_pipelines(), 0);
        assert_eq!(
            director.current_id(),
            None,
            "the director went on claiming a slot that no longer exists"
        );
    }

    /// An inline player must leave the feed's keys alone.
    ///
    /// `Space` already activates whichever button or quote card has focus,
    /// `Left`/`Right` belong to the list and to a focused scrubber, and with
    /// several videos realized at once `M` has no unambiguous target. So an
    /// inline player installs no key controller of its own — every key it needs
    /// falls out of ordinary focusable widgets.
    #[test]
    fn an_inline_player_installs_no_key_controller() {
        with_gtk(an_inline_player_installs_no_key_controller_body);
    }

    fn an_inline_player_installs_no_key_controller_body() {
        let director = VideoDirector::new(&crate::state::AppSettings::default());
        set_director(&director);

        let slot = new_slot(0);
        director.arm(&slot, Intent::Manual);
        assert!(
            slot.player.borrow().is_some(),
            "nothing was armed to inspect"
        );

        let mut found = Vec::new();
        collect_key_controllers(slot.root.clone().upcast(), &mut found);
        assert!(
            found.is_empty(),
            "an inline player put key controllers on {found:?}; a controller on \
             anything but a control of GTK's own is one that answers keys \
             belonging to the feed"
        );

        slot.release();
    }

    // ------------------------------------------------------------------
    // The feed harness.
    //
    // Everything above this line measures policy on plain numbers, and none
    // of it can catch a widget that lies about where it is — only a live
    // `GtkListView` in a mapped window does that. So the tests below build
    // one: real rows, a real scroller, a real page switch, and a real press
    // of the play button.
    //
    // They put a window on screen for a second or two, and they skip
    // themselves, loudly, where the session will not map one.
    // ------------------------------------------------------------------

    /// Posts in the harness feed. Enough that half a million pixels of scroll
    /// lands in the middle of it rather than at the end.
    const HARNESS_POSTS: u32 = 3000;

    /// How a feed's list is put into its scroller.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Layout {
        /// What every feed in `window.rs` does today: the list inside an
        /// `AdwClamp` — or, on the profile page, inside a box and an overlay —
        /// none of which is a `GtkScrollable`, so the scroller wraps the lot
        /// in a `GtkViewport` and the list is allocated its whole height.
        /// Nothing is ever recycled: see
        /// `the_shipped_clamp_layout_never_recycles_a_row`.
        Clamped,
        /// The list handed straight to the scroller, which is the arrangement
        /// `release_video_on_unbind` and every recycling comment in this file
        /// are written for: `GtkListView` keeps a pool of realized rows and
        /// binds the visible ones.
        Recycling,
    }

    /// A post whose only embed is a video, named so that a slot can be traced
    /// back to the row and the post it belongs to.
    fn harness_post(name: &str) -> crate::atproto::Post {
        crate::atproto::Post {
            uri: format!("at://did:plc:test/app.bsky.feed.post/{name}"),
            cid: "cid".into(),
            author: crate::atproto::Profile {
                did: "did:plc:test".into(),
                handle: "someone.bsky.social".into(),
                display_name: Some("Someone".into()),
                avatar: None,
                banner: None,
                description: None,
                followers_count: None,
                following_count: None,
                posts_count: None,
                viewer_following: None,
                viewer_followed_by: None,
            },
            text: format!("post {name}"),
            created_at: "2026-01-01T00:00:00Z".into(),
            indexed_at: "2026-01-01T00:00:00Z".into(),
            like_count: None,
            repost_count: None,
            reply_count: None,
            embed: Some(crate::atproto::Embed::Video(crate::atproto::VideoEmbed {
                // The slot's identity in every measurement below. No
                // thumbnail: three thousand rows must not fetch three
                // thousand images.
                playlist: format!("file:///nonexistent/{name}.mp4"),
                thumbnail: None,
                alt: Some("a cat".into()),
                aspect_ratio: Some((16, 9)),
            })),
            viewer_like: None,
            viewer_repost: None,
            repost_reason: None,
            reply_context: None,
        }
    }

    /// One page of the app: a scroller full of video posts, with the same
    /// factory wiring `HangarWindow` gives all five of its feeds.
    fn harness_feed(
        page: &'static str,
        posts: u32,
        layout: Layout,
    ) -> (gtk4::ScrolledWindow, gtk4::ListView) {
        let model = gtk4::gio::ListStore::new::<gtk4::StringObject>();
        for i in 0..posts {
            model.append(&gtk4::StringObject::new(&format!("{page}:{i}")));
        }

        let factory = gtk4::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() {
                list_item.set_child(Some(&crate::ui::post_row::PostRow::new()));
            }
        });
        factory.connect_bind(|_, item| {
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(name) = list_item
                    .item()
                    .and_downcast::<gtk4::StringObject>()
                    .map(|s| s.string())
                && let Some(row) = list_item
                    .child()
                    .and_downcast::<crate::ui::post_row::PostRow>()
            {
                row.bind(&harness_post(&name));
                // How a row is traced back to its post below.
                row.set_widget_name(&name);
            }
        });
        // The production wiring itself, not a copy of it.
        crate::ui::HangarWindow::release_video_on_unbind(&factory);

        let list = gtk4::ListView::new(Some(gtk4::NoSelection::new(Some(model))), Some(factory));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        // The timeline's own policy: no horizontal valve.
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        match layout {
            Layout::Clamped => {
                let clamp = libadwaita::Clamp::new();
                clamp.set_maximum_size(800);
                clamp.set_tightening_threshold(600);
                clamp.set_child(Some(&list));
                scrolled.set_child(Some(&clamp));
            }
            Layout::Recycling => scrolled.set_child(Some(&list)),
        }
        (scrolled, list)
    }

    /// A window shaped like the app's: `AdwWindow`, so a dialog presented over
    /// it goes into its dialog host and can be counted where it lands.
    fn harness_window(content: &impl IsA<gtk4::Widget>) -> libadwaita::Window {
        let window = libadwaita::Window::new();
        window.set_default_size(700, 900);
        window.set_content(Some(content));
        window.present();
        window
    }

    /// Pump the main context for `ms`, which is what gives GTK its frame
    /// clock, its layout pass and its list recycling, and what lets the
    /// election's own 200ms source fire.
    fn settle(ms: u64) {
        let ctx = glib::MainContext::default();
        let deadline = std::time::Instant::now() + Duration::from_millis(ms);
        while std::time::Instant::now() < deadline {
            while ctx.iteration(false) {}
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Scroll to `offset` and let the list catch up.
    ///
    /// Not one `set_value`: a `GtkScrolledWindow` clamps a new value into
    /// `[0, upper - page_size]`, and `upper` here is whatever the list has
    /// realized so far, which grows as it realizes more. A single set against
    /// a three thousand row feed therefore lands short — sometimes at zero —
    /// and the sweep silently measures the top of the feed over and over.
    /// Setting it again after each settle lets the value follow `upper` up.
    fn scroll_to(scrolled: &gtk4::ScrolledWindow, offset: f64) {
        let adjustment = scrolled.vadjustment();
        for _ in 0..8 {
            adjustment.set_value(offset);
            settle(40);
            if (adjustment.value() - offset).abs() < 1.0 {
                break;
            }
        }
    }

    fn descendants(widget: &gtk4::Widget, out: &mut Vec<gtk4::Widget>) {
        let mut child = widget.first_child();
        while let Some(next) = child {
            out.push(next.clone());
            descendants(&next, out);
            child = next.next_sibling();
        }
    }

    /// Every realized row of `list`, whether or not it is on screen.
    fn rows_of(list: &gtk4::ListView) -> Vec<crate::ui::post_row::PostRow> {
        let mut all = Vec::new();
        descendants(list.upcast_ref(), &mut all);
        all.into_iter()
            .filter_map(|w| w.downcast::<crate::ui::post_row::PostRow>().ok())
            .collect()
    }

    /// The post number a harness row or slot is showing.
    fn post_number(name: &str) -> Option<u32> {
        name.rsplit_once(':')?
            .1
            .trim_end_matches(".mp4")
            .parse()
            .ok()
    }

    fn slot_post_number(slot: &VideoSlot) -> Option<u32> {
        post_number(&slot.source.playlist_url)
    }

    /// Every realized row, as (post, top edge in the scroller, on screen),
    /// ordered by where the row claims to be.
    ///
    /// "On screen" is deliberately `is_mapped()` **and** a non-empty
    /// allocation. A `GtkListView` keeps a pool of realized rows it has not
    /// bound to a position: those are mapped, they are visible, they are
    /// drawable, and `compute_bounds` answers for them with a rectangle at
    /// `y = 0` — the top of the viewport — because they have never been
    /// allocated anywhere else. Counting them made this helper name a post
    /// GTK is not painting at all, which is a lie in the *opposite* direction
    /// from the one these tests are hunting: it invents an on-screen post to
    /// measure the director against. Their allocation is `0x0`, and that is
    /// what tells them apart from a row the list has actually placed.
    fn row_geometry(
        list: &gtk4::ListView,
        scrolled: &gtk4::ScrolledWindow,
    ) -> Vec<(u32, f64, bool)> {
        let mut seen: Vec<(u32, f64, bool)> = rows_of(list)
            .iter()
            .filter_map(|row| {
                let number = post_number(&row.widget_name())?;
                let top = row
                    .compute_bounds(scrolled)
                    .map(|b| f64::from(b.y()))
                    .unwrap_or(f64::MAX);
                let allocated = row.width() > 0 && row.height() > 0;
                Some((number, top, row.is_mapped() && allocated))
            })
            .collect();
        seen.sort_by(|a, b| a.1.total_cmp(&b.1));
        seen
    }

    /// The first post actually on screen: the topmost placed row that reaches
    /// into the viewport.
    fn topmost_mapped(list: &gtk4::ListView, scrolled: &gtk4::ScrolledWindow) -> Option<u32> {
        let height = f64::from(scrolled.height());
        row_geometry(list, scrolled)
            .into_iter()
            .find(|(_, top, on_screen)| *on_screen && *top + 100.0 > 0.0 && *top < height)
            .map(|(number, _, _)| number)
    }

    /// Every post GTK is actually painting in the viewport, by hit test.
    ///
    /// Ground truth for "on screen", and the reason it is not computed from
    /// `compute_bounds`: an allocation is what a widget *claims*, and this
    /// asks GTK which widget is really at a point instead. A row that is not
    /// drawn there cannot be picked there, whatever its rectangle says.
    ///
    /// Sampled down the middle of the viewport rather than at one point,
    /// because the tallest embed here is about a third of the window and the
    /// question is "is the armed post among the ones on screen", not "is it
    /// the one under the cursor".
    fn posts_rendered_in(scrolled: &gtk4::ScrolledWindow) -> Vec<u32> {
        let width = f64::from(scrolled.width());
        let height = f64::from(scrolled.height());
        let mut seen = Vec::new();
        let mut y = 10.0;
        while y < height {
            if let Some(hit) = scrolled.pick(width / 2.0, y, gtk4::PickFlags::DEFAULT) {
                let mut cursor = Some(hit);
                while let Some(widget) = cursor {
                    if widget.is::<crate::ui::post_row::PostRow>()
                        && let Some(number) = post_number(&widget.widget_name())
                        && !seen.contains(&number)
                    {
                        seen.push(number);
                        break;
                    }
                    cursor = widget.parent();
                }
            }
            y += 50.0;
        }
        seen
    }

    /// The centre play button of a row that is on screen, and its post.
    fn a_play_button_on_screen(list: &gtk4::ListView) -> Option<(u32, gtk4::Button)> {
        rows_of(list)
            .into_iter()
            .filter(|r| r.is_mapped())
            .find_map(|row| {
                let number = post_number(&row.widget_name())?;
                let mut all = Vec::new();
                descendants(row.upcast_ref(), &mut all);
                let button = all
                    .into_iter()
                    .filter_map(|w| w.downcast::<gtk4::Button>().ok())
                    .find(|b| b.tooltip_text().as_deref() == Some("Play video"))?;
                Some((number, button))
            })
    }

    /// How many `AdwDialog`s are open over `window` — the fallback path's
    /// footprint, counted rather than assumed.
    fn open_dialogs(window: &libadwaita::Window) -> usize {
        let mut all = Vec::new();
        descendants(window.upcast_ref(), &mut all);
        all.iter().filter(|w| w.is::<libadwaita::Dialog>()).count()
    }

    /// Give up if the session will not actually put a window on screen, which
    /// is the one thing none of this can measure without.
    fn window_is_usable(window: &libadwaita::Window, scrolled: &gtk4::ScrolledWindow) -> bool {
        if window.is_mapped() && scrolled.height() > 400 {
            return true;
        }
        eprintln!(
            "skipping: the window never mapped (mapped {}, viewport {}px), so there is \
             no geometry to measure",
            window.is_mapped(),
            scrolled.height()
        );
        false
    }

    /// FAIL A: the election must never play a post that is not on screen.
    ///
    /// The property, stated against what GTK actually paints: whatever the
    /// director has armed must be one of the posts in the viewport. That is
    /// the thing worth guaranteeing — under `WithSound` a post that is not on
    /// screen is audio with no visible source — and it holds however the list
    /// arrived at its current state.
    ///
    /// It is checked with [`posts_rendered_in`], by hit test, rather than by
    /// comparing against the topmost row's rectangle. An allocation is a
    /// claim; a hit test is an observation. This test used to make the
    /// comparison and was intermittently red because of it — see
    /// [`row_geometry`] for the pool rows that made the claim worthless.
    ///
    /// The measurement is the point: this scrolls a three thousand post feed
    /// and prints, at every checkpoint, which post the director armed against
    /// the posts actually on screen.
    ///
    /// Both layouts are swept: `Recycling` because it is the one with a row
    /// pool, and `Clamped` because it is the one that ships.
    #[test]
    fn the_election_never_plays_a_post_that_is_off_screen() {
        with_gtk(the_election_never_plays_a_post_that_is_off_screen_body);
    }

    fn the_election_never_plays_a_post_that_is_off_screen_body() {
        for layout in [Layout::Recycling, Layout::Clamped] {
            the_election_never_plays_a_post_that_is_off_screen_for(layout);
        }
    }

    fn the_election_never_plays_a_post_that_is_off_screen_for(layout: Layout) {
        let settings = crate::state::AppSettings {
            video_autoplay: VideoAutoplay::Muted,
            ..Default::default()
        };
        let director = VideoDirector::new(&settings);
        set_director(&director);

        let (scrolled, list) = harness_feed("home", HARNESS_POSTS, layout);
        director.set_timeline_scroller(&scrolled);
        let window = harness_window(&scrolled);
        settle(600);
        if !window_is_usable(&window, &scrolled) {
            window.destroy();
            return;
        }

        let mut off_screen = Vec::new();
        let mut plays = 0usize;
        let mut offset = 0.0;
        while offset <= 480_000.0 {
            scroll_to(&scrolled, offset);
            settle(120);
            director.run_election();

            let armed = director.current_slot();
            let armed_post = armed.as_ref().and_then(|s| slot_post_number(s));
            let armed_mapped = armed.as_ref().map(|s| s.root.is_mapped());
            let fraction = armed.as_ref().and_then(|s| director.visible_fraction_of(s));
            let topmost = topmost_mapped(&list, &scrolled);
            let rendered = posts_rendered_in(&scrolled);
            let geometry = row_geometry(&list, &scrolled);
            let realized = geometry.len();
            let on_screen_rows = geometry.iter().filter(|(_, _, m)| *m).count();

            let line = format!(
                "{layout:?} offset {offset:>7.0}: {realized:>3} rows realized, \
                 {on_screen_rows} placed, topmost post {topmost:?}, on screen {rendered:?}, \
                 ARMED post {armed_post:?} (row mapped {armed_mapped:?}, fraction {:.2}), \
                 live pipelines {}",
                fraction.unwrap_or(f64::NAN),
                crate::media::live_pipelines(),
            );
            eprintln!("{line}");

            if let Some(armed_post) = armed_post {
                plays += 1;
                // The whole property: whatever is playing is a post GTK is
                // painting in the viewport. Not "near the topmost row" — that
                // compares one claimed rectangle against another, and the
                // claims are what cannot be trusted.
                if armed_mapped != Some(true) || !rendered.contains(&armed_post) {
                    off_screen.push(line);
                }
            }
            offset += 20_000.0;
        }

        window.destroy();
        drop(director);
        settle(100);

        assert!(
            plays > 0,
            "{layout:?}: autoplay never armed anything, so this measured nothing"
        );
        assert!(
            off_screen.is_empty(),
            "{layout:?}: the election played {} post(s) GTK was not painting in the \
             viewport:\n{}",
            off_screen.len(),
            off_screen.join("\n")
        );
        assert_eq!(
            crate::media::live_pipelines(),
            0,
            "{layout:?}: the harness left a pipeline running"
        );
    }

    /// FAIL A, the half that needs no scrolling: a page that is not on screen.
    ///
    /// Every row of a hidden page keeps the rectangle it had when the page was
    /// visible, so a feed the `GtkStack` has switched away from is a feed full
    /// of rows that all claim to be perfectly visible. This is the shipped
    /// clamp layout, on the shipped page switch, and it is why a dialog
    /// closing over another page was able to start a video nobody could see.
    #[test]
    fn a_hidden_feed_has_no_visible_videos() {
        with_gtk(a_hidden_feed_has_no_visible_videos_body);
    }

    fn a_hidden_feed_has_no_visible_videos_body() {
        let settings = crate::state::AppSettings {
            video_autoplay: VideoAutoplay::Muted,
            ..Default::default()
        };
        let director = VideoDirector::new(&settings);
        set_director(&director);

        let (home, home_list) = harness_feed("home", 60, Layout::Clamped);
        let (profile, _) = harness_feed("profile", 60, Layout::Clamped);
        let stack = gtk4::Stack::new();
        stack.add_named(&home, Some("home"));
        stack.add_named(&profile, Some("profile"));
        director.set_timeline_scroller(&home);

        let window = harness_window(&stack);
        settle(600);
        if !window_is_usable(&window, &home) {
            window.destroy();
            return;
        }

        let on_home = topmost_mapped(&home_list, &home);
        eprintln!("on the home page: topmost mapped post {on_home:?}");

        stack.set_visible_child_name("profile");
        leaving_home(&director);
        settle(300);

        let hidden: Vec<_> = row_geometry(&home_list, &home)
            .into_iter()
            .take(4)
            .collect();
        let claiming: Vec<_> = director
            .candidates()
            .into_iter()
            .filter(|c| c.fraction >= AUTOPLAY_FRACTION)
            .collect();
        eprintln!(
            "with the stack on the profile page: home rows (post, top, mapped) {hidden:?}, \
             {} of {} home slots still claim to be at least {AUTOPLAY_FRACTION} visible, \
             election winner {:?}",
            claiming.len(),
            director.slots.borrow().len(),
            elect(&director.candidates()),
        );

        window.destroy();
        drop(director);
        settle(100);

        assert!(
            claiming.is_empty(),
            "{} rows of a page that is not on screen still report themselves visible; \
             the first few are {:?}",
            claiming.len(),
            claiming.iter().take(3).collect::<Vec<_>>()
        );
    }

    /// What `HangarWindow`'s stack handler does on a page switch, named for
    /// the direction these tests use it in. It exercises the shipped call, so
    /// it cannot drift into testing a copy of the policy.
    fn leaving_home(director: &Rc<VideoDirector>) {
        director.page_changed();
    }

    /// FAIL B: pressing play must do something on every page.
    ///
    /// Profile, Likes, Search, Mentions and Activity are ordinary feeds of
    /// ordinary `PostRow`s, and all five were given `release_video_on_unbind`
    /// so their videos could play in place. Suspension is there to stop
    /// *autoplay* deciding things about a feed nobody is looking at; a person
    /// pressing a button is not a decision the director gets to veto.
    #[test]
    fn pressing_play_off_the_home_page_plays_the_video() {
        with_gtk(pressing_play_off_the_home_page_plays_the_video_body);
    }

    fn pressing_play_off_the_home_page_plays_the_video_body() {
        // The shipped default: autoplay off. Nothing here is autoplay.
        let director = VideoDirector::new(&crate::state::AppSettings::default());
        set_director(&director);

        let (home, _) = harness_feed("home", 60, Layout::Clamped);
        let (profile, profile_list) = harness_feed("profile", 60, Layout::Clamped);
        let stack = gtk4::Stack::new();
        stack.add_named(&home, Some("home"));
        stack.add_named(&profile, Some("profile"));
        director.set_timeline_scroller(&home);

        let window = harness_window(&stack);
        settle(600);
        if !window_is_usable(&window, &home) {
            window.destroy();
            return;
        }

        stack.set_visible_child_name("profile");
        leaving_home(&director);
        settle(400);

        let found = a_play_button_on_screen(&profile_list);
        let Some((post, play)) = found else {
            window.destroy();
            panic!("no play button on any mapped profile row, so nothing could be pressed");
        };
        eprintln!(
            "on the profile page: {} rows realized, pressing play on post {post}",
            rows_of(&profile_list).len()
        );

        let mut outcomes = Vec::new();
        for press in 1..=3 {
            play.emit_clicked();
            // Measured before settling: the pipeline is built inside `arm`,
            // and this URL fails on the bus a few milliseconds later, which
            // is a different question from whether pressing play did anything.
            let pipelines = crate::media::live_pipelines();
            let dialogs = open_dialogs(&window);
            outcomes.push((pipelines, dialogs));
            eprintln!("  press {press}: {pipelines} pipeline(s), {dialogs} dialog(s)");
            assert!(
                pipelines <= 1,
                "press {press} left {pipelines} pipelines alive, breaking invariant 1"
            );
            settle(200);
        }

        window.destroy();
        drop(director);
        settle(100);

        assert!(
            outcomes.iter().all(|(p, d)| *p == 1 || *d == 1),
            "pressing play on a page that is not Home did nothing at all: \
             (pipelines, dialogs) per press = {outcomes:?}"
        );
        // And it played in place rather than falling back to the viewer. These
        // are ordinary feeds of the same rows the thread page has always
        // played inline, with the same unbind and unmap teardown behind them,
        // so opening a dialog here would be an inconsistency rather than a
        // safety net.
        assert!(
            outcomes.iter().all(|(p, d)| *p == 1 && *d == 0),
            "pressing play off Home opened a dialog instead of playing in \
             place: (pipelines, dialogs) per press = {outcomes:?}"
        );
        assert_eq!(
            crate::media::live_pipelines(),
            0,
            "the harness left a pipeline running"
        );
    }

    /// FAIL C: a dialog closing must not wake a feed nobody is looking at, and
    /// two dialogs must not un-suspend each other.
    ///
    /// `suspend` and `resume` are a balanced pair, so a count and not a flag.
    /// And `resume` re-runs the election rather than declaring playback fine:
    /// what may play is a question about what is on screen, which the dialog
    /// never knew the answer to.
    #[test]
    fn a_dialog_closing_does_not_wake_a_hidden_feed() {
        with_gtk(a_dialog_closing_does_not_wake_a_hidden_feed_body);
    }

    fn a_dialog_closing_does_not_wake_a_hidden_feed_body() {
        for mode in [VideoAutoplay::Muted, VideoAutoplay::WithSound] {
            let settings = crate::state::AppSettings {
                video_autoplay: mode,
                ..Default::default()
            };
            let director = VideoDirector::new(&settings);
            set_director(&director);

            let (home, _) = harness_feed("home", 60, Layout::Clamped);
            let (profile, _) = harness_feed("profile", 60, Layout::Clamped);
            let stack = gtk4::Stack::new();
            stack.add_named(&home, Some("home"));
            stack.add_named(&profile, Some("profile"));
            director.set_timeline_scroller(&home);

            let window = harness_window(&stack);
            settle(600);
            if !window_is_usable(&window, &home) {
                window.destroy();
                return;
            }

            // Every measurement below is taken the instant the election has
            // decided. `run_election` is the function the 200ms source
            // dispatches, called here so the sample lands on the decision:
            // `file:///nonexistent` posts its error on the bus a few
            // milliseconds later, and what GStreamer made of the URL is a
            // different question from what the director chose to play.
            let after_election = |director: &Rc<VideoDirector>| {
                director.run_election();
                crate::media::live_pipelines()
            };

            // The feed autoplays while it is the page on screen: without this
            // the rest proves nothing, because nothing was ever playing.
            let playing_on_home = after_election(&director);
            settle(200);

            stack.set_visible_child_name("profile");
            leaving_home(&director);
            let after_leaving = after_election(&director);
            settle(200);

            // A quoted video or a GIF opens the viewer from any page, which is
            // what makes this reachable: `show_video_at` suspends, and
            // `connect_closed` resumes.
            director.dialog_opened();
            director.dialog_closed();
            let after_the_dialog = after_election(&director);
            settle(200);

            eprintln!(
                "{mode:?}: playing while home is on screen {playing_on_home}, \
                 after leaving home {after_leaving}, after a dialog opened and closed \
                 over the profile page {after_the_dialog}"
            );

            // Back on Home, the balance itself: two dialogs need two closes.
            stack.set_visible_child_name("home");
            director.page_changed();
            settle(200);
            let back_home = after_election(&director);
            settle(200);
            director.dialog_opened();
            director.dialog_opened();
            director.dialog_closed();
            let one_of_two_closed = after_election(&director);
            settle(200);
            director.dialog_closed();
            let both_closed = after_election(&director);
            settle(200);
            eprintln!(
                "{mode:?}: back on home {back_home}, after two dialogs opened and one \
                 closed {one_of_two_closed}, after the second closed {both_closed}"
            );

            window.destroy();
            drop(director);
            settle(100);

            assert_eq!(
                playing_on_home, 1,
                "{mode:?}: autoplay did not play anything on the page that *is* on \
                 screen, so nothing below this proves anything"
            );
            assert_eq!(
                after_leaving, 0,
                "{mode:?}: leaving the feed left a video playing"
            );
            assert_eq!(
                after_the_dialog, 0,
                "{mode:?}: closing a dialog started a video on a feed that is not \
                 on screen"
            );
            assert_eq!(
                one_of_two_closed, 0,
                "{mode:?}: one dialog closing let go of two dialogs' hold"
            );
            assert_eq!(
                both_closed, 1,
                "{mode:?}: the second dialog closing did not give the visible feed \
                 its video back"
            );
            assert_eq!(
                back_home, 1,
                "{mode:?}: coming back to the feed did not resume autoplay"
            );
            assert_eq!(
                crate::media::live_pipelines(),
                0,
                "{mode:?}: the harness left a pipeline running"
            );
        }
    }

    /// The shipped layout, measured rather than assumed.
    ///
    /// Every feed in `window.rs` puts its `GtkListView` inside an `AdwClamp`,
    /// which is not a `GtkScrollable`, so the scroller wraps the pair in a
    /// `GtkViewport` and hands the list its entire height. The list then stops
    /// realizing rows at a bound of its own — everything past that is blank —
    /// and never recycles one, so `release_video_on_unbind` never fires and
    /// every realized row holds a slot for the length of the session.
    ///
    /// This is not something this change fixes. It is recorded because it is
    /// the reason the recycled-row half of FAIL A cannot bite the shipped
    /// build today, and because the number it prints is the number of video
    /// slots a page holds.
    #[test]
    fn the_shipped_clamp_layout_never_recycles_a_row() {
        with_gtk(the_shipped_clamp_layout_never_recycles_a_row_body);
    }

    fn the_shipped_clamp_layout_never_recycles_a_row_body() {
        let director = VideoDirector::new(&crate::state::AppSettings::default());
        set_director(&director);

        let (scrolled, list) = harness_feed("home", HARNESS_POSTS, Layout::Clamped);
        director.set_timeline_scroller(&scrolled);
        let window = harness_window(&scrolled);
        settle(600);
        if !window_is_usable(&window, &scrolled) {
            window.destroy();
            return;
        }

        let realized = rows_of(&list).len();
        let mut blank_from = None;
        let mut offset = 0.0;
        while offset <= 480_000.0 {
            scrolled.vadjustment().set_value(offset);
            settle(80);
            if blank_from.is_none() && topmost_mapped(&list, &scrolled).is_none() {
                blank_from = Some(offset);
            }
            offset += 20_000.0;
        }

        eprintln!(
            "clamped layout: {realized} of {HARNESS_POSTS} rows realized (each holding a \
             video slot), list allocated {}px inside a {}px viewport, nothing on screen \
             from offset {:?}px",
            list.height(),
            scrolled.height(),
            blank_from,
        );

        window.destroy();
        drop(director);
        settle(100);
        assert_eq!(crate::media::live_pipelines(), 0);
    }

    /// Walk the tree collecting key controllers on everything *we* built.
    ///
    /// GTK's own controls are skipped, and skipping them is the point rather
    /// than an exemption: a `GtkButton`'s key controller is what makes Space
    /// activate it, a `GtkRange`'s is what makes the arrows scrub, a
    /// `GtkPopover`'s is what makes Escape close it. Those are the mechanism
    /// this design relies on instead of shortcuts. What must have none are the
    /// boxes, overlays, frames and stacks in between, because a controller
    /// there would answer for the whole embed.
    fn collect_key_controllers(widget: gtk4::Widget, out: &mut Vec<String>) {
        let is_gtk_control = widget.is::<gtk4::Button>()          // and GtkToggleButton
            || widget.is::<gtk4::MenuButton>()
            || widget.is::<gtk4::Range>()                         // and GtkScale
            || widget.is::<gtk4::Popover>();
        if !is_gtk_control {
            for controller in widget.observe_controllers().into_iter().flatten() {
                if controller.is::<gtk4::EventControllerKey>() {
                    out.push(widget.type_().name().to_string());
                }
            }
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            collect_key_controllers(next.clone(), out);
            child = next.next_sibling();
        }
    }
}
