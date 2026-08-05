// SPDX-License-Identifier: MPL-2.0

//! Inline video in the feed.
//!
//! At most one GStreamer pipeline per process: [`VideoDirector::arm`] tears the
//! old one down before building the next, and
//! [`crate::media::live_pipelines`] counts them. The election runs on a single
//! [`glib::SourceId`] re-armed by every trigger; its closure holds a `Weak` and
//! [`VideoDirector::drop`] removes it.
//!
//! `compute_bounds` answers from a widget's last allocation, so a row on a
//! hidden GtkStack page still reports the rectangle it had when visible, hence
//! the `is_mapped()` check in [`VideoDirector::geometry_of`]. Pooled
//! GtkListView rows are mapped but allocated 0x0, so [`visible_fraction`]
//! returns 0.0 for them.
//!
//! Autoplay is refused for an unmapped row. The play button is refused only
//! while a dialog holds the pipeline, which is a count because two dialogs can
//! overlap.

use crate::state::VideoAutoplay;
use crate::ui::video_player::{Chrome, PlayerConfig, VideoPlayer, VideoSource};
use gtk4::glib;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// Scroll settle before autoplay elects. A fast scroll must build no pipelines.
const ELECTION_SETTLE: Duration = Duration::from_millis(200);

/// Minimum visible fraction before autoplay starts a video.
const AUTOPLAY_FRACTION: f64 = 0.6;

/// Teardown threshold. The gap from [`AUTOPLAY_FRACTION`] is the hysteresis.
const RELEASE_FRACTION: f64 = 0.1;

/// Fractions within this are equal; position breaks the tie.
const FRACTION_EPSILON: f64 = 1e-6;

/// Below this, don't record a resume position.
const RESUME_FLOOR: f64 = 1.0;

/// Identifies one embed for the length of its slot's life.
pub type SlotId = u64;

/// Why a video is playing, which decides whether the election may take it away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Elected by autoplay; the election may take it back.
    Auto,
    /// Started by the play button; the election leaves it alone.
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

/// Fraction of a `height`-tall box at `top` inside a `viewport`.
///
/// A free function so the edge cases are unit-testable without a GtkListView.
pub fn visible_fraction(top: f64, height: f64, viewport: f64) -> f64 {
    if height <= 0.0 {
        return 0.0;
    }
    let visible = (top + height).min(viewport) - top.max(0.0);
    (visible / height).clamp(0.0, 1.0)
}

/// Pick the video autoplay should be showing, if any.
///
/// Most visible wins; ties go to the higher one. Nothing below
/// [`AUTOPLAY_FRACTION`] wins.
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
    /// The window's director. A `PostRow` comes from a factory with no window
    /// handle, so this is how rows reach it.
    ///
    /// Weak, so the director dies with the window and its `Drop` removes the
    /// election source.
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

