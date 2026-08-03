// SPDX-License-Identifier: MPL-2.0

//! An independent audit of whether a recycled `PostRow` still paints its media.
//!
//! An adversarial check on the bind-idempotency fix and the virtualization
//! migration. Shares no helper with the tests in `post_row.rs`, `window.rs` or
//! `inline_video.rs`; only [`crate::ui::with_gtk`], since GTK takes exactly one
//! init per process.
//!
//! The question: for each kind of embed, on a row that has already shown other
//! posts, is there a picture, frame, overlay or play button under the embed
//! band, or is the band an empty container?
//!
//! Answered by hit test, `gtk_widget_pick` at nine points across the band. A
//! container that has the right size but never allocates its child cannot fake
//! that.

use crate::atproto::{Embed, ExternalEmbed, ImageEmbed, Post, Profile, QuoteEmbed, VideoEmbed};
use crate::ui::post_row::PostRow;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

// ---------------------------------------------------------------------------
// The feed
// ---------------------------------------------------------------------------

/// Refuses a connection on the first syscall, so nothing here waits on a
/// network that may not exist. Every image in this feed is this URL.
const REFUSED: &str = "http://127.0.0.1:1/audit.jpg";

/// A real Klipy URL shape. `atproto::gif::detect` is a pure function of the
/// URL; anything else renders as an ordinary link card.
const KLIPY: &str = "https://static.klipy.com/ii/4493325008d34b7bf8cd6813cd5c1619/76/03/cgMM2dB2oEoY.gif?hh=278&ww=498&mp4=UxPaHvFAwuHpj3&webm=MCEXFCTK5uN0J47r";

/// The embed kinds under audit, in the order posts cycle through them.
const KINDS: [&str; 10] = [
    "1-image",
    "2-images",
    "3-images",
    "4-images",
    "video",
    "gif",
    "link-card",
    "quote",
    "quote+media",
    "text-only",
];

/// Posts in the audited feed.
const FEED: u32 = 5000;

fn who() -> Profile {
    Profile::minimal(
        "did:plc:audit".into(),
        "auditor.bsky.social".into(),
        Some("Auditor".into()),
        None,
    )
}

fn img(w: u32, h: u32) -> ImageEmbed {
    ImageEmbed {
        thumb: REFUSED.into(),
        fullsize: REFUSED.into(),
        alt: "audit image".into(),
        aspect_ratio: Some((w, h)),
    }
}

fn quoted(nested: Option<Embed>) -> QuoteEmbed {
    QuoteEmbed {
        uri: "at://did:plc:audit/app.bsky.feed.post/quoted".into(),
        cid: "cid".into(),
        author: who(),
        text: "the quoted post body".into(),
        indexed_at: "2026-01-01T00:00:00Z".into(),
        embed: nested.map(Box::new),
    }
}

/// The embed for kind `k`.
fn embed_for(k: usize) -> Option<Embed> {
    match k {
        0 => Some(Embed::Images(vec![img(16, 9)])),
        1 => Some(Embed::Images(vec![img(1, 1), img(1, 1)])),
        2 => Some(Embed::Images(vec![img(4, 3), img(4, 3), img(4, 3)])),
        3 => Some(Embed::Images(vec![img(2, 3); 4])),
        4 => Some(Embed::Video(VideoEmbed {
            playlist: "http://127.0.0.1:1/audit.m3u8".into(),
            thumbnail: Some(REFUSED.into()),
            alt: Some("an audited video".into()),
            aspect_ratio: Some((16, 9)),
        })),
        5 => Some(Embed::External(ExternalEmbed {
            uri: KLIPY.into(),
            title: "Shrug".into(),
            description: "ALT: a shrug".into(),
            thumb: Some(REFUSED.into()),
        })),
        6 => Some(Embed::External(ExternalEmbed {
            uri: "https://www.example.com/2026/08/a-story".into(),
            title: "A story about something".into(),
            description: "Something happened and here is a sentence about it".into(),
            thumb: Some(REFUSED.into()),
        })),
        7 => Some(Embed::Quote(quoted(None))),
        8 => Some(Embed::QuoteWithMedia {
            quote: quoted(None),
            media: Box::new(Embed::Images(vec![img(16, 9)])),
        }),
        _ => None,
    }
}

