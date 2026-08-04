// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::type_complexity)]

//! A pushed page listing an account's followers or follows.
//!
//! These pages stack (followers of A, a profile from it, followers of B),
//! so the cursor and in-flight flag live on the page rather than in a
//! window slot. A popped page takes its state with it.

use super::actor_row::{ActorObject, ActorRow};
use crate::atproto::Profile;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

/// How many fetches one user action may chain when pages come back short.
///
/// The AppView filters accounts out of pages, so a page can be empty or
/// short with a cursor still stored. Chaining fills the viewport; the cap
/// keeps a pathological server from looping us forever.
const BACKFILL_CAP: u8 = 5;

/// Which side of the follow graph a page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowListKind {
    Followers,
    Following,
}

impl FollowListKind {
    /// Navigation tag prefix, so the two lists for one account are
    /// separate pages.
    pub fn tag_prefix(self) -> &'static str {
        match self {
            FollowListKind::Followers => "followers",
            FollowListKind::Following => "following",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            FollowListKind::Followers => "Followers",
            FollowListKind::Following => "Following",
        }
    }

    fn empty_title(self) -> &'static str {
        match self {
            FollowListKind::Followers => "No followers yet",
            FollowListKind::Following => "Not following anyone",
        }
    }

    fn empty_description(self) -> &'static str {
        match self {
            FollowListKind::Followers => "People who follow this account will show up here.",
            FollowListKind::Following => "Accounts this person follows will show up here.",
        }
    }
}

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct FollowListPage {
        pub model: RefCell<Option<gio::ListStore>>,
        pub overlay: RefCell<Option<gtk4::Overlay>>,
        pub spinner: RefCell<Option<gtk4::Spinner>>,
        pub empty_state: RefCell<Option<adw::StatusPage>>,
        pub error_state: RefCell<Option<adw::StatusPage>>,
        pub vadjustment: RefCell<Option<gtk4::Adjustment>>,
        pub cursor: RefCell<Option<String>>,
        pub fetching: Cell<bool>,
        pub loaded_once: Cell<bool>,
        /// Chained fetches left before the next user action re-arms it.
        pub backfill_budget: Cell<u8>,
        pub load_more_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub retry_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub profile_activated_callback: RefCell<Option<Box<dyn Fn(Profile) + 'static>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FollowListPage {
        const NAME: &'static str = "HangarFollowListPage";
        type Type = super::FollowListPage;
        type ParentType = gtk4::Box;
    }

    impl ObjectImpl for FollowListPage {}
    impl WidgetImpl for FollowListPage {}
    impl BoxImpl for FollowListPage {}
}

glib::wrapper! {
    pub struct FollowListPage(ObjectSubclass<imp::FollowListPage>)
        @extends gtk4::Box, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Orientable;
}

impl FollowListPage {
    pub fn new(kind: FollowListKind) -> Self {
        let page: Self = glib::Object::builder()
            .property("orientation", gtk4::Orientation::Vertical)
            .property("spacing", 0)
            .build();
        page.setup_ui(kind);
        // Opening the page is the first user action.
        page.imp().backfill_budget.set(BACKFILL_CAP);
        page
    }