/// One video embed: thumbnail, play button, and a [`VideoPlayer`] while armed.
///
/// Owned only by its `PostRow`, so [`Drop`] always tears the pipeline down.
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
    /// Take over the embed built by `PostRow::render_video`: thumbnail/player
    /// swap, failure banner, director registration.
    pub fn new(
        root: &gtk4::Overlay,
        frame: &gtk4::Frame,
        thumb: &gtk4::Picture,
        play_btn: &gtk4::Button,
        source: VideoSource,
    ) -> Rc<Self> {
        let error_label = gtk4::Label::new(None);
        // GtkOverlay does not measure overlay children. Capped and ellipsized
        // anyway so an inline failure cannot widen the window.
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

        // The post URL. A browser renders the HLS playlist as raw text, so
        // the button never opens that. Without a post URL no button is built.
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

        // unbind does not fire on a NavigationView pop, a GtkStack page switch,
        // or a thread page thrown away. unmap does.
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
            // No window (or a test): open the media viewer instead.
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

        // A missing GStreamer element fails before the constructor returns, and
        // on_fail has already restored the thumbnail. Installing the player now
        // would blank it permanently.
        if self.failed.get() {
            return;
        }

        self.frame.set_child(Some(player.widget()));
        // Hide the play button; it would sit over the video.
        self.play_btn.set_visible(false);
        self.player.replace(Some(player));
        self.intent.set(Some(intent));
    }

    /// Put the thumbnail back and drop the pipeline. Idempotent.
    fn disarm(&self) {
        let player = self.player.borrow_mut().take();
        if let Some(player) = player {
            // Drop would do this too. Explicit so teardown happens at a known
            // point.
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
        // Release the claim, or nothing else can arm for the rest of the
        // session.
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
        // Backstop: recycled, rebound or destroyed, the row drops its slot and
        // this runs.
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
    /// One source, re-armed rather than added to.
    election: RefCell<Option<glib::SourceId>>,
    /// The main timeline's scroller. Autoplay is scoped to it; threads,
    /// profiles and search play inline only on a press.
    timeline: glib::WeakRef<gtk4::ScrolledWindow>,
    autoplay: Cell<VideoAutoplay>,
    /// Cached so arming does not read the settings file on every scroll.
    volume: Cell<f64>,
    /// Once an autoplayed video is unmuted, later ones start unmuted.
    /// Session-only state; never written to settings.
    unmuted_by_user: Cell<bool>,
    /// How many dialogs hold the one pipeline.
    ///
    /// The viewer opens from a quoted video, a GIF and the inline fullscreen
    /// button, so two dialogs can overlap. A boolean would release the second
    /// dialog's hold when the first closed.
    dialog_holds: Cell<usize>,
    /// Where one video got to, so scrolling back to it does not start it over.
    /// One entry; a map would grow for the length of the session.
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

    /// Apply a new autoplay mode without refetching. The feed contents do not
    /// change, and a refetch would lose the scroll position.
    pub fn set_autoplay(self: &Rc<Self>, mode: VideoAutoplay) {
        self.autoplay.set(mode);
        if mode.enabled() {
            self.schedule_election();
            return;
        }
        // Stop an Auto video immediately; leave a Manual one alone.
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

    /// A dialog is taking the pipeline. Balanced by exactly one
    /// [`Self::dialog_closed`].
    ///
    /// Never call this on a page switch. That mistake refused the play button
    /// on five pages.
    pub fn dialog_opened(&self) {
        self.dialog_holds.set(self.dialog_holds.get() + 1);
        self.release_current();
    }

    /// A dialog gave the pipeline back. Re-runs the election rather than
    /// resuming.
    ///
    /// `saturating_sub` so an unbalanced call cannot wrap into permanent
    /// suspension.
    pub fn dialog_closed(self: &Rc<Self>) {
        let held = self.dialog_holds.get();
        self.dialog_holds.set(held.saturating_sub(1));
        if self.dialog_holds.get() == 0 {
            self.schedule_election();
        }
    }

    /// The main stack switched pages.
    ///
    /// A page switch changes what is on screen without scrolling or touching a
    /// model, so nothing else re-runs the election. Rows on a hidden page are
    /// unmapped and measure as invisible.
    pub fn page_changed(self: &Rc<Self>) {
        self.schedule_election();
    }

    pub fn register(self: &Rc<Self>, slot: &Rc<VideoSlot>) {
        // Prune here, not only in elections: with autoplay off elections
        // never run, and a night of feed churn built a registry of dead
        // weaks, each pinning its slot's allocation.
        self.prune();
        self.slots.borrow_mut().push((slot.id, Rc::downgrade(slot)));
        // So the first screen after login autoplays without a scroll.
        self.schedule_election();
    }

    /// Drop a dead slot. Called from [`VideoSlot::drop`], where there is no
    /// `Rc` left to hand over.
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
            // Mid-drop: the player is already stopped, only the claim is left.
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
            // A dialog owns the pipeline, so even a manual press is refused;
            // the dialog's player is not ours to tear down.
            //
            // Do not refuse here for "wrong page"; that made the play button
            // inert on Profile, Likes, Search, Mentions and Activity.
            return;
        }
        // Tear down the old pipeline first. Two decoders at once does not
        // just slow down; it posts a bus ERROR.
        self.release_current();

        // Checked in release builds too. A survivor is only attributable at
        // this point, before another hundred rows of scroll.
        let survivors = crate::media::live_pipelines();
        if survivors > 0 {
            eprintln!(
                "hangar: {survivors} video pipeline(s) survived teardown; \
                 arming another risks a decoder the system will refuse"
            );
        }

        // Manual is never muted. Autoplay is, until the user unmutes one.
        let muted = intent == Intent::Auto
            && self.autoplay.get().starts_muted()
            && !self.unmuted_by_user.get();
        let start_at = self.resume_for(&slot.source.playlist_url);

        // Claimed before arming: a pipeline that fails during construction
        // calls note_failed, and a later claim would overwrite it.
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

    /// Tear the current video down, saving position, volume and mute state.
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

    /// Where `slot` sits in the timeline: how much shows, and how far down.
    ///
    /// `None` when it is not in the timeline at all (a thread or profile page);
    /// unbind/unmap tear those down and the election never touches them.
    /// `Some(0.0, _)` is in the feed but off screen, which the release check
    /// does act on.
    fn geometry_of(&self, slot: &VideoSlot) -> Option<(f64, f64)> {
        let scrolled = self.timeline.upgrade()?;
        // compute_bounds succeeds for any two widgets under the same window, so
        // the ancestry check is what scopes autoplay to the feed.
        if !slot.root.is_ancestor(&scrolled) {
            return None;
        }

        let viewport_height = f64::from(scrolled.height());
        // In the feed but unmapped. compute_bounds answers from the last
        // allocation, so a row on a hidden GtkStack page kept its visible-time
        // rectangle, reported 1.00, and never fell below RELEASE_FRACTION.
        let unseen = Some((0.0, viewport_height));
        if !scrolled.is_mapped() || !slot.root.is_mapped() {
            return unseen;
        }
        let Some(bounds) = slot.root.compute_bounds(&scrolled) else {
            return unseen;
        };
        // visible_fraction only answers for the vertical; this rules out a
        // rectangle beside the viewport.
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
        // Clone the list first. Measuring touches widgets and can re-enter
        // this RefCell.
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
            // strong_count check only; an upgrade here could resurrect a slot.
            weak.strong_count() > 0
        });
    }

    /// The per-scroll-frame check: a playing video that has left the
    /// viewport stops now rather than after the debounce settles.
    /// Continuous scrolling re-arms the debounced election every frame,
    /// so without this a video kept decoding through the whole scroll
    /// and the UI paid for every frame it painted off screen.
    pub fn release_scrolled_away(&self) {
        let Some(slot) = self.current_slot() else {
            return;
        };
        if let Some(fraction) = self.visible_fraction_of(&slot)
            && fraction < RELEASE_FRACTION
        {
            self.release_current();
        }
    }

    /// Re-arm the one election source. Every trigger comes through here.
    pub fn schedule_election(self: &Rc<Self>) {
        // Autoplay off and nothing playing: arm nothing.
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
                // Forget the id first: this source is mid-dispatch and returns
                // Remove.
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
                // Not in the timeline: a thread or profile page. unbind and
                // unmap tear those down; the election leaves them alone.
                None => return,
                Some(fraction) => {
                    if fraction < RELEASE_FRACTION {
                        self.release_current();
                    } else if slot.intent() == Some(Intent::Manual) {
                        // Manual and still visible: leave it.
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
            // Nothing eligible to start. The release check above handles
            // stopping; without this a half-visible video would be torn down
            // and rebuilt.
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
        // The closure holds a Weak, but remove the source anyway.
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
            // playbin3 accepts this and never plays it. The pipeline is still
            // built and counted; no main loop runs, so the error never arrives.
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

    /// Scrolling past many videos must not accumulate pipelines.
    ///
    /// The counter is process-wide, but every test that touches it runs through
    /// `with_gtk`, which serialises them onto one worker thread.
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

    /// The scroll-frame release acts only on a video it can measure as
    /// gone. Nothing playing is a no-op, and a slot the timeline cannot
    /// measure (thread and profile pages) must be left alone; the
    /// threshold math itself is visible_fraction's, tested above.
    #[test]
    fn scroll_release_spares_what_it_cannot_measure() {
        with_gtk(scroll_release_spares_what_it_cannot_measure_body);
    }

    fn scroll_release_spares_what_it_cannot_measure_body() {
        let director = VideoDirector::new(&crate::state::AppSettings::default());
        set_director(&director);

        // Idle: nothing to do, nothing to break.
        director.release_scrolled_away();
        assert_eq!(director.current_id(), None);

        // A manual video on widgets the timeline cannot measure, which is
        // what a thread or profile page's player looks like from here.
        let slot = new_slot(0);
        director.arm(&slot, Intent::Manual);
        assert_eq!(media::live_pipelines(), 1);
        director.release_scrolled_away();
        assert_eq!(
            director.current_id(),
            Some(slot.id()),
            "an unmeasurable video must not be torn down by a timeline scroll"
        );

        slot.release();
        assert_eq!(media::live_pipelines(), 0);
    }

    /// Dropping the slot tears the pipeline down.
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

        // Nothing calls release_video, unbind, or unmap; the row just goes away.
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
    /// Space already activates the focused button or quote card, Left/Right
    /// belong to the list and to a focused scrubber, and with several videos
    /// realized at once M has no unambiguous target.
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
    // Feed harness: real rows, a real scroller, a real page switch, a real
    // press of the play button. Skipped where no window can be mapped.
    // ------------------------------------------------------------------

    /// Posts in the harness feed. Enough that half a million pixels of scroll
    /// lands in the middle of it rather than at the end.
    const HARNESS_POSTS: u32 = 3000;

    /// How a feed's list is put into its scroller, for the director tests.
    ///
    /// Both variants virtualize. Built here, so neither says anything about
    /// what `window.rs` uses; the call sites are pinned by
    /// `every_feed_the_app_builds_hands_its_list_straight_to_the_scroller`
    /// over there.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Layout {
        /// Shaped like the timeline and the other clamped feeds: the list
        /// inside an `AdwClampScrollable`, which caps width at 800 and passes
        /// the scroller's adjustments through to the list.
        Clamped,
        /// Shaped like the own profile page: the list handed straight to the
        /// scroller, unclamped.
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
                viewer_muted: false,
                viewer_blocking: None,
                viewer_blocked_by: false,
            },
            text: format!("post {name}"),
            created_at: "2026-01-01T00:00:00Z".into(),
            indexed_at: "2026-01-01T00:00:00Z".into(),
            like_count: None,
            repost_count: None,
            reply_count: None,
            embed: Some(crate::atproto::Embed::Video(crate::atproto::VideoEmbed {
                // No thumbnail: three thousand rows must not fetch three
                // thousand images.
                playlist: format!("file:///nonexistent/{name}.mp4"),
                thumbnail: None,
                alt: Some("a cat".into()),
                aspect_ratio: Some((16, 9)),
            })),
            viewer_like: None,
            viewer_repost: None,
            viewer_bookmarked: None,
            repost_reason: None,
            reply_context: None,
        }
    }

    /// Count how many times a list's factory binds and unbinds a row.
    ///
    /// Whether `unbind` fires at all is what separates a recycling list from
    /// one that is not, and it has to be counted as it happens. The counters go
    /// on the factory the list already has, so this works on the app's timeline
    /// as well as a harness one.
    fn count_binds(list: &gtk4::ListView) -> (Rc<Cell<usize>>, Rc<Cell<usize>>) {
        let binds = Rc::new(Cell::new(0usize));
        let unbinds = Rc::new(Cell::new(0usize));
        if let Some(factory) = list.factory().and_downcast::<gtk4::SignalListItemFactory>() {
            let b = binds.clone();
            factory.connect_bind(move |_, _| b.set(b.get() + 1));
            let u = unbinds.clone();
            factory.connect_unbind(move |_, _| u.set(u.get() + 1));
        }
        (binds, unbinds)
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
        // The test exercises the shipped release_video_on_unbind wiring itself.
        crate::ui::HangarWindow::release_video_on_unbind(&factory);

        let list = gtk4::ListView::new(Some(gtk4::NoSelection::new(Some(model))), Some(factory));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        // The timeline's own policy: no horizontal valve.
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        match layout {
            Layout::Clamped => {
                // Same widget and numbers as the feeds, but built here, so it
                // proves nothing about `window.rs`. See [`Layout`].
                let clamp = libadwaita::ClampScrollable::new();
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
    /// The `GtkEntry` above the content is a focus sink, and it is load
    /// bearing. A `GtkListView` holding the window focus keeps the focused row
    /// realized and pinned to the top of the viewport at every scroll offset.
    /// With the scroller as the whole content there is nothing else focusable,
    /// so the election only ever sees post 0. Measured: without the sink every
    /// offset from 20,000 to 480,000 reports `topmost post Some(0)`; with it
    /// the same sweep reports 205, 820, 1025, 1230. The real window has a
    /// sidebar and headerbar buttons to take that focus.
    fn harness_window(content: &impl IsA<gtk4::Widget>) -> libadwaita::Window {
        let window = libadwaita::Window::new();
        window.set_default_size(700, 900);
        let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let focus_sink = gtk4::Entry::new();
        outer.append(&focus_sink);
        outer.append(content);
        window.set_content(Some(&outer));
        window.present();
        focus_sink.grab_focus();
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
    /// GtkScrolledWindow clamps into `[0, upper - page_size]`, and `upper`
    /// grows as the list realizes more rows. One `set_value` against a three
    /// thousand row feed lands short, sometimes at zero. Setting it again after
    /// each settle lets the value follow `upper` up.
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

    /// The post a row is showing.
    ///
    /// Harness rows carry the number in their widget name. The app's own rows
    /// have no name, so the number comes off the text the row paints. Naming
    /// production rows for the benefit of a test would put the test back to
    /// measuring the harness.
    fn row_post_number(row: &crate::ui::post_row::PostRow) -> Option<u32> {
        if let Some(number) = post_number(&row.widget_name()) {
            return Some(number);
        }
        let mut all = Vec::new();
        descendants(row.upcast_ref(), &mut all);
        all.into_iter()
            .filter_map(|w| w.downcast::<gtk4::Label>().ok())
            .find_map(|label| label.text().strip_prefix("post home:")?.parse().ok())
    }

    fn slot_post_number(slot: &VideoSlot) -> Option<u32> {
        post_number(&slot.source.playlist_url)
    }

    /// Every realized row, as (post, top edge in the scroller, on screen),
    /// ordered by reported position.
    ///
    /// "On screen" is `is_mapped()` AND a non-empty allocation. Pooled
    /// GtkListView rows are mapped and report y=0, but are allocated 0x0.
    fn row_geometry(
        list: &gtk4::ListView,
        scrolled: &gtk4::ScrolledWindow,
    ) -> Vec<(u32, f64, bool)> {
        let mut seen: Vec<(u32, f64, bool)> = rows_of(list)
            .iter()
            .filter_map(|row| {
                let number = row_post_number(row)?;
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

    /// Posts GTK is actually painting, by hit test. Sampled down the middle of
    /// the viewport.
    fn posts_rendered_in(scrolled: &gtk4::ScrolledWindow) -> Vec<u32> {
        let width = f64::from(scrolled.width());
        let height = f64::from(scrolled.height());
        let mut seen = Vec::new();
        let mut y = 10.0;
        while y < height {
            if let Some(hit) = scrolled.pick(width / 2.0, y, gtk4::PickFlags::DEFAULT) {
                let mut cursor = Some(hit);
                while let Some(widget) = cursor {
                    if let Ok(row) = widget.clone().downcast::<crate::ui::post_row::PostRow>() {
                        if let Some(number) = row_post_number(&row)
                            && !seen.contains(&number)
                        {
                            seen.push(number);
                        }
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

    /// How many AdwDialogs are open over `window`.
    fn open_dialogs(window: &libadwaita::Window) -> usize {
        let mut all = Vec::new();
        descendants(window.upcast_ref(), &mut all);
        all.iter().filter(|w| w.is::<libadwaita::Dialog>()).count()
    }

    /// Skip if the session will not map a window.
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

    /// Autoplay must never arm a post that is not on screen.
    ///
    /// Checked by hit test ([`posts_rendered_in`]). Comparing rectangles was
    /// intermittently red because of pool rows, see [`row_geometry`]. Both
    /// layouts swept: `Recycling` has the pool, `Clamped` is what ships.
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
                // Whatever is playing must be a post GTK is painting.
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

    /// A hidden GtkStack page: every row keeps the rectangle it had when
    /// visible.
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

    /// Calls the shipped page-switch handler.
    fn leaving_home(director: &Rc<VideoDirector>) {
        director.page_changed();
    }

    /// Pressing play must work on every page; it once failed everywhere but Home.
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
            // Measured before settling: arm builds the pipeline, and this URL
            // fails on the bus a few milliseconds later.
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
        // And it must play in place; a dialog would show up as d == 1.
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

    /// Two overlapping dialogs must not un-hold each other, and closing one
    /// must not wake a hidden feed.
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

            // run_election is what the 200ms source dispatches. Called directly
            // so the sample lands on the decision, before the bus error.
            let after_election = |director: &Rc<VideoDirector>| {
                director.run_election();
                crate::media::live_pipelines()
            };

            // The feed autoplays while it is on screen, or the rest proves
            // nothing.
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

    /// Posts for the deep-scroll proof. Big enough that the offsets below land
    /// thousands of rows in.
    const DEEP_POSTS: u32 = 5000;

    /// One embed a row is painting: the widget's type, and its allocation.
    type PaintedMedia = (String, i32, i32);

    /// A row on screen: the post it is showing, and the media it is painting.
    type PaintedRow = (u32, Vec<PaintedMedia>);

    /// The media a row is painting.
    ///
    /// Filtered on the `video-embed` CSS class rather than widget type. A
    /// `PostRow` is full of `GtkImage` icons (like, repost, overflow) and
    /// counting those would pass on a row with no media at all.
    fn media_on(row: &crate::ui::post_row::PostRow) -> Vec<PaintedMedia> {
        let mut all = Vec::new();
        descendants(row.upcast_ref(), &mut all);
        all.into_iter()
            .filter(|w| {
                w.is_mapped()
                    && w.width() > 0
                    && w.height() > 0
                    && w.css_classes().iter().any(|c| c == "video-embed")
            })
            .map(|w| (w.type_().name().to_string(), w.width(), w.height()))
            .collect()
    }

    /// The rows the real timeline is painting right now, with their media.
    fn painted_rows(list: &gtk4::ListView, scrolled: &gtk4::ScrolledWindow) -> Vec<PaintedRow> {
        let painted = posts_rendered_in(scrolled);
        rows_of(list)
            .into_iter()
            .filter_map(|row| {
                let number = row_post_number(&row)?;
                painted.contains(&number).then(|| (number, media_on(&row)))
            })
            .collect()
    }

    /// The migration, measured on the timeline the app builds, at five offsets
    /// into a five thousand post feed.
    ///
    /// Five claims: the list is allocated the viewport rather than its whole
    /// natural height; the realized row set stays bounded; `unbind` fires, so
    /// `release_video_on_unbind` runs; GTK paints at every offset and what it
    /// paints follows the scroll; every row on screen still has its media.
    ///
    /// The last one needs the `bind` fix. At these offsets every visible row
    /// has been rebound hundreds of times, so a feed that virtualized without
    /// an idempotent `bind` would pass the first four and fail here.
    ///
    /// Drives a real `HangarWindow`. A clamp built inside this file measures
    /// libadwaita: an earlier version of this test stayed green after all seven
    /// `window.rs` call sites were flipped back to `AdwClamp`.
    #[test]
    fn the_real_timeline_virtualizes_paints_and_keeps_its_media() {
        with_gtk(the_real_timeline_virtualizes_paints_and_keeps_its_media_body);
    }

    fn the_real_timeline_virtualizes_paints_and_keeps_its_media_body() {
        use gtk4::subclass::prelude::ObjectSubclassIsExt;

        let window: crate::ui::HangarWindow = glib::Object::builder().build();
        window.set_default_size(700, 900);

        // The window's own director. `setup_ui` builds one and installs it as
        // the process director before any page exists, so a director built here
        // would be replaced before a single row bound and every slot would
        // register with the window's. The first draft did that and reported
        // zero live pipelines forever.
        let director = window
            .imp()
            .video_director
            .borrow()
            .clone()
            .expect("the window built a video director");

        // Autoplay is off by default, and with nothing armed the pipeline
        // bounds below hold on a build that leaks every one of them. Muted arms
        // videos as they come on screen. Set here; `AppSettings::load()` reads
        // the real settings file and would make coverage depend on the machine.
        director.set_autoplay(VideoAutoplay::Muted);

        let scrolled = window
            .imp()
            .scrolled_window
            .borrow()
            .clone()
            .expect("the timeline built a scroller");
        let list = window
            .imp()
            .timeline_list_view
            .borrow()
            .clone()
            .expect("the timeline built a list");

        // Counters on before the model is filled. `GtkListView` reacts to a
        // model change straight away, so filling first hides the couple of
        // hundred binds that happen before the window is mapped.
        let (binds, unbinds) = count_binds(&list);
        window.set_posts(
            (0..DEEP_POSTS)
                .map(|i| harness_post(&format!("home:{i}")))
                .collect(),
        );

        window.present();
        settle(600);

        if !window.is_mapped() || scrolled.height() <= 400 {
            eprintln!(
                "skipping: the window never mapped (mapped {}, viewport {}px), so there \
                 is no geometry to measure",
                window.is_mapped(),
                scrolled.height()
            );
            window.destroy();
            return;
        }

        // 1. The list is allocated the viewport. With an AdwClamp in the way
        //    the scroller interposed a GtkViewport, which allocates its child
        //    the child's full natural height: 5000 rows of about 418px, so
        //    roughly two million pixels of list inside a 900px window.
        let natural = scrolled.vadjustment().upper();
        assert!(
            f64::from(list.height()) < natural / 10.0,
            "the list is {}px tall against a feed {natural}px long — it has been \
             allocated its whole natural height rather than the viewport, which is \
             what a GtkViewport between the scroller and the list does, and a list \
             that believes it is fully visible does not recycle anything",
            list.height(),
        );

        let realized_at_rest = rows_of(&list).len();
        let binds_at_rest = binds.get();
        let mut painted_by_offset = Vec::new();
        let mut peak_pipelines = 0;
        let mut peak_slots = 0;

        for offset in [0.0, 200_000.0, 800_000.0, 1_500_000.0, 2_000_000.0] {
            scroll_to(&scrolled, offset);
            settle(200);
            // The election turns "on screen" into a live pipeline. Run here so
            // the count below lands at a defined point instead of wherever the
            // 200ms timer fell.
            director.run_election();
            // Sampled before the pipeline can die. These files do not exist,
            // so each armed video fails on its bus a moment later. The question
            // is how many were alive at once.
            let armed_pipelines = crate::media::live_pipelines();
            let armed_slots = director.slots.borrow().len();
            settle(120);

            let geometry = row_geometry(&list, &scrolled);
            let realized = geometry.len();
            let placed = geometry.iter().filter(|(_, _, on)| *on).count();
            let on_screen = painted_rows(&list, &scrolled);
            let painted: Vec<u32> = on_screen.iter().map(|(n, _)| *n).collect();
            peak_pipelines = peak_pipelines.max(armed_pipelines);
            peak_slots = peak_slots.max(armed_slots);

            eprintln!(
                "offset {offset:>9.0}: {realized:>3} realized, {placed} placed, painted \
                 {painted:?}, media {:?}, binds {}, unbinds {}, slots {armed_slots}, \
                 pipelines {armed_pipelines}",
                on_screen.iter().map(|(_, m)| m.len()).collect::<Vec<_>>(),
                binds.get(),
                unbinds.get(),
            );

            // 4a. Something is on screen. The old layout went blank here.
            assert!(
                !painted.is_empty(),
                "nothing painted at offset {offset} — the feed is blank"
            );

            // 2. Two bounds. `placed` counts rows GTK gives a rectangle inside
            //    the viewport; under the old layout that was the whole realized
            //    set. `realized` is looser on purpose: GtkListView keeps a pool
            //    it never shrinks, and what matters is that the pool does not
            //    grow with the feed.
            assert!(
                placed <= 8,
                "{placed} rows placed in one viewport at offset {offset} — the list is \
                 being allocated more than the viewport"
            );
            assert!(
                realized < 400,
                "{realized} rows realized at offset {offset} against a model of \
                 {DEEP_POSTS} — the realized set is tracking the feed, not the viewport"
            );

            // 5. Recycled rows still show their media.
            for (number, media) in &on_screen {
                assert!(
                    !media.is_empty(),
                    "post {number} is on screen at offset {offset} with no media \
                     allocated, though every post in this feed has a video embed. \
                     At this depth every row is a recycled one, so this is what a \
                     `bind` that does not rebuild its embed looks like once the feed \
                     virtualizes: not a blank feed, a feed of blank posts."
                );
                for (kind, width, height) in media {
                    assert!(
                        *width > 0 && *height > 0,
                        "post {number}'s {kind} is allocated {width}x{height} at offset \
                         {offset}"
                    );
                }
            }

            painted_by_offset.push((offset, painted));
        }

        // 4b. What is painted follows the scroll: every deep offset paints
        //     higher-numbered posts than the top did.
        let (_, at_top) = &painted_by_offset[0];
        let top_min = *at_top.iter().min().expect("something painted at the top");
        for (offset, painted) in painted_by_offset.iter().skip(1) {
            let lowest = *painted.iter().min().expect("something painted");
            assert!(
                lowest > top_min,
                "offset {offset} paints {painted:?}, no further into the feed than the \
                 top did ({at_top:?}) — the list is not scrolling its rows"
            );
        }

        // 3. Rows are recycled, which is what runs `release_video_on_unbind`,
        //    and pipelines stayed bounded across the sweep.
        assert!(
            unbinds.get() > 0,
            "`unbind` never fired across a two million pixel sweep, so \
             `release_video_on_unbind` never ran and every row that ever held a video \
             still holds it"
        );
        assert!(
            binds.get() > binds_at_rest,
            "no row was rebound while scrolling"
        );
        // A video was armed at every checkpoint. Without this the two bounds
        // below are vacuous, and they were: an earlier draft measured a
        // director no slot had registered with, read zero pipelines and passed.
        assert!(
            peak_pipelines >= 1,
            "no pipeline was ever live at a checkpoint, so the two bounds below \
             are measuring nothing. Either autoplay never armed a video or the \
             director being measured is not the one the rows registered with."
        );
        assert!(
            peak_pipelines <= 1,
            "{peak_pipelines} live pipelines at once — there is supposed to be at \
             most one video playing, and a virtualized feed that recycles a row \
             out from under a playing pipeline without releasing it is exactly how \
             that budget gets exceeded"
        );
        // Slots live as long as the row that owns them, so this is the count
        // that grows without bound if recycling leaks: one per post shown
        // instead of one per row in the pool.
        assert!(
            peak_slots < 400,
            "{peak_slots} video slots registered at once against a pool of about \
             {realized_at_rest} rows and {} posts bound over the sweep — slots are \
             accumulating per post shown rather than per row, so rows are not \
             releasing their video when they are recycled",
            binds.get(),
        );

        eprintln!(
            "real timeline: {realized_at_rest} rows realized at rest of {DEEP_POSTS} posts, \
             list allocated {}px in a {}px viewport against a {}px feed, {} binds and {} \
             unbinds over the sweep, at most {peak_slots} slots and \
             {peak_pipelines} live pipelines",
            list.height(),
            scrolled.height(),
            scrolled.vadjustment().upper(),
            binds.get(),
            unbinds.get(),
        );

        window.destroy();
        drop(director);
        settle(100);
        assert_eq!(crate::media::live_pipelines(), 0);
    }
    /// The "N new posts" banner must not move the feed under the reader.
    ///
    /// `insert_posts_at_top` restores the scroll position by the change in
    /// `upper`. That subtraction used to be exact, because `upper` was the sum
    /// of every row's height. A virtualized list extrapolates `upper` from the
    /// rows it has measured, so both terms are now estimates. Asserts on what a
    /// reader sees, since the arithmetic is allowed to be approximate.
    #[test]
    fn inserting_posts_at_the_top_leaves_the_reader_where_they_were() {
        with_gtk(inserting_posts_at_the_top_leaves_the_reader_where_they_were_body);
    }

    fn inserting_posts_at_the_top_leaves_the_reader_where_they_were_body() {
        use gtk4::subclass::prelude::ObjectSubclassIsExt;

        let window: crate::ui::HangarWindow = glib::Object::builder().build();
        window.set_default_size(700, 900);

        let scrolled = window
            .imp()
            .scrolled_window
            .borrow()
            .clone()
            .expect("the timeline built a scroller");

        window.set_posts(
            (0..1000)
                .map(|i| harness_post(&format!("home:{i}")))
                .collect(),
        );
        window.present();
        settle(600);

        if !window.is_mapped() || scrolled.height() <= 400 {
            eprintln!("skipping: the window never mapped, so there is no geometry to measure");
            window.destroy();
            return;
        }

        scroll_to(&scrolled, 200_000.0);
        settle(250);

        let before = posts_rendered_in(&scrolled);
        let anchor = *before
            .first()
            .expect("something on screen before the insert");
        let value_before = scrolled.vadjustment().value();
        let upper_before = scrolled.vadjustment().upper();

        // Numbered well clear of the existing feed so the two are told apart.
        const NEW_POSTS: u32 = 20;
        window.insert_posts_at_top(
            (0..NEW_POSTS)
                .map(|i| harness_post(&format!("home:{}", 90_000 + i)))
                .collect(),
        );
        // The restore runs on an idle source, after GTK has relaid out.
        settle(500);

        let after = posts_rendered_in(&scrolled);
        let value_after = scrolled.vadjustment().value();
        let upper_after = scrolled.vadjustment().upper();

        eprintln!(
            "insert at top: on screen {before:?} at {value_before:.0}/{upper_before:.0}, \
             after inserting {NEW_POSTS} posts {after:?} at {value_after:.0}/{upper_after:.0} \
             (moved {:.0}px, {NEW_POSTS} posts is about {:.0}px)",
            value_after - value_before,
            f64::from(NEW_POSTS) * 418.0,
        );

        assert!(
            !after.is_empty(),
            "nothing on screen after inserting at the top"
        );
        assert!(
            after.contains(&anchor),
            "post {anchor} was on screen before the insert and is not after it: the feed \
             now shows {after:?}. The scroll restore in `insert_posts_at_top` is derived \
             from the adjustment's `upper`, which a virtualized list only estimates, so \
             this is where that estimate being wrong would show up — as the feed jumping \
             under someone who was reading it."
        );
        assert!(
            after.iter().all(|post| *post < 90_000),
            "the feed jumped to the newly inserted posts {after:?} — inserting at the top \
             is supposed to leave the reader where they were, not scroll them to the top"
        );

        // Posts that are not the height of the ones already measured. Every
        // row so far is a video embed of the same aspect ratio, so the
        // extrapolation is exact. Text-only posts are a fraction of that
        // height, which is where the estimate does real work.
        let anchor = *after.first().expect("something on screen");
        let value_mixed_before = scrolled.vadjustment().value();
        window.insert_posts_at_top(
            (0..NEW_POSTS)
                .map(|i| {
                    let mut post = harness_post(&format!("home:{}", 80_000 + i));
                    post.embed = None;
                    post
                })
                .collect(),
        );
        settle(500);

        let mixed = posts_rendered_in(&scrolled);
        eprintln!(
            "insert at top, short rows: on screen {mixed:?} at {:.0} (moved {:.0}px for \
             {NEW_POSTS} text posts)",
            scrolled.vadjustment().value(),
            scrolled.vadjustment().value() - value_mixed_before,
        );

        assert!(
            mixed.contains(&anchor),
            "post {anchor} was on screen before {NEW_POSTS} short posts were inserted at \
             the top and is not after it: the feed now shows {mixed:?}. The restore adds \
             the change in `upper`, and for a virtualized list `upper` is extrapolated \
             from the rows measured so far — all of them tall video rows here — so \
             inserting shorter ones is where that extrapolation is furthest out."
        );

        window.destroy();
    }

    /// Key controllers on widgets we built.
    ///
    /// GTK's own controllers are skipped. GtkButton's makes Space activate,
    /// GtkRange's makes the arrows scrub, and GtkPopover's makes Escape close.
    /// The boxes, overlays and frames in between must have none.
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