/// Post `i`. The visible text carries the index, so a row on screen can be
/// identified without reaching into private state.
fn audit_post(i: u32) -> Post {
    let k = (i % KINDS.len() as u32) as usize;
    Post {
        uri: format!("at://did:plc:audit/app.bsky.feed.post/{i}"),
        cid: format!("cid{i}"),
        author: who(),
        text: format!("p{i}k{k} audited post body"),
        created_at: "2026-01-01T00:00:00Z".into(),
        indexed_at: "2026-01-01T00:00:00Z".into(),
        like_count: Some(i % 97),
        repost_count: Some(i % 31),
        reply_count: Some(i % 13),
        embed: embed_for(k),
        viewer_like: None,
        viewer_repost: None,
        // Reposts and replies, so the header furniture changes shape between
        // binds as well as the embed band.
        //
        // 3 and 7 share no factor with the ten-kind cycle. With `i % 5` every
        // one-image and every GIF post was a reply, `set_posts` filters replies
        // out, and both kinds vanished from the shipped-timeline census.
        repost_reason: (i.is_multiple_of(3)).then(|| crate::atproto::RepostReason {
            by: Profile::minimal(
                format!("did:plc:reposter{i}"),
                format!("reposter{i}.bsky.social"),
                Some(format!("Reposter {i}")),
                None,
            ),
            indexed_at: "2026-01-01T00:00:00Z".into(),
        }),
        reply_context: (i % 7 == 3).then(|| crate::atproto::ReplyContext {
            parent_author: Profile::minimal(
                format!("did:plc:parent{i}"),
                format!("parent{i}.bsky.social"),
                Some(format!("Parent {i}")),
                None,
            ),
            root_author: who(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

mod store {
    use super::*;
    use gtk4::subclass::prelude::*;

    #[derive(Default)]
    pub struct AuditItem {
        pub post: RefCell<Option<Post>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AuditItem {
        const NAME: &'static str = "HangarRebindAuditItem";
        type Type = super::AuditItem;
    }

    impl ObjectImpl for AuditItem {}
}

glib::wrapper! {
    pub struct AuditItem(ObjectSubclass<store::AuditItem>);
}

impl AuditItem {
    fn new(post: Post) -> Self {
        use gtk4::subclass::prelude::ObjectSubclassIsExt;
        let item: Self = glib::Object::builder().build();
        item.imp().post.replace(Some(post));
        item
    }

    fn post(&self) -> Post {
        use gtk4::subclass::prelude::ObjectSubclassIsExt;
        self.imp().post.borrow().clone().expect("item holds a post")
    }
}

/// Sweeps run over the feed, and stops in each.
const SWEEPS: u32 = 6;
const STOPS: u32 = 80;

/// Where in the feed sweep `sweep` stops at step `step`, as a fraction.
///
/// The first four sweeps run the feed end to end and back, twice. The last two
/// jump around pseudo-randomly, so nothing here is an artefact of always
/// arriving at a row from the same direction.
fn offset_fraction(sweep: u32, step: u32) -> f64 {
    if sweep < 4 {
        let along = f64::from(step) / f64::from(STOPS);
        return if sweep.is_multiple_of(2) {
            along
        } else {
            1.0 - along
        };
    }
    // An LCG, so the jumps repeat run to run and a failure reproduces.
    let mut state = u64::from(sweep)
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(u64::from(step))
        .wrapping_add(1);
    state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    f64::from((state >> 33) as u32) / f64::from(u32::MAX)
}

/// Run the main loop for `ms`, so GTK has frames in which to lay anything out.
fn pump(ms: u64) {
    let ctx = glib::MainContext::default();
    let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    while std::time::Instant::now() < end {
        while ctx.iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// Every widget at or under `w`, deepest last.
fn subtree(w: &gtk4::Widget, out: &mut Vec<gtk4::Widget>) {
    out.push(w.clone());
    let mut c = w.first_child();
    while let Some(child) = c {
        subtree(&child, out);
        c = child.next_sibling();
    }
}

/// The embed band of a row, found by the CSS class `PostRow` puts on it.
///
/// Not `row.imp().embed_container`. This module has no privileged view of the
/// widget it audits and compiles against a tree that predates the fix.
fn band_of(row: &PostRow) -> Option<gtk4::Widget> {
    let mut all = Vec::new();
    subtree(row.upcast_ref(), &mut all);
    all.into_iter()
        .find(|w| w.css_classes().iter().any(|c| c == "post-embed"))
}

/// The post index and embed kind a row is showing, read off the text it paints.
fn showing(row: &PostRow) -> Option<(u32, usize)> {
    let mut all = Vec::new();
    subtree(row.upcast_ref(), &mut all);
    for w in all {
        let Ok(label) = w.downcast::<gtk4::Label>() else {
            continue;
        };
        let text = label.text();
        let Some(rest) = text.strip_prefix('p') else {
            continue;
        };
        let Some((num, tail)) = rest.split_once('k') else {
            continue;
        };
        let Ok(i) = num.parse::<u32>() else { continue };
        let kind = tail.split_whitespace().next()?.parse::<usize>().ok()?;
        return Some((i, kind));
    }
    None
}

/// What GTK picks at nine points across `band`, as (type name, depth below the
/// band). Depth 0 is the band itself, which is what a container that never
/// allocated its child looks like.
fn hits(band: &gtk4::Widget) -> Vec<(String, usize)> {
    let w = f64::from(band.width());
    let h = f64::from(band.height());
    let mut out = Vec::new();
    for fy in [0.12, 0.5, 0.88] {
        for fx in [0.12, 0.5, 0.88] {
            let hit = band.pick(w * fx, h * fy, gtk4::PickFlags::DEFAULT);
            match hit {
                None => out.push(("<nothing>".to_string(), 0)),
                Some(widget) => {
                    let mut depth = 0usize;
                    let mut cursor = Some(widget.clone());
                    while let Some(current) = cursor {
                        if current == *band {
                            break;
                        }
                        depth += 1;
                        cursor = current.parent();
                    }
                    out.push((widget.type_().name().to_string(), depth));
                }
            }
        }
    }
    out
}

/// One row's verdict for one post.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// Every sampled point landed on something beneath the band, and at least
    /// one landed two or more levels down: a frame, picture, card or play
    /// button rather than a filler box.
    Media,
    /// The band is the right size but nothing is under it at one or more
    /// sampled points. The reported blocker's signature.
    Blank,
    /// Something under the band is visible but allocated 0x0.
    Unallocated,
    /// The band itself was never allocated.
    NoBand,
    /// A text-only post correctly showing no band at all.
    CorrectlyEmpty,
    /// A text-only post still showing the previous post's media.
    StaleMedia,
}

/// Audit one row as it currently stands.
fn judge(row: &PostRow, kind: usize) -> (Verdict, Vec<(String, usize)>) {
    let Some(band) = band_of(row) else {
        return (Verdict::NoBand, Vec::new());
    };

    // Text-only. The band must be hidden and childless.
    if kind == KINDS.len() - 1 {
        let empty = !band.is_visible() && band.first_child().is_none();
        return (
            if empty {
                Verdict::CorrectlyEmpty
            } else {
                Verdict::StaleMedia
            },
            Vec::new(),
        );
    }

    if !band.is_visible() || band.width() == 0 || band.height() == 0 {
        return (Verdict::NoBand, Vec::new());
    }

    let mut all = Vec::new();
    subtree(&band, &mut all);
    let zero = all
        .iter()
        .skip(1)
        .any(|w| w.is_visible() && (w.width() == 0 || w.height() == 0));

    let sampled = hits(&band);
    let dead = sampled.iter().any(|(_, depth)| *depth == 0);
    let deepest = sampled.iter().map(|(_, d)| *d).max().unwrap_or(0);

    let verdict = if dead || deepest < 2 {
        Verdict::Blank
    } else if zero {
        Verdict::Unallocated
    } else {
        Verdict::Media
    };
    (verdict, sampled)
}

/// Tally of verdicts, per embed kind.
#[derive(Default)]
struct Census {
    rows: BTreeMap<usize, BTreeMap<&'static str, u32>>,
    examples: BTreeMap<usize, String>,
}

impl Census {
    fn record(&mut self, kind: usize, verdict: Verdict, sampled: &[(String, usize)], post: u32) {
        let name = match verdict {
            Verdict::Media => "media",
            Verdict::Blank => "BLANK",
            Verdict::Unallocated => "UNALLOCATED",
            Verdict::NoBand => "NO BAND",
            Verdict::CorrectlyEmpty => "media",
            Verdict::StaleMedia => "STALE",
        };
        *self.rows.entry(kind).or_default().entry(name).or_insert(0) += 1;
        if verdict != Verdict::Media
            && verdict != Verdict::CorrectlyEmpty
            && !self.examples.contains_key(&kind)
        {
            self.examples.insert(
                kind,
                format!("post {post} ({}): {name}, picks {:?}", KINDS[kind], sampled),
            );
        }
    }

    fn bad(&self) -> u32 {
        self.rows
            .values()
            .flat_map(|counts| counts.iter())
            .filter(|(name, _)| **name != "media")
            .map(|(_, n)| *n)
            .sum()
    }

    fn total(&self) -> u32 {
        self.rows.values().flat_map(|counts| counts.values()).sum()
    }

    fn report(&self, title: &str) {
        eprintln!("\n=== {title} ===");
        eprintln!(
            "{:<12} {:>8} {:>8} {:>8}",
            "embed kind", "rendered", "blank", "other"
        );
        for (kind, counts) in &self.rows {
            let ok = counts.get("media").copied().unwrap_or(0);
            let blank = counts.get("BLANK").copied().unwrap_or(0);
            let other: u32 = counts
                .iter()
                .filter(|(name, _)| **name != "media" && **name != "BLANK")
                .map(|(_, n)| *n)
                .sum();
            eprintln!("{:<12} {ok:>8} {blank:>8} {other:>8}", KINDS[*kind]);
        }
        for example in self.examples.values() {
            eprintln!("  first failure: {example}");
        }
        eprintln!(
            "total observations {}, of which not-media {}",
            self.total(),
            self.bad()
        );
    }
}

// ---------------------------------------------------------------------------
// 1. The reported minimal repro: two binds on one presented row
// ---------------------------------------------------------------------------

/// Bind a presented row repeatedly and check its band after every bind.
///
/// The shape the blocker report describes: no list, no recycling, just `bind`
/// twice on a row that is on screen. Walked over every embed kind in both
/// directions, four times.
#[test]
fn rebinding_a_presented_row_keeps_its_media_for_every_embed_kind() {
    crate::ui::with_gtk(rebinding_a_presented_row_keeps_its_media_for_every_embed_kind_body);
}

fn rebinding_a_presented_row_keeps_its_media_for_every_embed_kind_body() {
    use libadwaita::prelude::AdwWindowExt;

    let column = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    column.set_valign(gtk4::Align::Start);
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(800);
    clamp.set_tightening_threshold(600);
    clamp.set_child(Some(&column));
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scrolled.set_child(Some(&clamp));
    let window = adw::Window::new();
    window.set_default_size(700, 900);
    window.set_content(Some(&scrolled));
    window.present();
    pump(300);

    if !window.is_mapped() || scrolled.width() < 400 {
        eprintln!("skipping: no mapped window to measure");
        window.destroy();
        return;
    }

    let row = PostRow::new();
    column.append(&row);

    let mut census = Census::default();
    let order: Vec<usize> = (0..KINDS.len()).chain((0..KINDS.len()).rev()).collect();
    let mut binds = 0u32;
    for pass in 0..4 {
        for kind in &order {
            let post = audit_post(u32::try_from(pass * 100 + kind).unwrap() * 10 + *kind as u32);
            // Force the kind rather than take whatever the index implies.
            let mut post = post;
            post.embed = embed_for(*kind);
            post.text = format!("p{binds}k{kind} audited post body");
            row.bind(&post);
            pump(90);
            binds += 1;
            // The first bind of all is not a rebind; everything after is.
            if binds == 1 {
                continue;
            }
            let (verdict, sampled) = judge(&row, *kind);
            census.record(*kind, verdict, &sampled, binds);
        }
    }

    census.report("minimal repro: one presented row, bound over and over");
    let bad = census.bad();
    window.destroy();
    assert_eq!(
        bad,
        0,
        "{bad} of {} rebinds left the band without media",
        census.total()
    );
}

// ---------------------------------------------------------------------------
// 2. A virtualized list of 5000 posts, scrolled deep and repeatedly
// ---------------------------------------------------------------------------

/// Rows that have actually been recycled still paint their media.
///
/// Builds the shape `window.rs` builds (scroller with no horizontal valve,
/// `AdwClampScrollable`, `GtkListView`) over five thousand posts cycling
/// through every embed kind, sweeps it end to end several times, and hit-tests
/// the band of every row in the viewport at every stop.
///
/// Only rows the factory has bound at least twice are counted, so every
/// observation is of a recycled row.
#[test]
fn recycled_rows_in_a_virtualized_list_keep_their_media() {
    crate::ui::with_gtk(recycled_rows_in_a_virtualized_list_keep_their_media_body);
}

fn recycled_rows_in_a_virtualized_list_keep_their_media_body() {
    use libadwaita::prelude::AdwWindowExt;

    let model = gtk4::gio::ListStore::new::<AuditItem>();
    model.extend_from_slice(
        &(0..FEED)
            .map(|i| AuditItem::new(audit_post(i)))
            .collect::<Vec<_>>(),
    );

    // Binds per pooled row, keyed by the row's address. Rows live in the
    // list's pool for the whole test, so the address is stable.
    let bind_counts: Rc<RefCell<HashMap<usize, u32>>> = Rc::default();
    let unbinds = Rc::new(std::cell::Cell::new(0u32));

    let factory = gtk4::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() else {
            return;
        };
        list_item.set_child(Some(&PostRow::new()));
    });
    let counts = bind_counts.clone();
    factory.connect_bind(move |_, item| {
        let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() else {
            return;
        };
        let Some(row) = list_item.child().and_downcast::<PostRow>() else {
            return;
        };
        let Some(audit) = list_item.item().and_downcast::<AuditItem>() else {
            return;
        };
        row.bind(&audit.post());
        *counts
            .borrow_mut()
            .entry(row.as_ptr() as usize)
            .or_insert(0) += 1;
    });
    let unbind_counter = unbinds.clone();
    factory.connect_unbind(move |_, _| unbind_counter.set(unbind_counter.get() + 1));

    let selection = gtk4::NoSelection::new(Some(model));
    let list = gtk4::ListView::new(Some(selection), Some(factory));
    list.add_css_class("background");

    let clamp = adw::ClampScrollable::new();
    clamp.set_maximum_size(800);
    clamp.set_tightening_threshold(600);
    clamp.set_child(Some(&list));

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scrolled.set_child(Some(&clamp));

    let window = adw::Window::new();
    window.set_default_size(700, 900);
    window.set_content(Some(&scrolled));
    window.present();
    pump(600);

    if !window.is_mapped() || scrolled.height() < 400 {
        eprintln!(
            "skipping: no mapped window to measure (mapped {}, {}px tall)",
            window.is_mapped(),
            scrolled.height()
        );
        window.destroy();
        return;
    }

    let adjustment = scrolled.vadjustment();
    let mut census = Census::default();
    let mut recycled_seen = 0u32;
    let mut fresh_skipped = 0u32;
    let mut peak_realized = 0usize;

    // Down, up, down, up, then two random sweeps. The same pooled rows are
    // rebound hundreds of times over the run.
    for sweep in 0..SWEEPS {
        for step in 0..=STOPS {
            let upper = adjustment.upper() - adjustment.page_size();
            adjustment.set_value(upper.max(0.0) * offset_fraction(sweep, step));
            pump(60);

            let rows = visible_rows(&list, &scrolled);
            peak_realized = peak_realized.max(realized(&list));
            for row in rows {
                let Some((post, kind)) = showing(&row) else {
                    continue;
                };
                let count = bind_counts
                    .borrow()
                    .get(&(row.as_ptr() as usize))
                    .copied()
                    .unwrap_or(0);
                if count < 2 {
                    fresh_skipped += 1;
                    continue;
                }
                recycled_seen += 1;
                let (verdict, sampled) = judge(&row, kind);
                census.record(kind, verdict, &sampled, post);
            }
        }
    }

    let total_binds: u32 = bind_counts.borrow().values().sum();
    let pool = bind_counts.borrow().len();
    eprintln!(
        "\nvirtualized list: {FEED} posts, {pool} pooled rows, {total_binds} binds, \
         {} unbinds, peak {peak_realized} realized, list {}px in a {}px viewport \
         against a {}px feed",
        unbinds.get(),
        list.height(),
        scrolled.height(),
        adjustment.upper(),
    );
    eprintln!(
        "observations: {recycled_seen} on recycled rows, {fresh_skipped} on rows not \
         yet recycled (skipped)"
    );
    census.report("recycled rows in a virtualized list");

    let bad = census.bad();
    let total = census.total();
    window.destroy();

    assert!(
        unbinds.get() > 0,
        "no row was ever unbound, so nothing was recycled and this measured nothing"
    );
    assert!(
        total_binds > 500,
        "only {total_binds} binds over four sweeps of a {FEED} post feed — the list is \
         not recycling and this measured nothing"
    );
    assert!(
        total > 200,
        "only {total} recycled rows were ever observed, which is too few to conclude \
         anything"
    );
    assert_eq!(
        bad, 0,
        "{bad} of {total} recycled-row observations had no media under the band"
    );
}

/// The rows a list has realized.
fn realized(list: &gtk4::ListView) -> usize {
    let children = list.observe_children();
    (0..children.n_items()).len()
}

/// Every `PostRow` currently overlapping the scroller's viewport.
fn visible_rows(list: &gtk4::ListView, scrolled: &gtk4::ScrolledWindow) -> Vec<PostRow> {
    let viewport = f64::from(scrolled.height());
    let children = list.observe_children();
    let mut out = Vec::new();
    for i in 0..children.n_items() {
        let Some(child) = children.item(i).and_downcast::<gtk4::Widget>() else {
            continue;
        };
        let mut all = Vec::new();
        subtree(&child, &mut all);
        let Some(row) = all.into_iter().find_map(|w| w.downcast::<PostRow>().ok()) else {
            continue;
        };
        if !row.is_mapped() {
            continue;
        }
        // Any row overlapping the viewport. "Wholly inside" would throw away
        // most of the feed at a 900px viewport with 700px rows, and a row's
        // allocation is the same whether or not the scroller clipped it.
        let Some(origin) = row.compute_point(scrolled, &gtk4::graphene::Point::new(0.0, 0.0))
        else {
            continue;
        };
        let top = f64::from(origin.y());
        let bottom = top + f64::from(row.height());
        if bottom > 0.0 && top < viewport {
            out.push(row);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 3. The same census, on the timeline the app itself builds
// ---------------------------------------------------------------------------

/// The shipped timeline, filled with the same five thousand mixed posts.
///
/// Reaches the widgets by walking the window instead of through its private
/// state, so this measures the page a user scrolls.
#[test]
fn the_shipped_timeline_keeps_media_on_recycled_rows() {
    crate::ui::with_gtk(the_shipped_timeline_keeps_media_on_recycled_rows_body);
}

fn the_shipped_timeline_keeps_media_on_recycled_rows_body() {
    let window: crate::ui::HangarWindow = glib::Object::builder().build();
    window.set_default_size(700, 900);

    // No video starts, whatever the settings file on this machine says. The
    // audit is about media being present and a live pipeline hits the network.
    if let Some(director) = crate::ui::inline_video::director() {
        director.set_autoplay(crate::state::VideoAutoplay::Never);
    }

    window.set_posts((0..FEED).map(audit_post).collect());
    window.present();

    // Up to six seconds for the page to map. A fixed wait made this skip
    // itself on a loaded machine.
    let mut found = None;
    for _ in 0..30 {
        pump(200);
        if let Some(pair) = timeline_of(&window)
            && pair.0.height() > 400
        {
            found = Some(pair);
            break;
        }
    }

    let Some((scrolled, list)) = found else {
        let mapped = window.is_mapped();
        window.destroy();
        assert!(
            !mapped,
            "the window mapped but no scroller holding the {FEED} audited posts ever \
             became {}px tall, so there was nothing to measure",
            400
        );
        eprintln!("skipping: the session never mapped a window at all");
        return;
    };

    let adjustment = scrolled.vadjustment();
    let mut census = Census::default();
    let mut seen_rows: HashMap<usize, u32> = HashMap::new();
    let mut recycled_seen = 0u32;

    // The first pass fills `seen_rows`. After that, a row showing a different
    // post than last time is a recycled row.
    for sweep in 0..SWEEPS {
        for step in 0..=STOPS {
            let upper = adjustment.upper() - adjustment.page_size();
            adjustment.set_value(upper.max(0.0) * offset_fraction(sweep, step));
            pump(60);

            for row in visible_rows(&list, &scrolled) {
                let Some((post, kind)) = showing(&row) else {
                    continue;
                };
                let key = row.as_ptr() as usize;
                let previously = seen_rows.insert(key, post);
                let recycled = matches!(previously, Some(earlier) if earlier != post);
                if !recycled {
                    continue;
                }
                recycled_seen += 1;
                let (verdict, sampled) = judge(&row, kind);
                census.record(kind, verdict, &sampled, post);
            }
        }
    }

    eprintln!(
        "\nshipped timeline: {} of {FEED} audited posts in the model, {} rows ever \
         seen, list {}px in a {}px viewport against a {}px feed, {recycled_seen} \
         observations of recycled rows",
        list.model().map(|m| m.n_items()).unwrap_or(0),
        seen_rows.len(),
        list.height(),
        scrolled.height(),
        adjustment.upper(),
    );
    census.report("shipped timeline, recycled rows");

    let bad = census.bad();
    let total = census.total();
    window.destroy();

    assert!(
        total > 200,
        "only {total} recycled-row observations on the shipped timeline, which is too \
         few to conclude anything"
    );
    assert_eq!(
        bad, 0,
        "{bad} of {total} recycled-row observations on the shipped timeline had no \
         media under the band"
    );
}

/// The mapped scroller and list showing the audited feed, found by walking.
fn timeline_of(window: &crate::ui::HangarWindow) -> Option<(gtk4::ScrolledWindow, gtk4::ListView)> {
    let mut all = Vec::new();
    subtree(window.upcast_ref(), &mut all);
    for widget in &all {
        let Ok(scrolled) = widget.clone().downcast::<gtk4::ScrolledWindow>() else {
            continue;
        };
        let mut inner = Vec::new();
        subtree(scrolled.upcast_ref(), &mut inner);
        let Some(list) = inner
            .into_iter()
            .find_map(|w| w.downcast::<gtk4::ListView>().ok())
        else {
            continue;
        };
        // Not `>= FEED`. `set_posts` runs the feed through
        // `filter_feed_posts`, which drops replies, so how many audited posts
        // survive depends on the settings file. Any list holding thousands of
        // them is the timeline.
        let items = list.model().map(|m| m.n_items()).unwrap_or(0);
        if items >= 1000 {
            return Some((scrolled, list));
        }
    }
    None
}