    fn setup_ui(&self, kind: FollowListKind) {
        self.set_vexpand(true);

        let overlay = gtk4::Overlay::new();
        overlay.set_vexpand(true);

        let model = gio::ListStore::new::<ActorObject>();
        let factory = gtk4::SignalListItemFactory::new();

        factory.connect_setup(|_, item| {
            let row = ActorRow::new();
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() {
                list_item.set_child(Some(&row));
            }
        });

        let page_weak = self.downgrade();
        factory.connect_bind(move |_, item| {
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(actor_object) = list_item.item().and_downcast::<ActorObject>()
                && let Some(profile) = actor_object.profile()
                && let Some(row) = list_item.child().and_downcast::<ActorRow>()
            {
                row.bind(&profile);
                let page_weak = page.downgrade();
                row.set_activated_callback(move |profile| {
                    let Some(page) = page_weak.upgrade() else {
                        return;
                    };
                    if let Some(cb) = page.imp().profile_activated_callback.borrow().as_ref() {
                        cb(profile);
                    }
                });
            }
        });

        let selection = gtk4::NoSelection::new(Some(model.clone()));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.add_css_class("background");

        // The virtualized chain every feed uses: scroller over clamp
        // scrollable over list view, nothing between. See `build_timeline`.
        let clamp = adw::ClampScrollable::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&clamp));
        overlay.set_child(Some(&scrolled));

        let spinner = gtk4::Spinner::new();
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        // Shown in place of the list once a first load comes back empty.
        let empty_state = adw::StatusPage::new();
        empty_state.set_icon_name(Some("system-users-symbolic"));
        empty_state.set_title(kind.empty_title());
        empty_state.set_description(Some(kind.empty_description()));
        empty_state.set_vexpand(true);
        empty_state.set_visible(false);

        // Shown when a fetch fails with nothing listed yet.
        let error_state = adw::StatusPage::new();
        error_state.set_icon_name(Some("dialog-warning-symbolic"));
        error_state.set_title("Couldn't Load This List");
        error_state.set_description(Some("Check your connection and try again."));
        error_state.set_vexpand(true);
        error_state.set_visible(false);

        let retry_button = gtk4::Button::with_label("Try Again");
        retry_button.add_css_class("pill");
        retry_button.add_css_class("suggested-action");
        retry_button.set_halign(gtk4::Align::Center);
        let page_weak = self.downgrade();
        retry_button.connect_clicked(move |_| {
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            // A retry is a user action.
            page.imp().backfill_budget.set(BACKFILL_CAP);
            if let Some(cb) = page.imp().retry_callback.borrow().as_ref() {
                cb();
            }
        });
        error_state.set_child(Some(&retry_button));

        self.append(&overlay);
        self.append(&empty_state);
        self.append(&error_state);

        // Wired once here. This only reports nearing the bottom; whether a
        // fetch is due is the callback's call.
        let adj = scrolled.vadjustment();
        let page_weak = self.downgrade();
        adj.connect_value_changed(move |adj| {
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            let value = adj.value();
            let upper = adj.upper();
            let page_size = adj.page_size();
            if value >= upper - page_size - 200.0
                && let Some(cb) = page.imp().load_more_callback.borrow().as_ref()
            {
                // A real scroll re-arms the chained-fetch budget.
                page.imp().backfill_budget.set(BACKFILL_CAP);
                cb();
            }
        });

        let imp = self.imp();
        imp.model.replace(Some(model));
        imp.overlay.replace(Some(overlay));
        imp.spinner.replace(Some(spinner));
        imp.empty_state.replace(Some(empty_state));
        imp.error_state.replace(Some(error_state));
        imp.vadjustment.replace(Some(adj));
    }

    /// Add a fetched page of profiles to the list.
    ///
    /// The first call also decides between list and empty state, so it must
    /// come from a completed fetch, never from an error path. Store the
    /// cursor first: a page can be empty with a cursor still to come, and
    /// only a spent cursor proves the list is empty.
    pub fn append_profiles(&self, profiles: Vec<Profile>) {
        let imp = self.imp();
        if let Some(model) = imp.model.borrow().as_ref() {
            for profile in profiles {
                model.append(&ActorObject::new(profile));
            }
        }
        imp.loaded_once.set(true);

        let empty = imp.model.borrow().as_ref().is_none_or(|m| m.n_items() == 0)
            && imp.cursor.borrow().is_none();
        if let Some(overlay) = imp.overlay.borrow().as_ref() {
            overlay.set_visible(!empty);
        }
        if let Some(empty_state) = imp.empty_state.borrow().as_ref() {
            empty_state.set_visible(empty);
        }
        if let Some(error_state) = imp.error_state.borrow().as_ref() {
            error_state.set_visible(false);
        }
    }

    /// Mark a fetch in flight and show or hide the spinner with it.
    ///
    /// Starting a fetch also stands the error state down; the retry path
    /// goes through here.
    pub fn set_fetching(&self, fetching: bool) {
        let imp = self.imp();
        imp.fetching.set(fetching);
        if fetching
            && let Some(error_state) = imp.error_state.borrow().as_ref()
            && error_state.is_visible()
        {
            error_state.set_visible(false);
            if let Some(overlay) = imp.overlay.borrow().as_ref() {
                overlay.set_visible(true);
            }
        }
        if let Some(spinner) = imp.spinner.borrow().as_ref() {
            spinner.set_visible(fetching);
            spinner.set_spinning(fetching);
        }
    }

    pub fn is_fetching(&self) -> bool {
        self.imp().fetching.get()
    }

    pub fn set_cursor(&self, cursor: Option<String>) {
        self.imp().cursor.replace(cursor);
    }

    /// The cursor for the next page, if the server said there is one.
    pub fn cursor(&self) -> Option<String> {
        self.imp().cursor.borrow().clone()
    }

    /// Swap in the error state after a failed fetch, unless rows are shown.
    ///
    /// With rows already listed the list stays up; the next scroll or a
    /// reopen fetches again from the stored cursor.
    pub fn show_load_failed(&self) {
        let imp = self.imp();
        let listed = imp.model.borrow().as_ref().is_some_and(|m| m.n_items() > 0);
        if listed {
            return;
        }
        if let Some(overlay) = imp.overlay.borrow().as_ref() {
            overlay.set_visible(false);
        }
        if let Some(empty_state) = imp.empty_state.borrow().as_ref() {
            empty_state.set_visible(false);
        }
        if let Some(error_state) = imp.error_state.borrow().as_ref() {
            error_state.set_visible(true);
        }
    }

    /// True when no fetch ever completed and none is under way, as after a
    /// failed first load. Reopening such a page fetches again.
    pub fn needs_reload(&self) -> bool {
        !self.imp().loaded_once.get() && !self.imp().fetching.get()
    }

    /// Chain another fetch when the list has a cursor and room to spare.
    ///
    /// Load-more rides `value_changed`, which a list shorter than its
    /// viewport never emits, and a page can even come back empty with a
    /// cursor. Call this after every appended page. The budget bounds the
    /// chain per user action; when it runs out with nothing listed, the
    /// empty state stands in.
    pub fn backfill_if_short(&self) {
        let imp = self.imp();
        if imp.fetching.get() || imp.cursor.borrow().is_none() {
            return;
        }
        if self.content_overflows() {
            return;
        }
        if imp.backfill_budget.get() == 0 {
            if imp.model.borrow().as_ref().is_none_or(|m| m.n_items() == 0) {
                if let Some(overlay) = imp.overlay.borrow().as_ref() {
                    overlay.set_visible(false);
                }
                if let Some(empty_state) = imp.empty_state.borrow().as_ref() {
                    empty_state.set_visible(true);
                }
            }
            return;
        }
        imp.backfill_budget.set(imp.backfill_budget.get() - 1);
        if let Some(cb) = imp.load_more_callback.borrow().as_ref() {
            cb();
        }
    }

    /// Whether the listed rows already extend past the viewport.
    fn content_overflows(&self) -> bool {
        self.imp()
            .vadjustment
            .borrow()
            .as_ref()
            .is_some_and(|adj| adj.upper() > adj.page_size() + 1.0)
    }

    /// Replace the handler run when a row's account is activated.
    pub fn set_profile_activated_callback<F: Fn(Profile) + 'static>(&self, callback: F) {
        self.imp()
            .profile_activated_callback
            .replace(Some(Box::new(callback)));
    }

    /// Replace the handler run when the list nears its bottom.
    pub fn set_load_more_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .load_more_callback
            .replace(Some(Box::new(callback)));
    }

    /// Replace the handler run by the error state's retry button.
    pub fn set_retry_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp().retry_callback.replace(Some(Box::new(callback)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_profile(handle: &str) -> Profile {
        Profile::minimal(
            format!("did:plc:{handle}"),
            format!("{handle}.bsky.social"),
            None,
            None,
        )
    }

    /// The empty state may only appear once a first load came back empty;
    /// before that the page is simply loading.
    #[test]
    fn the_empty_state_waits_for_the_first_load() {
        crate::ui::with_gtk(the_empty_state_waits_for_the_first_load_body);
    }

    fn the_empty_state_waits_for_the_first_load_body() {
        let page = FollowListPage::new(FollowListKind::Followers);
        let imp = page.imp();

        assert!(!imp.empty_state.borrow().as_ref().unwrap().is_visible());

        page.set_fetching(true);
        assert!(page.is_fetching());
        assert!(imp.spinner.borrow().as_ref().unwrap().is_visible());

        page.append_profiles(vec![]);
        page.set_fetching(false);

        assert!(!page.is_fetching());
        assert!(!imp.spinner.borrow().as_ref().unwrap().is_visible());
        assert!(imp.empty_state.borrow().as_ref().unwrap().is_visible());
        assert!(!imp.overlay.borrow().as_ref().unwrap().is_visible());
    }

    /// Pages accumulate in the model, the empty state stands down as soon
    /// as anyone is listed, and the cursor is whatever was stored last.
    #[test]
    fn profiles_accumulate_and_the_cursor_tracks_the_last_page() {
        crate::ui::with_gtk(profiles_accumulate_and_the_cursor_tracks_the_last_page_body);
    }

    fn profiles_accumulate_and_the_cursor_tracks_the_last_page_body() {
        let page = FollowListPage::new(FollowListKind::Following);
        let imp = page.imp();

        assert_eq!(page.cursor(), None);
        page.set_cursor(Some("page-2".into()));
        assert_eq!(page.cursor(), Some("page-2".into()));

        page.append_profiles(vec![a_profile("first"), a_profile("second")]);
        page.append_profiles(vec![a_profile("third")]);

        let model = imp.model.borrow().clone().unwrap();
        assert_eq!(model.n_items(), 3);
        assert!(!imp.empty_state.borrow().as_ref().unwrap().is_visible());
        assert!(imp.overlay.borrow().as_ref().unwrap().is_visible());

        page.set_cursor(None);
        assert_eq!(page.cursor(), None, "the last page clears the cursor");
    }

    /// The AppView can filter every account out of a page and still hand
    /// back a cursor. Such a page is mid-list: no empty state, and the next
    /// page is fetched without waiting for a scroll.
    #[test]
    fn an_empty_page_with_a_cursor_chains_the_next_fetch() {
        crate::ui::with_gtk(an_empty_page_with_a_cursor_chains_the_next_fetch_body);
    }

    fn an_empty_page_with_a_cursor_chains_the_next_fetch_body() {
        use std::cell::Cell;
        use std::rc::Rc;

        let page = FollowListPage::new(FollowListKind::Followers);
        let imp = page.imp();

        let fetches = Rc::new(Cell::new(0u32));
        let counter = fetches.clone();
        let page_weak = page.downgrade();
        page.set_load_more_callback(move || {
            counter.set(counter.get() + 1);
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            // The second page brings someone and ends the list.
            page.set_cursor(None);
            page.append_profiles(vec![a_profile("found")]);
            page.backfill_if_short();
        });

        // First page: everyone filtered out, cursor stored.
        page.set_cursor(Some("page-2".into()));
        page.append_profiles(vec![]);
        assert!(
            !imp.empty_state.borrow().as_ref().unwrap().is_visible(),
            "a stored cursor means the list is only part way through"
        );
        page.backfill_if_short();

        assert_eq!(fetches.get(), 1, "the empty page chained one fetch");
        let model = imp.model.borrow().clone().unwrap();
        assert_eq!(model.n_items(), 1);
        assert!(!imp.empty_state.borrow().as_ref().unwrap().is_visible());
        assert!(imp.overlay.borrow().as_ref().unwrap().is_visible());
    }

    /// A first page shorter than the viewport never moves the adjustment,
    /// so the next page is fetched right after the append.
    #[test]
    fn a_short_first_page_fetches_the_next_without_a_scroll() {
        crate::ui::with_gtk(a_short_first_page_fetches_the_next_without_a_scroll_body);
    }

    fn a_short_first_page_fetches_the_next_without_a_scroll_body() {
        use std::cell::Cell;
        use std::rc::Rc;

        let page = FollowListPage::new(FollowListKind::Following);
        let imp = page.imp();

        let fetches = Rc::new(Cell::new(0u32));
        let counter = fetches.clone();
        let page_weak = page.downgrade();
        page.set_load_more_callback(move || {
            counter.set(counter.get() + 1);
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            page.set_cursor(None);
            page.append_profiles(vec![a_profile("second"), a_profile("third")]);
            page.backfill_if_short();
        });

        page.set_cursor(Some("page-2".into()));
        page.append_profiles(vec![a_profile("first")]);
        page.backfill_if_short();

        assert_eq!(fetches.get(), 1, "the short page chained one fetch");
        let model = imp.model.borrow().clone().unwrap();
        assert_eq!(model.n_items(), 3);
        assert_eq!(page.cursor(), None);

        // The spent cursor ends the chain.
        page.backfill_if_short();
        assert_eq!(fetches.get(), 1, "no cursor, no fetch");
    }

    /// A server that hands back empty pages with fresh cursors forever gets
    /// cut off at the budget, and the page then admits it has nothing.
    #[test]
    fn chained_backfill_stops_at_the_budget() {
        crate::ui::with_gtk(chained_backfill_stops_at_the_budget_body);
    }

    fn chained_backfill_stops_at_the_budget_body() {
        use std::cell::Cell;
        use std::rc::Rc;

        let page = FollowListPage::new(FollowListKind::Followers);
        let imp = page.imp();

        let fetches = Rc::new(Cell::new(0u32));
        let counter = fetches.clone();
        let page_weak = page.downgrade();
        page.set_load_more_callback(move || {
            counter.set(counter.get() + 1);
            let Some(page) = page_weak.upgrade() else {
                return;
            };
            page.set_cursor(Some(format!("page-{}", counter.get() + 2)));
            page.append_profiles(vec![]);
            page.backfill_if_short();
        });

        page.set_cursor(Some("page-2".into()));
        page.append_profiles(vec![]);
        page.backfill_if_short();

        assert_eq!(
            fetches.get(),
            u32::from(super::BACKFILL_CAP),
            "the chain stops at the budget"
        );
        assert!(
            imp.empty_state.borrow().as_ref().unwrap().is_visible(),
            "with the budget spent and nothing listed, the page reads empty"
        );
        assert!(!imp.overlay.borrow().as_ref().unwrap().is_visible());
    }

    /// A failed first load shows the error state with a working retry, and
    /// the page reports it needs a reload until a fetch completes.
    #[test]
    fn a_failed_first_load_offers_retry_and_reload() {
        crate::ui::with_gtk(a_failed_first_load_offers_retry_and_reload_body);
    }

    fn a_failed_first_load_offers_retry_and_reload_body() {
        use std::cell::Cell;
        use std::rc::Rc;

        let page = FollowListPage::new(FollowListKind::Followers);
        let imp = page.imp();

        page.set_fetching(true);
        assert!(!page.needs_reload(), "a fetch in flight is progress");
        page.set_fetching(false);
        page.show_load_failed();

        assert!(imp.error_state.borrow().as_ref().unwrap().is_visible());
        assert!(!imp.overlay.borrow().as_ref().unwrap().is_visible());
        assert!(page.needs_reload(), "nothing loaded and nothing in flight");

        let retried = Rc::new(Cell::new(false));
        let flag = retried.clone();
        page.set_retry_callback(move || {
            flag.set(true);
        });
        let button = imp
            .error_state
            .borrow()
            .as_ref()
            .unwrap()
            .child()
            .and_downcast::<gtk4::Button>()
            .expect("the error state holds the retry button");
        button.emit_clicked();
        assert!(retried.get(), "the retry button runs the wired callback");

        // The retry fetch stands the error state down and then loads.
        page.set_fetching(true);
        assert!(!imp.error_state.borrow().as_ref().unwrap().is_visible());
        assert!(imp.overlay.borrow().as_ref().unwrap().is_visible());
        page.set_cursor(None);
        page.append_profiles(vec![a_profile("late")]);
        page.set_fetching(false);
        assert!(!page.needs_reload());

        // A later failure with rows on screen keeps the list up.
        page.show_load_failed();
        assert!(!imp.error_state.borrow().as_ref().unwrap().is_visible());
        assert!(imp.overlay.borrow().as_ref().unwrap().is_visible());
    }
}
