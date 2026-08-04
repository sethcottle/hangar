// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::type_complexity)]
#![allow(clippy::option_map_unit_fn)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_else_if)]

use super::actor_row::{ActorObject, ActorRow};
use super::follow_list_page::{FollowListKind, FollowListPage};
use super::message_page::{MessagePage, MessagePush};
use super::post_row::PostRow;
use super::sidebar::Sidebar;
use crate::atproto::{Conversation, Notification, Post, SavedFeed};
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

mod post_object {
    use super::*;

    mod imp {
        use super::*;

        #[derive(Default)]
        pub struct PostObject {
            pub post: RefCell<Option<Post>>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for PostObject {
            const NAME: &'static str = "HangarPostObject";
            type Type = super::PostObject;
            type ParentType = glib::Object;
        }

        impl ObjectImpl for PostObject {}
    }

    glib::wrapper! {
        pub struct PostObject(ObjectSubclass<imp::PostObject>);
    }

    impl PostObject {
        pub fn new(post: Post) -> Self {
            let obj: Self = glib::Object::builder().build();
            obj.imp().post.replace(Some(post));
            obj
        }

        pub fn post(&self) -> Option<Post> {
            self.imp().post.borrow().clone()
        }
    }
}

use post_object::PostObject;

/// What `push_follow_list_page` did with the request.
pub enum FollowListPush {
    /// A new page went onto the stack and needs wiring and a first fetch.
    Pushed(FollowListPage),
    /// An already-open copy was popped back to. Its callbacks are wired;
    /// it only needs a fetch if its first load never completed.
    PoppedBack(FollowListPage),
}

/// One drill-down profile page's feed state, shared between its tab
/// buttons, its scroll handler, and the app's fetch completions. The
/// generation strands a fetch whose tab was switched away mid-flight.
pub struct ProfileFeedCtx {
    /// A RefCell because the own page is built before sign-in fills it.
    pub did: RefCell<String>,
    pub filter: Cell<&'static str>,
    pub cursor: RefCell<Option<String>>,
    pub fetching: Cell<bool>,
    pub generation: Cell<u64>,
    model: gio::ListStore,
}

impl ProfileFeedCtx {
    /// Back to a clean first page: tab switches and reopens both start
    /// here, stranding whatever the previous state still had in flight.
    pub fn begin_refresh(&self) {
        self.generation.set(self.generation.get() + 1);
        self.cursor.replace(None);
        self.fetching.set(false);
        self.model.remove_all();
    }

    /// Posts landing from a fetch; the model stays this module's business.
    pub fn append_posts(&self, posts: Vec<Post>) {
        for post in posts {
            self.model.append(&PostObject::new(post));
        }
    }

    #[cfg(test)]
    fn listed(&self) -> u32 {
        self.model.n_items()
    }
}

mod notification_object {
    use super::*;

    mod imp {
        use super::*;

        #[derive(Default)]
        pub struct NotificationObject {
            pub notification: RefCell<Option<Notification>>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for NotificationObject {
            const NAME: &'static str = "HangarNotificationObject";
            type Type = super::NotificationObject;
            type ParentType = glib::Object;
        }

        impl ObjectImpl for NotificationObject {}
    }

    glib::wrapper! {
        pub struct NotificationObject(ObjectSubclass<imp::NotificationObject>);
    }

    impl NotificationObject {
        pub fn new(notification: Notification) -> Self {
            let obj: Self = glib::Object::builder().build();
            obj.imp().notification.replace(Some(notification));
            obj
        }

        pub fn notification(&self) -> Option<Notification> {
            self.imp().notification.borrow().clone()
        }
    }
}

use notification_object::NotificationObject;

mod conversation_object {
    use super::*;

    mod imp {
        use super::*;

        #[derive(Default)]
        pub struct ConversationObject {
            pub conversation: RefCell<Option<Conversation>>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for ConversationObject {
            const NAME: &'static str = "HangarConversationObject";
            type Type = super::ConversationObject;
            type ParentType = glib::Object;
        }

        impl ObjectImpl for ConversationObject {}
    }

    glib::wrapper! {
        pub struct ConversationObject(ObjectSubclass<imp::ConversationObject>);
    }

    impl ConversationObject {
        pub fn new(conversation: Conversation) -> Self {
            let obj: Self = glib::Object::builder().build();
            obj.imp().conversation.replace(Some(conversation));
            obj
        }

        pub fn conversation(&self) -> Option<Conversation> {
            self.imp().conversation.borrow().clone()
        }
    }
}

use conversation_object::ConversationObject;

use crate::atproto::Profile;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct HangarWindow {
        pub sidebar: RefCell<Option<Sidebar>>,
        // Main content stack for top-level pages (Home, Mentions, etc.)
        pub main_stack: RefCell<Option<gtk4::Stack>>,
        pub offline_banner: RefCell<Option<adw::Banner>>,
        // NavigationView for Home section (for thread/profile drill-down)
        pub home_nav_view: RefCell<Option<adw::NavigationView>>,
        // NavigationView for Mentions section (for thread/profile drill-down)
        pub mentions_nav_view: RefCell<Option<adw::NavigationView>>,
        pub timeline_model: RefCell<Option<gio::ListStore>>,
        pub timeline_list_view: RefCell<Option<gtk4::ListView>>,
        pub load_more_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub refresh_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub like_callback: RefCell<Option<Box<dyn Fn(Post, glib::WeakRef<PostRow>) + 'static>>>,
        pub repost_callback: RefCell<Option<Box<dyn Fn(Post, glib::WeakRef<PostRow>) + 'static>>>,
        pub quote_callback: RefCell<Option<Box<dyn Fn(Post) + 'static>>>,
        pub reply_callback: RefCell<Option<Box<dyn Fn(Post) + 'static>>>,
        pub compose_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub loading_spinner: RefCell<Option<gtk4::Spinner>>,
        // Empty states: each list's overlay and StatusPage trade places
        // when a load turns out empty. Search keeps its pages inside the
        // results stack, so those two toggle alone.
        pub timeline_overlay: RefCell<Option<gtk4::Overlay>>,
        pub timeline_empty_state: RefCell<Option<adw::StatusPage>>,
        pub mentions_overlay: RefCell<Option<gtk4::Overlay>>,
        pub mentions_empty_state: RefCell<Option<adw::StatusPage>>,
        pub activity_overlay: RefCell<Option<gtk4::Overlay>>,
        pub activity_empty_state: RefCell<Option<adw::StatusPage>>,
        pub likes_overlay: RefCell<Option<gtk4::Overlay>>,
        pub likes_empty_state: RefCell<Option<adw::StatusPage>>,
        pub search_empty_state: RefCell<Option<adw::StatusPage>>,
        pub search_people_empty_state: RefCell<Option<adw::StatusPage>>,
        // Error states: shown when a first load fails with nothing listed;
        // the Try Again button re-runs the section's own fetch path.
        pub timeline_error_state: RefCell<Option<adw::StatusPage>>,
        // Skeleton rows shown while a surface's first load is out.
        pub timeline_skeleton: RefCell<Option<gtk4::Box>>,
        pub mentions_skeleton: RefCell<Option<gtk4::Box>>,
        pub activity_skeleton: RefCell<Option<gtk4::Box>>,
        pub likes_skeleton: RefCell<Option<gtk4::Box>>,
        pub bookmarks_skeleton: RefCell<Option<gtk4::Box>>,
        pub chat_skeleton: RefCell<Option<gtk4::Box>>,
        pub mentions_error_state: RefCell<Option<adw::StatusPage>>,
        pub activity_error_state: RefCell<Option<adw::StatusPage>>,
        pub likes_error_state: RefCell<Option<adw::StatusPage>>,
        pub bookmarks_error_state: RefCell<Option<adw::StatusPage>>,
        pub chat_error_state: RefCell<Option<adw::StatusPage>>,
        pub search_suggestions_box: RefCell<Option<gtk4::Box>>,
        // New Message picker
        pub new_message_dialog: RefCell<Option<adw::Dialog>>,
        pub new_message_results_box: RefCell<Option<gtk4::Box>>,
        pub new_message_query_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
        pub new_message_chosen_callback: RefCell<Option<Box<dyn Fn(Profile) + 'static>>>,
        pub new_posts_banner: RefCell<Option<gtk4::Button>>,
        pub new_posts_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub scrolled_window: RefCell<Option<gtk4::ScrolledWindow>>,
        pub saved_scroll_position: RefCell<f64>,
        pub feed_btn_label: RefCell<Option<gtk4::Label>>,
        pub feed_menu: RefCell<Option<gio::Menu>>,
        pub feed_action: RefCell<Option<gio::SimpleAction>>,
        pub feed_changed_callback: RefCell<Option<Box<dyn Fn(SavedFeed) + 'static>>>,
        pub saved_feeds: RefCell<Vec<SavedFeed>>,
        pub current_feed_uri: RefCell<String>,
        // Navigation callbacks
        pub post_clicked_callback: RefCell<Option<Box<dyn Fn(Post) + 'static>>>,
        pub profile_clicked_callback: RefCell<Option<Box<dyn Fn(Profile) + 'static>>>,
        pub mention_clicked_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
        /// Args: subject DID, the page's follow-URI cell, the button.
        pub follow_callback: RefCell<
            Option<
                Box<
                    dyn Fn(String, Rc<RefCell<Option<String>>>, glib::WeakRef<gtk4::Button>)
                        + 'static,
                >,
            >,
        >,
        /// Args: the profile to message and the button that asked.
        pub message_callback:
            RefCell<Option<Box<dyn Fn(Profile, glib::WeakRef<gtk4::Button>) + 'static>>>,
        /// Args: the page's feed context and whether this is a first page.
        pub profile_tab_callback: RefCell<Option<Box<dyn Fn(Rc<ProfileFeedCtx>, bool) + 'static>>>,
        pub edit_profile_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub nav_changed_callback:
            RefCell<Option<Box<dyn Fn(crate::ui::sidebar::NavItem) + 'static>>>,
        // Mentions page state
        pub mentions_model: RefCell<Option<gio::ListStore>>,
        pub mentions_load_more_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub mentions_scrolled_window: RefCell<Option<gtk4::ScrolledWindow>>,
        pub mentions_spinner: RefCell<Option<gtk4::Spinner>>,
        // Activity page state
        pub activity_nav_view: RefCell<Option<adw::NavigationView>>,
        pub activity_model: RefCell<Option<gio::ListStore>>,
        pub activity_load_more_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub activity_scrolled_window: RefCell<Option<gtk4::ScrolledWindow>>,
        pub activity_spinner: RefCell<Option<gtk4::Spinner>>,
        // Chat page state
        pub chat_nav_view: RefCell<Option<adw::NavigationView>>,
        pub chat_model: RefCell<Option<gio::ListStore>>,
        pub chat_load_more_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub chat_scrolled_window: RefCell<Option<gtk4::ScrolledWindow>>,
        pub chat_spinner: RefCell<Option<gtk4::Spinner>>,
        pub chat_overlay: RefCell<Option<gtk4::Overlay>>,
        pub chat_empty_state: RefCell<Option<adw::StatusPage>>,
        pub conversation_clicked_callback: RefCell<Option<Box<dyn Fn(Conversation) + 'static>>>,
        // Current user info
        pub current_user_did: RefCell<Option<String>>,
        // Profile page state (for own profile in sidebar)
        pub profile_nav_view: RefCell<Option<adw::NavigationView>>,
        pub profile_page_model: RefCell<Option<gio::ListStore>>,
        pub own_profile_feed_ctx: RefCell<Option<Rc<ProfileFeedCtx>>>,
        pub profile_page_spinner: RefCell<Option<gtk4::Spinner>>,
        pub profile_page_scrolled: RefCell<Option<gtk4::ScrolledWindow>>,
        // Store current profile for updating UI
        pub current_profile: RefCell<Option<Profile>>,
        // Profile page widgets (for updating header)
        pub profile_banner_picture: RefCell<Option<gtk4::Picture>>,
        pub profile_name_label: RefCell<Option<gtk4::Label>>,
        pub profile_handle_label: RefCell<Option<gtk4::Label>>,
        pub profile_bio_label: RefCell<Option<gtk4::Label>>,
        pub profile_avatar: RefCell<Option<adw::Avatar>>,
        pub profile_followers_label: RefCell<Option<gtk4::Label>>,
        pub profile_following_label: RefCell<Option<gtk4::Label>>,
        pub profile_posts_label: RefCell<Option<gtk4::Label>>,
        // Likes page state
        pub likes_nav_view: RefCell<Option<adw::NavigationView>>,
        pub likes_model: RefCell<Option<gio::ListStore>>,
        pub likes_load_more_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub likes_scrolled_window: RefCell<Option<gtk4::ScrolledWindow>>,
        pub likes_spinner: RefCell<Option<gtk4::Spinner>>,
        // Saved posts page state
        pub bookmarks_nav_view: RefCell<Option<adw::NavigationView>>,
        pub bookmarks_model: RefCell<Option<gio::ListStore>>,
        pub bookmarks_load_more_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub bookmarks_scrolled_window: RefCell<Option<gtk4::ScrolledWindow>>,
        pub bookmarks_spinner: RefCell<Option<gtk4::Spinner>>,
        pub bookmarks_overlay: RefCell<Option<gtk4::Overlay>>,
        pub bookmarks_empty_state: RefCell<Option<adw::StatusPage>>,
        // Search page state
        pub search_nav_view: RefCell<Option<adw::NavigationView>>,
        pub search_model: RefCell<Option<gio::ListStore>>,
        pub search_load_more_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub search_scrolled_window: RefCell<Option<gtk4::ScrolledWindow>>,
        pub search_spinner: RefCell<Option<gtk4::Spinner>>,
        pub search_entry: RefCell<Option<gtk4::SearchEntry>>,
        pub search_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
        // People results live beside the post results on their own stack
        // page; the two lists paginate on independent cursors.
        pub search_people_model: RefCell<Option<gio::ListStore>>,
        pub search_people_load_more_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub search_people_scrolled_window: RefCell<Option<gtk4::ScrolledWindow>>,
        pub search_people_spinner: RefCell<Option<gtk4::Spinner>>,
        // Toast overlay for notifications
        pub toast_overlay: RefCell<Option<adw::ToastOverlay>>,
        // Settings page
        pub font_size_css_provider: RefCell<Option<gtk4::CssProvider>>,
        pub font_size_changed_callback:
            RefCell<Option<Box<dyn Fn(crate::state::FontSize) + 'static>>>,
        pub reduce_motion_css_provider: RefCell<Option<gtk4::CssProvider>>,
        // Track which page was visible before settings (for back button)
        pub previous_page: RefCell<Option<String>>,
        /// Only strong ref; rows find it through a thread-local `Weak`. See
        /// `ui::inline_video`.
        pub video_director: RefCell<Option<Rc<crate::ui::inline_video::VideoDirector>>>,
        /// Poll-inserted posts the user has not scrolled up to yet. One
        /// number feeds both the banner and the Home badge so they cannot
        /// disagree.
        pub unseen_posts: Cell<usize>,
        // Follower and following stat clicks, own profile and pushed pages alike
        pub follow_list_clicked_callback:
            RefCell<Option<Box<dyn Fn(Profile, FollowListKind) + 'static>>>,
        // The own-profile stat boxes, so their spoken labels can follow the counts
        pub profile_followers_box: RefCell<Option<gtk4::Box>>,
        pub profile_following_box: RefCell<Option<gtk4::Box>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for HangarWindow {
        const NAME: &'static str = "HangarWindow";
        type Type = super::HangarWindow;
        type ParentType = adw::ApplicationWindow;
    }

    impl ObjectImpl for HangarWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_ui();
        }
    }

    impl WidgetImpl for HangarWindow {}
    impl WindowImpl for HangarWindow {}
    impl ApplicationWindowImpl for HangarWindow {}
    impl AdwApplicationWindowImpl for HangarWindow {}
}

glib::wrapper! {
    pub struct HangarWindow(ObjectSubclass<imp::HangarWindow>)
        @extends adw::ApplicationWindow, gtk4::ApplicationWindow, gtk4::Window, gtk4::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk4::Accessible, gtk4::Buildable,
                    gtk4::ConstraintTarget, gtk4::Native, gtk4::Root, gtk4::ShortcutManager;
}

impl HangarWindow {
    pub fn new(app: &adw::Application) -> Self {
        let title = if crate::config::IS_DEVEL {
            "Hangar (Nightly)"
        } else {
            "Hangar"
        };
        let window: Self = glib::Object::builder()
            .property("application", app)
            .property("default-width", 620)
            .property("default-height", 780)
            .property("title", title)
            .build();
        window.restore_window_state();
        window
    }

    fn setup_ui(&self) {
        // First, before any page is built: `build_timeline` hands it the
        // scroller, and every `PostRow` a factory creates from here on looks it
        // up to register its video embed.
        let director =
            crate::ui::inline_video::VideoDirector::new(&crate::state::AppSettings::load());
        crate::ui::inline_video::set_director(&director);
        self.imp().video_director.replace(Some(director));

        let main_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);

        let sidebar = Sidebar::new();
        main_box.append(&sidebar);

        let separator = gtk4::Separator::new(gtk4::Orientation::Vertical);
        main_box.append(&separator);

        // Main stack for top-level pages (no animation for sidebar switching)
        let main_stack = gtk4::Stack::new();
        main_stack.set_hexpand(true);
        main_stack.set_vexpand(true);
        main_stack.set_transition_type(gtk4::StackTransitionType::None);

        // Home section: NavigationView for thread/profile drill-down
        let home_nav_view = adw::NavigationView::new();
        Self::focus_after_pop(&home_nav_view);
        let timeline_page = self.build_timeline_page();
        home_nav_view.add(&timeline_page);
        // Restore scroll position when a page is popped (user navigates back)
        home_nav_view.connect_popped(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                // Restore saved scroll position after a short delay to let GTK settle
                glib::idle_add_local_once(glib::clone!(
                    #[weak]
                    window,
                    move || {
                        let imp = window.imp();
                        let saved_pos = *imp.saved_scroll_position.borrow();
                        if let Some(scrolled) = imp.scrolled_window.borrow().as_ref() {
                            scrolled.vadjustment().set_value(saved_pos);
                        }
                    }
                ));
            }
        ));
        main_stack.add_named(&home_nav_view, Some("home"));

        // Mentions section: NavigationView for thread/profile drill-down
        let mentions_nav_view = adw::NavigationView::new();
        Self::focus_after_pop(&mentions_nav_view);
        let mentions_page = self.build_mentions_page();
        mentions_nav_view.add(&mentions_page);
        main_stack.add_named(&mentions_nav_view, Some("mentions"));

        // Activity section: NavigationView for thread/profile drill-down
        let activity_nav_view = adw::NavigationView::new();
        Self::focus_after_pop(&activity_nav_view);
        let activity_page = self.build_activity_page();
        activity_nav_view.add(&activity_page);
        main_stack.add_named(&activity_nav_view, Some("activity"));

        // Chat section: NavigationView for conversation drill-down
        let chat_nav_view = adw::NavigationView::new();
        Self::focus_after_pop(&chat_nav_view);
        let chat_page = self.build_chat_page();
        chat_nav_view.add(&chat_page);
        main_stack.add_named(&chat_nav_view, Some("chat"));

        // Profile section: NavigationView for own profile
        let profile_nav_view = adw::NavigationView::new();
        Self::focus_after_pop(&profile_nav_view);
        let profile_page = self.build_own_profile_page();
        profile_nav_view.add(&profile_page);
        main_stack.add_named(&profile_nav_view, Some("profile"));

        // Likes section: NavigationView for liked posts
        let likes_nav_view = adw::NavigationView::new();
        Self::focus_after_pop(&likes_nav_view);
        let likes_page = self.build_likes_page();
        likes_nav_view.add(&likes_page);
        main_stack.add_named(&likes_nav_view, Some("likes"));

        // Saved posts section: NavigationView for thread/profile drill-down
        let bookmarks_nav_view = adw::NavigationView::new();
        Self::focus_after_pop(&bookmarks_nav_view);
        let bookmarks_page = self.build_bookmarks_page();
        bookmarks_nav_view.add(&bookmarks_page);
        main_stack.add_named(&bookmarks_nav_view, Some("bookmarks"));

        // Search section: NavigationView for search results
        let search_nav_view = adw::NavigationView::new();
        Self::focus_after_pop(&search_nav_view);
        let search_page = self.build_search_page();
        search_nav_view.add(&search_page);
        main_stack.add_named(&search_nav_view, Some("search"));

        // Settings section
        let settings_page = self.build_settings_page();
        main_stack.add_named(&settings_page, Some("settings"));

        // A page switch changes what is on screen without scrolling or a model
        // change, so re-run the election. Do not suspend on a page switch -
        // that refused the play button on five feeds.
        main_stack.connect_visible_child_notify(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                let Some(director) = window.imp().video_director.borrow().clone() else {
                    return;
                };
                director.page_changed();
            }
        ));

        // Banner above the stack: connectivity is page-independent.
        let offline_banner = adw::Banner::new("You're offline");
        let content_column = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_column.set_hexpand(true);
        content_column.append(&offline_banner);
        content_column.append(&main_stack);
        main_box.append(&content_column);
        self.imp().offline_banner.replace(Some(offline_banner));

        // Wrap in AdwToastOverlay
        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&main_box));

        self.set_content(Some(&toast_overlay));

        let imp = self.imp();
        imp.toast_overlay.replace(Some(toast_overlay));
        imp.sidebar.replace(Some(sidebar));
        imp.main_stack.replace(Some(main_stack));
        imp.home_nav_view.replace(Some(home_nav_view));
        imp.mentions_nav_view.replace(Some(mentions_nav_view));
        imp.activity_nav_view.replace(Some(activity_nav_view));
        imp.chat_nav_view.replace(Some(chat_nav_view));
        imp.profile_nav_view.replace(Some(profile_nav_view));
        imp.likes_nav_view.replace(Some(likes_nav_view));
        imp.bookmarks_nav_view.replace(Some(bookmarks_nav_view));
        imp.search_nav_view.replace(Some(search_nav_view));

        // Narrow windows drop the rail's captions; icons, tooltips, and
        // accessible names keep carrying the meaning. Watched by property
        // rather than an AdwBreakpoint: a breakpoint surrenders the
        // window's minimum size, and pages that cannot shrink that far
        // clipped at the edge instead of adapting.
        self.connect_default_width_notify(|win| {
            if let Some(sidebar) = win.imp().sidebar.borrow().as_ref() {
                sidebar.set_compact(win.default_width() < 640);
            }
        });

        // Keyboard shortcuts live on the application as GActions; see
        // `HangarApplication::setup_gactions`.

        // Window state is saved on the way out, restored in `new`.
        self.connect_close_request(|window| {
            window.save_window_state();
            glib::Propagation::Proceed
        });
    }

    /// The installed gschema, when there is one. A plain `Settings::new`
    /// aborts without the schema, which is exactly what a `cargo run`
    /// outside the flatpak looks like, so the lookup guards it. Stable and
    /// nightly share the schema id and therefore the remembered geometry.
    fn state_settings() -> Option<gio::Settings> {
        gio::SettingsSchemaSource::default()?.lookup("io.github.sethcottle.Hangar", true)?;
        Some(gio::Settings::new("io.github.sethcottle.Hangar"))
    }

    /// Open at the size the window closed at. Without the schema this is
    /// a no-op and the builder defaults stand.
    fn restore_window_state(&self) {
        let Some(settings) = Self::state_settings() else {
            return;
        };
        self.set_default_size(settings.int("window-width"), settings.int("window-height"));
        if settings.boolean("window-maximized") {
            self.maximize();
        }
    }

    fn save_window_state(&self) {
        let Some(settings) = Self::state_settings() else {
            return;
        };
        let (width, height) = self.default_size();
        let _ = settings.set_int("window-width", width);
        let _ = settings.set_int("window-height", height);
        let _ = settings.set_boolean("window-maximized", self.is_maximized());
    }

    /// Pop the visible section's stack one page. At the root this does
    /// nothing, matching what a headerbar back button would do.
    pub fn go_back(&self) {
        if let Some(nav_view) = self.current_nav_view() {
            nav_view.pop();
        }
    }

    /// The shortcuts reference, built fresh per showing.
    fn build_help_overlay() -> gtk4::ShortcutsWindow {
        let shortcut = |title: &str, accelerator: &str| -> gtk4::ShortcutsShortcut {
            glib::Object::builder()
                .property("title", title)
                .property("accelerator", accelerator)
                .build()
        };

        let general: gtk4::ShortcutsGroup =
            glib::Object::builder().property("title", "General").build();
        general.add_shortcut(&shortcut("Refresh", "F5 <Control>R"));
        general.add_shortcut(&shortcut("New Post", "<Control>N"));
        // Ctrl+K also focuses the entry; both land on the same page.
        general.add_shortcut(&shortcut("Search", "<Control>K <Alt>8"));
        general.add_shortcut(&shortcut("Keyboard Shortcuts", "<Control>question"));

        let navigation: gtk4::ShortcutsGroup = glib::Object::builder()
            .property("title", "Navigation")
            .build();
        navigation.add_shortcut(&shortcut("Back", "<Alt>Left"));
        for (i, item) in crate::ui::sidebar::NavItem::all().iter().enumerate() {
            // Search is already listed under General with both of its keys.
            if *item != crate::ui::sidebar::NavItem::Search {
                navigation.add_shortcut(&shortcut(item.label(), &format!("<Alt>{}", i + 1)));
            }
        }

        // Single letters, live while a post row has keyboard focus.
        let posts: gtk4::ShortcutsGroup = glib::Object::builder()
            .property("title", "Focused Post")
            .build();
        posts.add_shortcut(&shortcut("Next Post", "J"));
        posts.add_shortcut(&shortcut("Previous Post", "K"));
        posts.add_shortcut(&shortcut("Like", "L"));
        posts.add_shortcut(&shortcut("Reply", "R"));
        posts.add_shortcut(&shortcut("Repost or Quote", "T"));
        posts.add_shortcut(&shortcut("Open Thread", "O"));
        posts.add_shortcut(&shortcut("More Options", "M"));

        let section: gtk4::ShortcutsSection = glib::Object::builder()
            .property("section-name", "shortcuts")
            .build();
        section.add_group(&general);
        section.add_group(&navigation);
        section.add_group(&posts);

        let window: gtk4::ShortcutsWindow = glib::Object::builder().property("modal", true).build();
        window.add_section(&section);
        window
    }

    pub fn present_shortcuts_window(&self) {
        let overlay = Self::build_help_overlay();
        overlay.set_transient_for(Some(self));
        overlay.present();
    }

    fn build_timeline_page(&self) -> adw::NavigationPage {
        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        // Feed selector button with popover
        let feed_menu_btn = gtk4::MenuButton::new();
        let feed_label_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let title_label = gtk4::Label::new(Some("Following"));
        title_label.add_css_class("title");
        feed_label_box.append(&title_label);
        let dropdown_icon = gtk4::Image::from_icon_name("pan-down-symbolic");
        feed_label_box.append(&dropdown_icon);
        feed_menu_btn.set_child(Some(&feed_label_box));
        feed_menu_btn.add_css_class("flat");

        // The menu is a GMenu; the stateful action's string state marks
        // the current feed with the radio indicator, no custom rows.
        let feed_menu = gio::Menu::new();
        feed_menu_btn.set_menu_model(Some(&feed_menu));

        let feed_action = gio::SimpleAction::new_stateful(
            "select",
            Some(glib::VariantTy::STRING),
            &"".to_variant(),
        );
        let win = self.downgrade();
        feed_action.connect_activate(move |_, parameter| {
            let Some(win) = win.upgrade() else {
                return;
            };
            let Some(uri) = parameter.and_then(|v| v.get::<String>()) else {
                return;
            };
            let feed = {
                let feeds = win.imp().saved_feeds.borrow();
                feeds.iter().find(|f| f.uri == uri).cloned()
            };
            if let Some(feed) = feed {
                if let Some(cb) = win.imp().feed_changed_callback.borrow().as_ref() {
                    cb(feed);
                }
            }
        });
        let feed_group = gio::SimpleActionGroup::new();
        feed_group.add_action(&feed_action);
        self.insert_action_group("feeds", Some(&feed_group));

        header.set_title_widget(Some(&feed_menu_btn));

        // Store references
        self.imp().feed_btn_label.replace(Some(title_label));
        self.imp().feed_menu.replace(Some(feed_menu));
        self.imp().feed_action.replace(Some(feed_action));

        let refresh_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
        refresh_btn.set_tooltip_text(Some("Refresh"));
        refresh_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                if let Some(cb) = window.imp().refresh_callback.borrow().as_ref() {
                    cb();
                }
            }
        ));
        header.pack_start(&refresh_btn);

        // Add window controls (close, minimize, maximize) to the end
        let window_controls = gtk4::WindowControls::new(gtk4::PackType::End);
        header.pack_end(&window_controls);

        content_box.append(&header);
        content_box.append(&self.build_timeline());

        let page = adw::NavigationPage::new(&content_box, "Home");
        page.set_tag(Some("timeline"));
        page
    }

    fn build_timeline(&self) -> gtk4::Box {
        let timeline_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        timeline_box.set_vexpand(true);

        // Overlay for the "N new posts" banner
        let overlay = gtk4::Overlay::new();
        overlay.set_vexpand(true);

        let model = gio::ListStore::new::<PostObject>();
        let factory = gtk4::SignalListItemFactory::new();

        factory.connect_setup(|_, item| {
            let post_row = PostRow::new();
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() {
                list_item.set_child(Some(&post_row));
            }
        });
        Self::release_video_on_unbind(&factory);

        // Weak, and so is every callback below: window -> list view -> factory
        // -> this closure is a cycle, and `VideoDirector::drop` would never run.
        factory.connect_bind(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, item| {
                if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                    && let Some(post_object) = list_item.item().and_downcast::<PostObject>()
                    && let Some(post) = post_object.post()
                    && let Some(post_row) = list_item.child().and_downcast::<PostRow>()
                {
                    post_row.bind(&post);
                    post_row.set_list_position(list_item.position());
                    // Like callback receives (post_row, was_liked, like_uri) captured before toggle
                    let post_for_like = post.clone();
                    let w = win.downgrade();
                    post_row.connect_like_clicked(move |row, was_liked, like_uri| {
                        let Some(w) = w.upgrade() else {
                            return;
                        };
                        let mut post_with_state = post_for_like.clone();
                        // If it was liked, we're unliking, so pass the like_uri
                        // If it wasn't liked, we're liking, so pass None
                        post_with_state.viewer_like = if was_liked { like_uri } else { None };
                        let row_weak = row.downgrade();
                        if let Some(cb) = w.imp().like_callback.borrow().as_ref() {
                            cb(post_with_state, row_weak);
                        }
                    });
                    // Repost callback receives (post_row, was_reposted, repost_uri) captured before toggle
                    let post_for_repost = post.clone();
                    let w = win.downgrade();
                    post_row.connect_repost_clicked(move |row, was_reposted, repost_uri| {
                        let Some(w) = w.upgrade() else {
                            return;
                        };
                        let mut post_with_state = post_for_repost.clone();
                        post_with_state.viewer_repost =
                            if was_reposted { repost_uri } else { None };
                        let row_weak = row.downgrade();
                        if let Some(cb) = w.imp().repost_callback.borrow().as_ref() {
                            cb(post_with_state, row_weak);
                        }
                    });
                    let post_for_quote = post.clone();
                    let w = win.downgrade();
                    post_row.connect_quote_clicked(move || {
                        let Some(w) = w.upgrade() else {
                            return;
                        };
                        if let Some(cb) = w.imp().quote_callback.borrow().as_ref() {
                            cb(post_for_quote.clone());
                        }
                    });
                    let post_clone = post.clone();
                    let w = win.downgrade();
                    post_row.connect_reply_clicked(move || {
                        let Some(w) = w.upgrade() else {
                            return;
                        };
                        if let Some(cb) = w.imp().reply_callback.borrow().as_ref() {
                            cb(post_clone.clone());
                        }
                    });
                    // Navigation callbacks
                    let w = win.downgrade();
                    post_row.set_post_clicked_callback(move |p| {
                        let Some(w) = w.upgrade() else {
                            return;
                        };
                        if let Some(cb) = w.imp().post_clicked_callback.borrow().as_ref() {
                            cb(p);
                        }
                    });
                    let w = win.downgrade();
                    post_row.set_profile_clicked_callback(move |profile| {
                        let Some(w) = w.upgrade() else {
                            return;
                        };
                        if let Some(cb) = w.imp().profile_clicked_callback.borrow().as_ref() {
                            cb(profile);
                        }
                    });
                    let w = win.downgrade();
                    post_row.set_mention_clicked_callback(move |handle| {
                        let Some(w) = w.upgrade() else {
                            return;
                        };
                        if let Some(cb) = w.imp().mention_clicked_callback.borrow().as_ref() {
                            cb(handle);
                        }
                    });
                }
            }
        ));

        let selection = gtk4::NoSelection::new(Some(model.clone()));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.add_css_class("background");

        // AdwClampScrollable. Same AdwClampLayout and the same content width
        // as AdwClamp (800 max, tightening from 600); the difference is that it
        // implements GtkScrollable.
        //
        // A GtkScrolledWindow handed a non-scrollable child interposes a
        // GtkViewport, and a viewport allocates its child the child's natural
        // height. The GtkListView was told it had 1,254,000px to fill inside a
        // 900px window, so it never virtualized: 205 rows realized, nothing
        // painted past them, and `unbind` never fired, so
        // `release_video_on_unbind` never ran.
        //
        // AdwClampScrollable forwards its adjustments and scroll policies
        // straight through to its child, which makes the chain rigid. The list
        // has to be this widget's direct child and this widget the scroller's.
        // A GtkBox or GtkOverlay in between has no such properties to bind, so
        // GLib warns, `upper` stays 0 and the page stops scrolling. Hence the
        // spinner overlays outside the scroller, and the plain AdwClamp on the
        // thread page and the settings sidebar, whose children are boxes.
        let clamp = adw::ClampScrollable::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        // Horizontal policy Never is a backstop: it hands the rows' minimum
        // width to the window instead of scrolling, so the ellipsized labels
        // in PostRow are what keep it small.
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&clamp));
        overlay.set_child(Some(&scrolled));

        // "N new posts" banner with icon
        let new_posts_btn = gtk4::Button::new();
        let banner_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let banner_icon = gtk4::Image::from_icon_name("go-up-symbolic");
        banner_icon.add_css_class("banner-icon");
        let banner_label = gtk4::Label::new(Some("New posts"));
        banner_label.add_css_class("banner-label");
        banner_box.append(&banner_icon);
        banner_box.append(&banner_label);
        new_posts_btn.set_child(Some(&banner_box));
        new_posts_btn.add_css_class("suggested-action");
        new_posts_btn.add_css_class("pill");
        new_posts_btn.add_css_class("new-posts-banner");
        new_posts_btn.set_halign(gtk4::Align::Center);
        new_posts_btn.set_valign(gtk4::Align::Start);
        new_posts_btn.set_margin_top(12);
        new_posts_btn.set_visible(false);
        new_posts_btn.set_opacity(0.0);
        new_posts_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                if let Some(cb) = window.imp().new_posts_callback.borrow().as_ref() {
                    cb();
                }
            }
        ));
        overlay.add_overlay(&new_posts_btn);

        // Loading spinner as an overlay at the bottom
        let spinner = gtk4::Spinner::new();
        spinner.update_property(&[gtk4::accessible::Property::Label("Loading")]);
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        timeline_box.append(&overlay);

        let empty_state = Self::empty_state_page(
            "go-home-symbolic",
            "Your feed is empty",
            "Posts from accounts you follow will appear here.",
        );
        timeline_box.append(&empty_state);
        let win = self.downgrade();
        let error_state = Self::error_state_page(move || {
            if let Some(win) = win.upgrade() {
                win.refresh_feed();
            }
        });
        timeline_box.append(&error_state);
        self.imp().timeline_error_state.replace(Some(error_state));

        let skeleton = Self::skeleton_list();
        timeline_box.append(&skeleton);
        self.imp().timeline_skeleton.replace(Some(skeleton));

        self.imp().timeline_overlay.replace(Some(overlay.clone()));
        self.imp().timeline_empty_state.replace(Some(empty_state));

        // Autoplay is scoped to this scroller. Other feeds play inline only on
        // a press.
        if let Some(director) = self.imp().video_director.borrow().as_ref() {
            director.set_timeline_scroller(&scrolled);
        }
        // A refresh or an appended page can bring a video onto a screen that is
        // not being scrolled, so the model gets a trigger of its own.
        model.connect_items_changed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, _, _, _| win.schedule_video_election()
        ));

        let imp = self.imp();
        imp.timeline_model.replace(Some(model));
        imp.timeline_list_view.replace(Some(list_view.clone()));
        imp.loading_spinner.replace(Some(spinner));
        imp.new_posts_banner.replace(Some(new_posts_btn));
        imp.scrolled_window.replace(Some(scrolled.clone()));

        // Start timestamp refresh timer (every 60 seconds)
        glib::timeout_add_seconds_local(
            60,
            glib::clone!(
                #[weak]
                list_view,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    // Iterate over visible children and refresh their timestamps
                    let mut child = list_view.first_child();
                    while let Some(widget) = child {
                        if let Some(post_row) = widget.downcast_ref::<PostRow>() {
                            post_row.refresh_timestamp();
                        }
                        child = widget.next_sibling();
                    }
                    glib::ControlFlow::Continue
                }
            ),
        );

        let adj = scrolled.vadjustment();
        adj.connect_value_changed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |adj| {
                let value = adj.value();
                let upper = adj.upper();
                let page_size = adj.page_size();

                // This fires per scroll frame, so only re-arm the debounce here.
                win.schedule_video_election();

                // Auto-hide "new posts" banner when user scrolls to top
                if value < 50.0 {
                    win.hide_new_posts_banner();
                }

                // Prefetch when user is at 70% scroll (30% remaining content)
                // This gives more time for content to load before user reaches the end
                let scroll_threshold = (upper - page_size) * 0.7;
                if value >= scroll_threshold
                    && let Some(cb) = win.imp().load_more_callback.borrow().as_ref()
                {
                    cb();
                }
            }
        ));

        timeline_box
    }

    pub fn set_load_more_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .load_more_callback
            .replace(Some(Box::new(callback)));
    }

    pub fn set_refresh_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .refresh_callback
            .replace(Some(Box::new(callback)));
    }

    /// Make a `PostRow` factory give up its video when a row is recycled.
    ///
    /// Every `PostRow` factory needs this, the timeline's and the rest: a
    /// video started on the Likes page and scrolled past would otherwise keep
    /// its decoder and bus watch bound to a rebound row. `release_video` is
    /// idempotent.
    ///
    /// Fires constantly now that the lists virtualize. The own profile page
    /// wraps its rows, so the row is looked up with [`Self::post_row_of`].
    pub(crate) fn release_video_on_unbind(factory: &gtk4::SignalListItemFactory) {
        factory.connect_unbind(|_, item| {
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(post_row) = Self::post_row_of(list_item)
            {
                post_row.release_video();
            }
        });
    }

    /// The `PostRow` a list slot is showing.
    ///
    /// Most feeds set a bare `PostRow` as the slot child. The own profile page
    /// sets a box holding a `PostRow` plus a host for the page header, so there
    /// the row is one level down. One level only; this is a lookup for a known
    /// shape.
    fn post_row_of(list_item: &gtk4::ListItem) -> Option<PostRow> {
        let child = list_item.child()?;
        if let Ok(post_row) = child.clone().downcast::<PostRow>() {
            return Some(post_row);
        }
        let mut candidate = child.first_child();
        while let Some(widget) = candidate {
            if let Ok(post_row) = widget.clone().downcast::<PostRow>() {
                return Some(post_row);
            }
            candidate = widget.next_sibling();
        }
        None
    }

    /// Re-arm the election timer if a director exists.
    fn schedule_video_election(&self) {
        let director = self.imp().video_director.borrow().clone();
        if let Some(director) = director {
            director.schedule_election();
        }
    }

    /// Refetch the timeline, exactly as the refresh button does.
    ///
    /// `filter_feed_posts` runs as posts enter the model, so posts already in it
    /// cannot be re-filtered.
    pub fn refresh_feed(&self) {
        if let Some(callback) = self.imp().refresh_callback.borrow().as_ref() {
            callback();
        }
    }

    pub fn set_like_callback<F: Fn(Post, glib::WeakRef<PostRow>) + 'static>(&self, callback: F) {
        self.imp().like_callback.replace(Some(Box::new(callback)));
    }

    pub fn set_repost_callback<F: Fn(Post, glib::WeakRef<PostRow>) + 'static>(&self, callback: F) {
        self.imp().repost_callback.replace(Some(Box::new(callback)));
    }

    pub fn set_quote_callback<F: Fn(Post) + 'static>(&self, callback: F) {
        self.imp().quote_callback.replace(Some(Box::new(callback)));
    }

    pub fn set_reply_callback<F: Fn(Post) + 'static>(&self, callback: F) {
        self.imp().reply_callback.replace(Some(Box::new(callback)));
    }

    /// Args: subject DID, the page's follow-URI cell, the button.
    pub fn set_follow_callback<F>(&self, callback: F)
    where
        F: Fn(String, Rc<RefCell<Option<String>>>, glib::WeakRef<gtk4::Button>) + 'static,
    {
        self.imp().follow_callback.replace(Some(Box::new(callback)));
    }

    /// Args: the profile to message and the button that asked.
    pub fn set_message_callback<F>(&self, callback: F)
    where
        F: Fn(Profile, glib::WeakRef<gtk4::Button>) + 'static,
    {
        self.imp()
            .message_callback
            .replace(Some(Box::new(callback)));
    }

    /// The own page's feed context, for the app to point at the signed-in
    /// DID and refresh.
    pub fn own_profile_feed_ctx(&self) -> Option<Rc<ProfileFeedCtx>> {
        self.imp().own_profile_feed_ctx.borrow().clone()
    }

    pub fn set_edit_profile_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .edit_profile_callback
            .replace(Some(Box::new(callback)));
    }

    /// The profile the own page currently shows.
    pub fn current_profile(&self) -> Option<Profile> {
        self.imp().current_profile.borrow().clone()
    }

    /// Args: the page's feed context and whether this is a first page.
    pub fn set_profile_tab_callback<F>(&self, callback: F)
    where
        F: Fn(Rc<ProfileFeedCtx>, bool) + 'static,
    {
        self.imp()
            .profile_tab_callback
            .replace(Some(Box::new(callback)));
    }

    /// Posts, Replies, Media, Videos. Switching clears the list and asks
    /// the app for the tab's own feed; the shared context strands stale
    /// fetches. Centered to sit under the centered profile header.
    fn build_profile_tabs(&self, feed_ctx: &Rc<ProfileFeedCtx>) -> gtk4::Box {
        let tabs = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        tabs.add_css_class("linked");
        tabs.set_halign(gtk4::Align::Center);
        tabs.set_margin_top(12);
        tabs.set_margin_bottom(8);
        let mut first_tab: Option<gtk4::ToggleButton> = None;
        for (label, filter) in [
            ("Posts", "posts_and_author_threads"),
            ("Replies", "posts_with_replies"),
            ("Media", "posts_with_media"),
            ("Videos", "posts_with_video"),
        ] {
            let tab = gtk4::ToggleButton::with_label(label);
            match &first_tab {
                Some(first) => tab.set_group(Some(first)),
                None => {
                    tab.set_active(true);
                    first_tab = Some(tab.clone());
                }
            }
            let win = self.downgrade();
            let ctx = feed_ctx.clone();
            tab.connect_toggled(move |tab| {
                if !tab.is_active() || ctx.filter.get() == filter {
                    return;
                }
                ctx.filter.set(filter);
                ctx.begin_refresh();
                if let Some(win) = win.upgrade()
                    && let Some(cb) = win.imp().profile_tab_callback.borrow().as_ref()
                {
                    cb(ctx.clone(), true);
                }
            });
            tabs.append(&tab);
        }
        tabs
    }

    /// "No posts yet", following the model so every tab can say when it
    /// has nothing.
    fn build_feed_empty_label(model: &gio::ListStore, initially_visible: bool) -> gtk4::Label {
        let none = gtk4::Label::new(Some("No posts yet"));
        none.add_css_class("dim-label");
        none.set_margin_top(12);
        none.set_margin_bottom(12);
        none.set_visible(initially_visible);
        let none_weak = none.downgrade();
        model.connect_items_changed(move |model, _, _, _| {
            if let Some(none) = none_weak.upgrade() {
                none.set_visible(model.n_items() == 0);
            }
        });
        none
    }

    /// Near the scroller's bottom, ask for the current tab's next page.
    fn wire_feed_pagination(&self, scrolled: &gtk4::ScrolledWindow, feed_ctx: &Rc<ProfileFeedCtx>) {
        let win = self.downgrade();
        let ctx = feed_ctx.clone();
        scrolled.vadjustment().connect_value_changed(move |adj| {
            let near_bottom = adj.value() >= adj.upper() - adj.page_size() - 400.0;
            if near_bottom
                && ctx.cursor.borrow().is_some()
                && !ctx.fetching.get()
                && let Some(win) = win.upgrade()
                && let Some(cb) = win.imp().profile_tab_callback.borrow().as_ref()
            {
                cb(ctx.clone(), false);
            }
        });
    }

    /// One StatusPage, hidden until its list turns out empty.
    fn empty_state_page(icon: &str, title: &str, description: &str) -> adw::StatusPage {
        let page = adw::StatusPage::new();
        page.set_icon_name(Some(icon));
        page.set_title(title);
        page.set_description(Some(description));
        page.set_vexpand(true);
        page.set_visible(false);
        page
    }

    /// Show the list or its empty state, whichever holds the truth.
    fn apply_empty_state(
        overlay: &RefCell<Option<gtk4::Overlay>>,
        empty_state: &RefCell<Option<adw::StatusPage>>,
        empty: bool,
    ) {
        if let Some(overlay) = overlay.borrow().as_ref() {
            overlay.set_visible(!empty);
        }
        if let Some(page) = empty_state.borrow().as_ref() {
            page.set_visible(empty);
        }
    }

    /// Six rows of shapes standing in for content while a first load is
    /// out. Purely decorative; the accessibility layer sees nothing.
    fn skeleton_list() -> gtk4::Box {
        let column = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        column.set_visible(false);
        for _ in 0..6 {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
            row.set_margin_start(16);
            row.set_margin_end(16);
            row.set_margin_top(14);
            row.set_margin_bottom(14);

            let avatar = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            avatar.set_size_request(40, 40);
            avatar.set_valign(gtk4::Align::Start);
            avatar.add_css_class("skeleton-shape");
            avatar.add_css_class("skeleton-circle");
            row.append(&avatar);

            let lines = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            lines.set_hexpand(true);
            let wide = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            wide.set_size_request(-1, 12);
            wide.add_css_class("skeleton-shape");
            lines.append(&wide);
            let narrow = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            narrow.set_size_request(-1, 12);
            narrow.set_margin_end(120);
            narrow.add_css_class("skeleton-shape");
            lines.append(&narrow);
            row.append(&lines);

            column.append(&row);
        }
        column
    }

    /// "Couldn't load" with a Try Again button running `retry`. Hidden
    /// until a first load fails.
    fn error_state_page(retry: impl Fn() + 'static) -> adw::StatusPage {
        let page = adw::StatusPage::new();
        page.set_icon_name(Some("dialog-warning-symbolic"));
        page.set_title("Couldn't Load");
        page.set_description(Some("Check your connection and try again."));
        page.set_vexpand(true);
        page.set_visible(false);

        let retry_button = gtk4::Button::with_label("Try Again");
        retry_button.add_css_class("pill");
        retry_button.add_css_class("suggested-action");
        retry_button.set_halign(gtk4::Align::Center);
        retry_button.connect_clicked(move |_| retry());
        page.set_child(Some(&retry_button));
        page
    }

    /// A first load failed. With content on screen the toast said enough;
    /// with nothing listed the error page and its Try Again take over.
    fn apply_load_failed(
        model_empty: bool,
        overlay: &RefCell<Option<gtk4::Overlay>>,
        empty_state: &RefCell<Option<adw::StatusPage>>,
        error_state: &RefCell<Option<adw::StatusPage>>,
        skeleton: &RefCell<Option<gtk4::Box>>,
    ) {
        if let Some(skeleton) = skeleton.borrow().as_ref() {
            skeleton.set_visible(false);
        }
        if !model_empty {
            return;
        }
        if let Some(overlay) = overlay.borrow().as_ref() {
            overlay.set_visible(false);
        }
        if let Some(page) = empty_state.borrow().as_ref() {
            page.set_visible(false);
        }
        if let Some(page) = error_state.borrow().as_ref() {
            page.set_visible(true);
        }
    }

    /// Point a Follow button at the given state: label, style, and back on.
    pub fn sync_follow_button(button: &gtk4::Button, following: bool) {
        button.set_label(if following { "Following" } else { "Follow" });
        if following {
            button.remove_css_class("suggested-action");
        } else {
            button.add_css_class("suggested-action");
        }
        button.set_sensitive(true);
    }

    pub fn set_compose_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .compose_callback
            .replace(Some(Box::new(callback)));
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            let win = self.downgrade();
            sidebar.connect_compose_clicked(move || {
                let Some(win) = win.upgrade() else {
                    return;
                };
                if let Some(cb) = win.imp().compose_callback.borrow().as_ref() {
                    cb();
                }
            });
        }
    }

    /// Drop replies from a feed batch when the user has asked for a quieter
    /// timeline.
    ///
    /// Main feed only. Threads, profiles and likes keep their replies.
    fn filter_feed_posts(posts: Vec<Post>) -> Vec<Post> {
        if !crate::state::AppSettings::load().hide_replies_in_feed {
            return posts;
        }
        posts
            .into_iter()
            .filter(|p| p.reply_context.is_none())
            .collect()
    }

    /// First timeline load without a cache: skeleton rows instead of a
    /// blank pane.
    pub fn set_timeline_loading(&self, loading: bool) {
        let imp = self.imp();
        let empty = imp
            .timeline_model
            .borrow()
            .as_ref()
            .is_none_or(|m| m.n_items() == 0);
        if let Some(skeleton) = imp.timeline_skeleton.borrow().as_ref() {
            skeleton.set_visible(loading && empty);
        }
        if loading && empty {
            if let Some(overlay) = imp.timeline_overlay.borrow().as_ref() {
                overlay.set_visible(false);
            }
        }
    }

    pub fn set_posts(&self, posts: Vec<Post>) {
        let posts = Self::filter_feed_posts(posts);
        let empty = posts.is_empty();
        if let Some(model) = self.imp().timeline_model.borrow().as_ref() {
            model.remove_all();

            for post in posts {
                let post_object = PostObject::new(post);
                model.append(&post_object);
            }
        }
        let imp = self.imp();
        Self::apply_empty_state(&imp.timeline_overlay, &imp.timeline_empty_state, empty);
        if let Some(skeleton) = imp.timeline_skeleton.borrow().as_ref() {
            skeleton.set_visible(false);
        }
        if let Some(err) = imp.timeline_error_state.borrow().as_ref() {
            err.set_visible(false);
        }
    }

    pub fn append_posts(&self, posts: Vec<Post>) {
        let posts = Self::filter_feed_posts(posts);
        if let Some(model) = self.imp().timeline_model.borrow().as_ref() {
            for post in posts {
                let post_object = PostObject::new(post);
                model.append(&post_object);
            }
        }
    }

    pub fn set_user_avatar(&self, display_name: &str, avatar_url: Option<&str>) {
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            sidebar.set_user_avatar(display_name, avatar_url);
        }
    }

    /// Set the current user's DID (used to filter out self in conversations)
    pub fn set_current_user_did(&self, did: &str) {
        self.imp().current_user_did.replace(Some(did.to_string()));
    }

    pub fn set_loading_more(&self, loading: bool) {
        if let Some(spinner) = self.imp().loading_spinner.borrow().as_ref() {
            spinner.set_visible(loading);
            spinner.set_spinning(loading);
        }
    }

    pub fn set_new_posts_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .new_posts_callback
            .replace(Some(Box::new(callback)));
    }

    pub fn show_new_posts_banner(&self, count: usize) {
        // At the top there is nothing to announce: the posts just appeared
        // in view. A banner here had nowhere to go when clicked.
        if self.timeline_at_top() {
            return;
        }

        // Each poll reports only its own batch, so keep a running total
        // until the user actually looks at the top of the feed.
        let total = self.imp().unseen_posts.get().saturating_add(count);
        self.imp().unseen_posts.set(total);
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            sidebar.set_badge(
                crate::ui::sidebar::NavItem::Home,
                u32::try_from(total).unwrap_or(u32::MAX),
            );
        }

        if let Some(banner) = self.imp().new_posts_banner.borrow().as_ref() {
            let label_text = if total == 1 {
                "1 new post".to_string()
            } else if total > 99 {
                "99+ new posts".to_string()
            } else {
                format!("{} new posts", total)
            };
            // Find the label inside the button's box
            if let Some(banner_box) = banner.child().and_then(|c| c.downcast::<gtk4::Box>().ok()) {
                if let Some(label) = banner_box
                    .last_child()
                    .and_then(|c| c.downcast::<gtk4::Label>().ok())
                {
                    label.set_label(&label_text);
                }
            }
            banner.set_visible(true);
            // Animate fade in (skip if animations disabled)
            let animations_enabled = gtk4::Settings::default()
                .map(|s| s.is_gtk_enable_animations())
                .unwrap_or(true);
            if animations_enabled {
                let target = adw::PropertyAnimationTarget::new(banner, "opacity");
                let animation = adw::TimedAnimation::new(banner, 0.0, 1.0, 200, target);
                animation.play();
            } else {
                banner.set_opacity(1.0);
            }
            // A live region: the banner is easy to miss without sight of it.
            self.announce(&label_text, gtk4::AccessibleAnnouncementPriority::Medium);
        }
    }

    pub fn hide_new_posts_banner(&self) {
        // Scrolling near the top calls this every frame; only touch the
        // badge when there was something to clear.
        if self.imp().unseen_posts.get() != 0 {
            self.imp().unseen_posts.set(0);
            if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
                sidebar.set_badge(crate::ui::sidebar::NavItem::Home, 0);
            }
        }

        if let Some(banner) = self.imp().new_posts_banner.borrow().as_ref() {
            // Animate fade out then hide (skip animation if disabled)
            let animations_enabled = gtk4::Settings::default()
                .map(|s| s.is_gtk_enable_animations())
                .unwrap_or(true);
            if animations_enabled {
                let target = adw::PropertyAnimationTarget::new(banner, "opacity");
                let animation = adw::TimedAnimation::new(banner, 1.0, 0.0, 150, target);
                animation.connect_done(glib::clone!(
                    #[weak]
                    banner,
                    move |_| {
                        banner.set_visible(false);
                    }
                ));
                animation.play();
            } else {
                banner.set_visible(false);
            }
        }
    }

    pub fn scroll_to_top(&self) {
        if let Some(scrolled) = self.imp().scrolled_window.borrow().as_ref() {
            let adj = scrolled.vadjustment();
            adj.set_value(0.0);

            // Schedule another scroll after GTK processes
            let scrolled_clone = scrolled.clone();
            glib::idle_add_local_once(move || {
                scrolled_clone.vadjustment().set_value(0.0);
            });
        }
    }

    pub fn is_at_top(&self) -> bool {
        if let Some(scrolled) = self.imp().scrolled_window.borrow().as_ref() {
            let adj = scrolled.vadjustment();
            adj.value() < 50.0 // Consider "at top" if within 50 pixels
        } else {
            true
        }
    }

    pub fn prepend_posts(&self, posts: Vec<Post>) {
        if let Some(model) = self.imp().timeline_model.borrow().as_ref() {
            // Collect existing posts first
            let existing_count = model.n_items();
            let mut all_posts: Vec<Post> = posts;
            for i in 0..existing_count {
                if let Some(obj) = model.item(i) {
                    if let Ok(post_obj) = obj.downcast::<PostObject>() {
                        if let Some(post) = post_obj.post() {
                            all_posts.push(post);
                        }
                    }
                }
            }

            // Clear and rebuild to reset the ListView state
            model.remove_all();
            for post in all_posts {
                let post_object = PostObject::new(post);
                model.append(&post_object);
            }
        }
    }

    /// Insert new posts at the top of the timeline without disrupting the current view.
    /// Rebuilds the model with new posts prepended, then restores scroll position.
    pub fn insert_posts_at_top(&self, posts: Vec<Post>) {
        let posts = Self::filter_feed_posts(posts);
        let Some(model) = self.imp().timeline_model.borrow().as_ref().cloned() else {
            return;
        };
        if self.imp().timeline_list_view.borrow().is_none() {
            return;
        }
        let Some(scrolled) = self.imp().scrolled_window.borrow().as_ref().cloned() else {
            return;
        };

        let new_count = posts.len();
        if new_count == 0 {
            return;
        }

        // Capture scroll state BEFORE modifying the model
        let adj = scrolled.vadjustment();
        let current_scroll = adj.value();
        let old_upper = adj.upper();
        // At the top the new posts should simply appear in view; keeping
        // the old position would shove the reader down past them.
        let at_top = current_scroll < 50.0;

        // Collect all existing posts
        let existing_count = model.n_items();
        let mut all_posts: Vec<Post> = posts;
        for i in 0..existing_count {
            if let Some(obj) = model.item(i) {
                if let Ok(post_obj) = obj.downcast::<PostObject>() {
                    if let Some(post) = post_obj.post() {
                        all_posts.push(post);
                    }
                }
            }
        }

        // Clear and rebuild model
        model.remove_all();
        for post in all_posts {
            let post_object = PostObject::new(post);
            model.append(&post_object);
        }

        // Restore scroll position after GTK recalculates layout.
        // The new content adds height at the top, so we add that difference
        // to the current scroll to keep viewing the same content.
        glib::idle_add_local_once(move || {
            if at_top {
                return;
            }
            let new_upper = adj.upper();
            let height_added = new_upper - old_upper;

            if height_added > 0.0 {
                // Add the height of new posts to maintain position
                let new_scroll = current_scroll + height_added;
                adj.set_value(new_scroll);
            }
        });
    }

    /// Whether the timeline is scrolled to (or near) the top, where new
    /// posts land straight in view.
    fn timeline_at_top(&self) -> bool {
        self.imp()
            .scrolled_window
            .borrow()
            .as_ref()
            .is_some_and(|scrolled| scrolled.vadjustment().value() < 50.0)
    }

    /// Refresh all visible post timestamps
    pub fn refresh_timestamps(&self) {
        if let Some(list_view) = self.imp().timeline_list_view.borrow().as_ref() {
            let mut child = list_view.first_child();
            while let Some(widget) = child {
                if let Some(post_row) = widget.downcast_ref::<PostRow>() {
                    post_row.refresh_timestamp();
                }
                child = widget.next_sibling();
            }
        }
    }

    /// Set the callback for when the user selects a different feed
    pub fn set_feed_changed_callback<F: Fn(SavedFeed) + 'static>(&self, callback: F) {
        self.imp()
            .feed_changed_callback
            .replace(Some(Box::new(callback)));
    }

    /// Update the list of available feeds in the popover
    pub fn set_saved_feeds(&self, feeds: Vec<SavedFeed>) {
        self.imp().saved_feeds.replace(feeds.clone());
        self.rebuild_feed_list();
    }

    /// Rebuild the feed menu; the action state picks out the current one.
    fn rebuild_feed_list(&self) {
        let Some(menu) = self.imp().feed_menu.borrow().clone() else {
            return;
        };
        menu.remove_all();
        for feed in self.imp().saved_feeds.borrow().iter() {
            let item = gio::MenuItem::new(Some(&feed.display_name), None);
            item.set_action_and_target_value(Some("feeds.select"), Some(&feed.uri.to_variant()));
            menu.append_item(&item);
        }
    }

    /// Update the feed selector button label and selection state
    pub fn set_current_feed_name(&self, name: &str, uri: &str) {
        if let Some(label) = self.imp().feed_btn_label.borrow().as_ref() {
            label.set_text(name);
        }
        self.imp().current_feed_uri.replace(uri.to_string());
        if let Some(action) = self.imp().feed_action.borrow().as_ref() {
            action.set_state(&uri.to_variant());
        }
    }

    /// Set callback for when a post is clicked (to open thread view)
    pub fn set_post_clicked_callback<F: Fn(Post) + 'static>(&self, callback: F) {
        self.imp()
            .post_clicked_callback
            .replace(Some(Box::new(callback)));
    }

    /// Set callback for when a profile is clicked (to open profile view)
    pub fn set_profile_clicked_callback<F: Fn(Profile) + 'static>(&self, callback: F) {
        self.imp()
            .profile_clicked_callback
            .replace(Some(Box::new(callback)));
    }

    /// Set callback for when an @mention in post text is clicked (handle without @)
    pub fn set_mention_clicked_callback<F: Fn(String) + 'static>(&self, callback: F) {
        self.imp()
            .mention_clicked_callback
            .replace(Some(Box::new(callback)));
    }

    /// Set callback for when a followers or following count is clicked
    pub fn set_follow_list_clicked_callback<F: Fn(Profile, FollowListKind) + 'static>(
        &self,
        callback: F,
    ) {
        self.imp()
            .follow_list_clicked_callback
            .replace(Some(Box::new(callback)));
    }

    /// Push a thread view page onto the current section's navigation stack
    pub fn push_thread_page(&self, post: &Post, thread_posts: Vec<Post>) {
        let nav_view = self.current_nav_view();
        let Some(nav_view) = nav_view else {
            return;
        };

        // Save current scroll position before navigating
        self.save_scroll_position();

        // Tag pages by what they show so revisiting one returns to it instead of
        // stacking a second copy. Suppressing the click on the focused row was
        // not enough: the same post also renders as a parent row inside a
        // reply's thread, and that row is an ordinary link.
        let tag = format!("thread:{}", post.uri);
        if nav_view.find_page(&tag).is_some() {
            nav_view.pop_to_tag(&tag);
            return;
        }

        let page = self.build_thread_page(post, thread_posts);
        page.set_tag(Some(&tag));
        nav_view.push(&page);
    }

    /// Push a profile view page onto the current section's navigation stack
    pub fn push_profile_page(
        &self,
        profile: &Profile,
        posts: Vec<Post>,
        feed_cursor: Option<String>,
    ) {
        let nav_view = self.current_nav_view();
        let Some(nav_view) = nav_view else {
            return;
        };

        // Save current scroll position before navigating
        self.save_scroll_position();

        // Same treatment as threads. build_profile_page tagged every profile
        // "profile", so opening a second one put two identically tagged pages
        // in the stack, which AdwNavigationView does not allow.
        let tag = format!("profile:{}", profile.did);
        if nav_view.find_page(&tag).is_some() {
            nav_view.pop_to_tag(&tag);
            return;
        }

        let page = self.build_profile_page(profile, posts, feed_cursor);
        page.set_tag(Some(&tag));
        nav_view.push(&page);
    }

    /// Push a followers or following list onto the current section's stack.
    ///
    /// Returns the list widget either way; the caller decides whether a
    /// popped-back page still needs a fetch.
    pub fn push_follow_list_page(
        &self,
        profile: &Profile,
        kind: FollowListKind,
    ) -> Option<FollowListPush> {
        let nav_view = self.current_nav_view()?;

        self.save_scroll_position();

        // Same treatment as threads and profiles: revisiting pops back
        // instead of stacking a second copy.
        let tag = format!("{}:{}", kind.tag_prefix(), profile.did);
        if let Some(existing) = nav_view.find_page(&tag) {
            nav_view.pop_to_tag(&tag);
            // The list is the content box's last child; see the push below.
            let list = existing
                .child()
                .and_then(|content| content.last_child())
                .and_downcast::<FollowListPage>()?;
            return Some(FollowListPush::PoppedBack(list));
        }

        let list = FollowListPage::new(kind);

        // Tapping a listed account opens their profile through the same
        // route as everywhere else, so cycles dedupe on the profile tag.
        let win = self.downgrade();
        list.set_profile_activated_callback(move |profile| {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(cb) = win.imp().profile_clicked_callback.borrow().as_ref() {
                cb(profile);
            }
        });

        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let display_name = profile.display_name.as_deref().unwrap_or(&profile.handle);
        let title_text = format!("{} · {}", kind.title(), display_name);
        let title = gtk4::Label::new(Some(&title_text));
        title.add_css_class("title");
        // A bare label's minimum width is its full text; a long display name
        // would push the window past the screen.
        title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        header.set_title_widget(Some(&title));

        let window_controls = gtk4::WindowControls::new(gtk4::PackType::End);
        header.pack_end(&window_controls);

        content_box.append(&header);
        content_box.append(&list);

        let page = adw::NavigationPage::new(&content_box, &title_text);
        page.set_tag(Some(&tag));
        nav_view.push(&page);

        Some(FollowListPush::Pushed(list))
    }

    /// Save the current scroll position for the active section
    fn save_scroll_position(&self) {
        let imp = self.imp();
        if let Some(scrolled) = imp.scrolled_window.borrow().as_ref() {
            let pos = scrolled.vadjustment().value();
            imp.saved_scroll_position.replace(pos);
        }
    }

    /// Pop to the root page of the current section (used for back navigation within a section)
    pub fn pop_to_root(&self) {
        let stack = self.imp().main_stack.borrow();
        let Some(stack) = stack.as_ref() else { return };
        let Some(visible_name) = stack.visible_child_name() else {
            return;
        };

        // Pop to the root tag for the current section
        let root_tag = match visible_name.as_str() {
            "home" => "timeline",
            "mentions" => "mentions",
            "activity" => "activity",
            "chat" => "chat",
            "profile" => "own-profile",
            "likes" => "likes",
            "bookmarks" => "bookmarks",
            "search" => "search",
            _ => return,
        };

        if let Some(nav_view) = self.current_nav_view() {
            nav_view.pop_to_tag(root_tag);
        }
    }

    /// Get the NavigationView backing a top-level stack page
    fn nav_view_named(&self, page_name: &str) -> Option<adw::NavigationView> {
        match page_name {
            "home" => self.imp().home_nav_view.borrow().clone(),
            "mentions" => self.imp().mentions_nav_view.borrow().clone(),
            "activity" => self.imp().activity_nav_view.borrow().clone(),
            "chat" => self.imp().chat_nav_view.borrow().clone(),
            "profile" => self.imp().profile_nav_view.borrow().clone(),
            "likes" => self.imp().likes_nav_view.borrow().clone(),
            "bookmarks" => self.imp().bookmarks_nav_view.borrow().clone(),
            "search" => self.imp().search_nav_view.borrow().clone(),
            _ => self.imp().home_nav_view.borrow().clone(),
        }
    }

    /// Tag of the root NavigationPage for a top-level stack page
    fn root_tag_for(page_name: &str) -> Option<&'static str> {
        match page_name {
            "home" => Some("timeline"),
            "mentions" => Some("mentions"),
            "activity" => Some("activity"),
            "chat" => Some("chat"),
            "profile" => Some("own-profile"),
            "likes" => Some("likes"),
            "bookmarks" => Some("bookmarks"),
            "search" => Some("search"),
            // Settings has no NavigationView of its own, and `nav_view_named`
            // falls back to Home's, so a tag here would unwind Home instead.
            _ => None,
        }
    }

    /// Get the NavigationView for the currently visible stack page
    fn current_nav_view(&self) -> Option<adw::NavigationView> {
        let stack = self.imp().main_stack.borrow();
        let stack = stack.as_ref()?;
        let visible_name = stack.visible_child_name()?;
        self.nav_view_named(visible_name.as_str())
    }

    /// Show a page without touching its navigation stack (entering and leaving
    /// settings).
    fn show_page(&self, page_name: &str) {
        if let Some(stack) = self.imp().main_stack.borrow().as_ref() {
            stack.set_visible_child_name(page_name);
        }
    }

    /// Switch to a top-level page (Home, Mentions, etc.) with no animation.
    ///
    /// Each section keeps its own NavigationView, so selecting the section
    /// again has to unwind it. Without that, switching the GtkStack is a no-op
    /// and the drilled-in page stays on screen.
    pub fn switch_to_page(&self, page_name: &str) {
        if let (Some(nav_view), Some(root_tag)) = (
            self.nav_view_named(page_name),
            Self::root_tag_for(page_name),
        ) {
            nav_view.pop_to_tag(root_tag);
        }

        self.show_page(page_name);

        // Keyboard users arrive with their focus, not just their eyes. The
        // idle lets the page settle before the grab walks into it.
        if let Some(nav_view) = self.nav_view_named(page_name) {
            glib::idle_add_local_once(move || {
                if let Some(page) = nav_view.visible_page() {
                    page.grab_focus();
                }
            });
        }
    }

    /// Focus follows a pop: the page underneath takes the keyboard back
    /// instead of leaving focus on a widget that just left the tree.
    fn focus_after_pop(nav_view: &adw::NavigationView) {
        nav_view.connect_popped(|view, _| {
            if let Some(page) = view.visible_page() {
                page.grab_focus();
            }
        });
    }

    /// Build a thread view page
    fn build_thread_page(&self, main_post: &Post, posts: Vec<Post>) -> adw::NavigationPage {
        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        // Hide the default title buttons; explicit window controls come later
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let title = gtk4::Label::new(Some("Thread"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));

        // Add window controls (close button) to the end
        let window_controls = gtk4::WindowControls::new(gtk4::PackType::End);
        header.pack_end(&window_controls);

        content_box.append(&header);

        // Split posts into: parents (before main), main post, replies (after main)
        let main_uri = &main_post.uri;
        let main_post_idx = posts.iter().position(|p| &p.uri == main_uri);

        let (parent_posts, main_and_replies): (Vec<_>, Vec<_>) = match main_post_idx {
            Some(idx) => (posts[..idx].to_vec(), posts[idx..].to_vec()),
            None => (vec![], posts),
        };

        let (main_post_vec, reply_posts): (Vec<_>, Vec<_>) = if main_and_replies.is_empty() {
            (vec![main_post.clone()], vec![])
        } else {
            (
                vec![main_and_replies[0].clone()],
                main_and_replies.into_iter().skip(1).collect(),
            )
        };

        let the_main_post = main_post_vec
            .first()
            .cloned()
            .unwrap_or_else(|| main_post.clone());

        // Scrollable content in a single Box. Multiple ListViews had height
        // calculation issues with nested scrolling.
        let scroll_content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        scroll_content.add_css_class("background");

        // Helper to create a PostRow with all callbacks wired up
        // `clickable` controls whether clicking the post opens its thread
        let create_post_row = |win: &Self, post: Post| -> PostRow {
            let post_row = PostRow::new();
            post_row.bind(&post);

            // Like callback
            let post_for_like = post.clone();
            let w = win.downgrade();
            post_row.connect_like_clicked(move |row, was_liked, like_uri| {
                let Some(w) = w.upgrade() else {
                    return;
                };
                let mut post_with_state = post_for_like.clone();
                post_with_state.viewer_like = if was_liked { like_uri } else { None };
                let row_weak = row.downgrade();
                w.imp()
                    .like_callback
                    .borrow()
                    .as_ref()
                    .map(|cb| cb(post_with_state, row_weak));
            });

            // Repost callback
            let post_for_repost = post.clone();
            let w = win.downgrade();
            post_row.connect_repost_clicked(move |row, was_reposted, repost_uri| {
                let Some(w) = w.upgrade() else {
                    return;
                };
                let mut post_with_state = post_for_repost.clone();
                post_with_state.viewer_repost = if was_reposted { repost_uri } else { None };
                let row_weak = row.downgrade();
                w.imp()
                    .repost_callback
                    .borrow()
                    .as_ref()
                    .map(|cb| cb(post_with_state, row_weak));
            });

            // Quote callback
            let post_for_quote = post.clone();
            let w = win.downgrade();
            post_row.connect_quote_clicked(move || {
                let Some(w) = w.upgrade() else {
                    return;
                };
                w.imp()
                    .quote_callback
                    .borrow()
                    .as_ref()
                    .map(|cb| cb(post_for_quote.clone()));
            });

            // Reply callback
            let post_clone = post.clone();
            let w = win.downgrade();
            post_row.connect_reply_clicked(move || {
                let Some(w) = w.upgrade() else {
                    return;
                };
                w.imp()
                    .reply_callback
                    .borrow()
                    .as_ref()
                    .map(|cb| cb(post_clone.clone()));
            });

            // The post click callback is always set so embedded quotes can navigate.
            // When !clickable (main post in thread), the row body click is
            // disabled via set_not_clickable(), but quote cards still fire this.
            {
                let w = win.downgrade();
                post_row.set_post_clicked_callback(move |p| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    if let Some(cb) = w.imp().post_clicked_callback.borrow().as_ref() {
                        cb(p);
                    }
                });
            }

            let w = win.downgrade();
            post_row.set_profile_clicked_callback(move |profile| {
                let Some(w) = w.upgrade() else {
                    return;
                };
                if let Some(cb) = w.imp().profile_clicked_callback.borrow().as_ref() {
                    cb(profile);
                }
            });

            let w = win.downgrade();
            post_row.set_mention_clicked_callback(move |handle| {
                let Some(w) = w.upgrade() else {
                    return;
                };
                if let Some(cb) = w.imp().mention_clicked_callback.borrow().as_ref() {
                    cb(handle);
                }
            });

            post_row
        };

        // Parent posts, when present. Clicking one navigates to it.
        for post in parent_posts {
            let post_row = create_post_row(self, post);
            scroll_content.append(&post_row);
        }

        // The main post. Body click is disabled since we are already viewing it.
        let main_row = create_post_row(self, the_main_post.clone());
        main_row.set_not_clickable();
        scroll_content.append(&main_row);

        // Add "Posted {date}" separator
        let posted_separator = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        posted_separator.add_css_class("thread-separator");

        // Format the date: "Posted Sat, Jan 31, 2026 at 12:50 PM"
        let posted_text = Self::format_full_timestamp(&the_main_post.indexed_at);
        let posted_label = gtk4::Label::new(Some(&posted_text));
        posted_label.add_css_class("dim-label");
        posted_label.set_hexpand(true);
        posted_label.set_halign(gtk4::Align::Center);
        posted_separator.append(&posted_label);

        scroll_content.append(&posted_separator);

        // Add "Replies" section if there are replies
        if !reply_posts.is_empty() {
            let replies_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            replies_header.set_margin_start(16);
            replies_header.set_margin_end(16);
            replies_header.set_margin_top(16);
            replies_header.set_margin_bottom(8);

            let replies_label = gtk4::Label::new(Some("Replies"));
            replies_label.add_css_class("title-4");
            replies_label.set_halign(gtk4::Align::Start);
            replies_header.append(&replies_label);

            scroll_content.append(&replies_header);

            for post in reply_posts {
                let post_row = create_post_row(self, post);
                scroll_content.append(&post_row);
            }
        }

        // Add bottom padding to ensure last post isn't clipped
        let bottom_spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        bottom_spacer.set_height_request(24);
        scroll_content.append(&bottom_spacer);

        // Plain AdwClamp, where the feeds use AdwClampScrollable.
        //
        // `scroll_content` is a hand-built GtkBox of PostRows, so there is
        // nothing to virtualize and no GtkScrollable to forward to.
        // AdwClampScrollable would bind its adjustments onto a GtkBox that has
        // no such properties and the page would stop scrolling: measured
        // `upper` 0 against 16,720. Virtualizing a thread means converting this
        // box to a list first.
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&scroll_content));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        // No horizontal scrolling; content wraps or clips within the clamp
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&clamp));
        content_box.append(&scrolled);

        // No tag. Each thread is unique and several may sit in the stack.
        adw::NavigationPage::new(&content_box, "Thread")
    }

    /// Format a timestamp as "Posted Sat, Jan 31, 2026 at 12:50 PM" in local timezone
    fn format_full_timestamp(indexed_at: &str) -> String {
        if indexed_at.is_empty() {
            return "Posted".to_string();
        }

        let Ok(post_time) = chrono::DateTime::parse_from_rfc3339(indexed_at) else {
            return "Posted".to_string();
        };

        // Convert to local timezone
        let local_time: chrono::DateTime<chrono::Local> = post_time.into();

        // Format: "Posted Sat, Jan 31, 2026 at 12:50 PM"
        format!(
            "Posted {}",
            local_time
                .format("%a, %b %d, %Y at %l:%M %p")
                .to_string()
                .trim()
        )
    }

    /// Build a profile view page
    fn build_profile_page(
        &self,
        profile: &Profile,
        posts: Vec<Post>,
        feed_cursor: Option<String>,
    ) -> adw::NavigationPage {
        let no_posts = posts.is_empty();
        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let display_name = profile.display_name.as_deref().unwrap_or(&profile.handle);
        let title = gtk4::Label::new(Some(display_name));
        title.add_css_class("title");
        // A bare label's minimum width is its full text; a long display name
        // would push the window past the screen.
        title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        header.set_title_widget(Some(&title));

        // Add window controls (close button) to the end
        let window_controls = gtk4::WindowControls::new(gtk4::PackType::End);
        header.pack_end(&window_controls);

        content_box.append(&header);

        // Everything above the posts scrolls with them; see the list below.
        let header_block = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Banner strip. A tinted strip stands in when the profile has none,
        // so the avatar always has an edge to straddle.
        let banner_frame = gtk4::Frame::new(None);
        banner_frame.set_overflow(gtk4::Overflow::Hidden);
        banner_frame.add_css_class("profile-banner");
        banner_frame.set_hexpand(true);
        banner_frame.set_size_request(-1, 140);
        if let Some(banner_url) = &profile.banner {
            let banner_picture = gtk4::Picture::new();
            banner_picture.set_hexpand(true);
            banner_picture.set_vexpand(true);
            banner_picture.set_can_shrink(true);
            banner_picture.set_content_fit(gtk4::ContentFit::Cover);
            banner_frame.set_child(Some(&banner_picture));
            crate::ui::avatar_cache::load_image_into_picture(banner_picture, banner_url.clone());
        }

        // Text stays off the banner; only the avatar overlaps it, so there
        // is no contrast problem to scrim away.
        let profile_header = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        profile_header.set_margin_start(16);
        profile_header.set_margin_end(16);
        // Room for the avatar's lower half hanging in from the banner.
        profile_header.set_margin_top(52);
        profile_header.set_margin_bottom(16);

        let header_column = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        header_column.append(&banner_frame);
        header_column.append(&profile_header);

        let header_overlay = gtk4::Overlay::new();
        header_overlay.set_child(Some(&header_column));

        let avatar = adw::Avatar::new(80, Some(display_name), true);
        if let Some(avatar_url) = &profile.avatar {
            crate::ui::avatar_cache::load_avatar(avatar.clone(), avatar_url.clone());
        }
        avatar.set_halign(gtk4::Align::Center);
        avatar.set_valign(gtk4::Align::Start);
        avatar.set_margin_top(96);
        avatar.add_css_class("profile-avatar");
        header_overlay.add_overlay(&avatar);

        header_block.append(&header_overlay);

        let name_label = gtk4::Label::new(Some(display_name));
        name_label.add_css_class("title-1");
        name_label.set_halign(gtk4::Align::Center);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        profile_header.append(&name_label);

        let handle_label = gtk4::Label::new(Some(&format!("@{}", profile.handle)));
        handle_label.add_css_class("dim-label");
        handle_label.set_halign(gtk4::Align::Center);
        handle_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        profile_header.append(&handle_label);

        if profile.viewer_followed_by.is_some() {
            let follows_you = gtk4::Label::new(Some("Follows you"));
            follows_you.add_css_class("dim-label");
            follows_you.add_css_class("caption");
            follows_you.set_halign(gtk4::Align::Center);
            profile_header.append(&follows_you);
        }

        // Follow/unfollow and Message, hidden on your own page. The follow
        // record URI lives in a cell shared with the app; each click hands
        // its button over and goes insensitive until the result comes back.
        let own_page = self.imp().current_user_did.borrow().as_deref() == Some(&profile.did);
        // Shared with the condensed bar's copy of the button, so both read
        // the same record URI.
        let follow_uri = Rc::new(RefCell::new(profile.viewer_following.clone()));
        if !own_page {
            let buttons_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            buttons_row.set_halign(gtk4::Align::Center);

            let follow_btn = gtk4::Button::new();
            follow_btn.add_css_class("pill");
            Self::sync_follow_button(&follow_btn, follow_uri.borrow().is_some());

            let win = self.downgrade();
            let did = profile.did.clone();
            let uri_cell = follow_uri.clone();
            follow_btn.connect_clicked(move |btn| {
                let Some(win) = win.upgrade() else {
                    return;
                };
                if let Some(cb) = win.imp().follow_callback.borrow().as_ref() {
                    // Show the hoped-for state, locked until the server answers.
                    let following = uri_cell.borrow().is_some();
                    HangarWindow::sync_follow_button(btn, !following);
                    btn.set_sensitive(false);
                    cb(did.clone(), uri_cell.clone(), btn.downgrade());
                }
            });
            buttons_row.append(&follow_btn);

            let message_btn = gtk4::Button::with_label("Message");
            message_btn.add_css_class("pill");
            let win = self.downgrade();
            let profile_for_message = profile.clone();
            message_btn.connect_clicked(move |btn| {
                let Some(win) = win.upgrade() else {
                    return;
                };
                if let Some(cb) = win.imp().message_callback.borrow().as_ref() {
                    // Locked while the availability check is out.
                    btn.set_sensitive(false);
                    cb(profile_for_message.clone(), btn.downgrade());
                }
            });
            buttons_row.append(&message_btn);

            profile_header.append(&buttons_row);
        }

        // Followers and following, clickable. A count can be missing when
        // the full profile fetch failed; the lists open fine without it.
        let stats_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
        stats_row.set_halign(gtk4::Align::Center);
        for (kind, count) in [
            (FollowListKind::Followers, profile.followers_count),
            (FollowListKind::Following, profile.following_count),
        ] {
            stats_row.append(&self.build_follow_stat(profile, kind, count));
        }
        profile_header.append(&stats_row);

        // Bio, with links. Wire text: it wraps and caps its line length,
        // there is no horizontal scrollbar to save it.
        if let Some(bio) = profile.description.as_deref().filter(|b| !b.is_empty()) {
            let bio_label = gtk4::Label::new(None);
            bio_label.set_wrap(true);
            bio_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
            bio_label.set_max_width_chars(60);
            bio_label.set_justify(gtk4::Justification::Center);
            bio_label.set_use_markup(true);
            crate::ui::rich_text::set_linkified(&bio_label, bio);
            self.connect_bio_links(&bio_label);
            profile_header.append(&bio_label);
        }

        // The profile header itself is already in the tree through the
        // banner overlay; only the separator and the tabs follow it.
        header_block.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        // The header scrolls with the posts: row 0 of the list is a marker
        // the factory answers with the header widget. Same mechanism as the
        // own profile page; see `build_own_profile_content`.
        let model = gio::ListStore::new::<PostObject>();

        let feed_ctx = Rc::new(ProfileFeedCtx {
            did: RefCell::new(profile.did.clone()),
            filter: Cell::new("posts_and_author_threads"),
            cursor: RefCell::new(feed_cursor),
            fetching: Cell::new(false),
            generation: Cell::new(0),
            model: model.clone(),
        });

        header_block.append(&self.build_profile_tabs(&feed_ctx));
        header_block.append(&Self::build_feed_empty_label(&model, no_posts));
        let header_marker = gio::ListStore::new::<gtk4::StringObject>();
        header_marker.append(&gtk4::StringObject::new("profile-header"));
        let sections = gio::ListStore::new::<gio::ListStore>();
        sections.append(&header_marker);
        sections.append(&model);
        let flattened = gtk4::FlattenListModel::new(Some(sections));

        // The header parks here whenever no slot holds it, so it stays
        // findable and dies with the page.
        let parking = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        parking.set_visible(false);
        parking.append(&header_block);
        content_box.append(&parking);

        let factory = gtk4::SignalListItemFactory::new();

        // Each slot can be either row; see `build_own_profile_content` for
        // why this is a box with a hidden host and not a stack.
        factory.connect_setup(|_, item| {
            let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() else {
                return;
            };
            let slot = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            let header_host = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            header_host.set_visible(false);
            let post_row = PostRow::new();
            slot.append(&header_host);
            slot.append(&post_row);
            list_item.set_child(Some(&slot));
        });
        Self::release_video_on_unbind(&factory);

        let header_for_unbind = header_block.clone();
        let parking_for_unbind = parking.clone();
        factory.connect_unbind(move |_, item| {
            let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() else {
                return;
            };
            let Some(host) = header_for_unbind.parent().and_downcast::<gtk4::Box>() else {
                return;
            };
            if host.parent() != list_item.child() {
                return;
            }
            host.remove(&header_for_unbind);
            host.set_visible(false);
            parking_for_unbind.append(&header_for_unbind);
        });

        let win = self.downgrade();
        let header_for_bind = header_block.clone();
        factory.connect_bind(move |_, item| {
            let Some(win) = win.upgrade() else {
                return;
            };
            let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() else {
                return;
            };
            let slot = list_item.child().and_downcast::<gtk4::Box>();
            let host = slot
                .as_ref()
                .and_then(|slot| slot.first_child())
                .and_downcast::<gtk4::Box>();

            // Row 0: the header takes this slot.
            if list_item
                .item()
                .and_downcast::<gtk4::StringObject>()
                .is_some()
            {
                if let Some(host) = host.as_ref() {
                    if let Some(previous) = header_for_bind.parent().and_downcast::<gtk4::Box>() {
                        previous.remove(&header_for_bind);
                        previous.set_visible(false);
                    }
                    host.append(&header_for_bind);
                    host.set_visible(true);
                    if let Some(post_row) = host.next_sibling() {
                        post_row.set_visible(false);
                    }
                }
                return;
            }

            // An ordinary post. Reclaim the header if this slot held it.
            if let Some(host) = host.as_ref() {
                if header_for_bind.parent().as_ref() == Some(host.upcast_ref::<gtk4::Widget>()) {
                    host.remove(&header_for_bind);
                }
                host.set_visible(false);
            }

            if let Some(post_object) = list_item.item().and_downcast::<PostObject>()
                && let Some(post) = post_object.post()
                && let Some(post_row) = host
                    .as_ref()
                    .and_then(|host| host.next_sibling())
                    .and_downcast::<PostRow>()
            {
                post_row.set_visible(true);
                post_row.bind(&post);
                post_row.set_list_position(list_item.position());
                // Wire up like/repost/reply/quote callbacks
                let post_for_like = post.clone();
                let w = win.downgrade();
                post_row.connect_like_clicked(move |row, was_liked, like_uri| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    let mut post_with_state = post_for_like.clone();
                    post_with_state.viewer_like = if was_liked { like_uri } else { None };
                    let row_weak = row.downgrade();
                    w.imp()
                        .like_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_with_state, row_weak));
                });
                let post_for_repost = post.clone();
                let w = win.downgrade();
                post_row.connect_repost_clicked(move |row, was_reposted, repost_uri| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    let mut post_with_state = post_for_repost.clone();
                    post_with_state.viewer_repost = if was_reposted { repost_uri } else { None };
                    let row_weak = row.downgrade();
                    w.imp()
                        .repost_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_with_state, row_weak));
                });
                let post_for_quote = post.clone();
                let w = win.downgrade();
                post_row.connect_quote_clicked(move || {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .quote_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_for_quote.clone()));
                });
                let post_clone = post.clone();
                let w = win.downgrade();
                post_row.connect_reply_clicked(move || {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .reply_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_clone.clone()));
                });
            }
        });

        // Add posts to model
        for post in posts {
            model.append(&PostObject::new(post));
        }

        let selection = gtk4::NoSelection::new(Some(flattened));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.add_css_class("background");

        // Content width, and the scrollability the list needs to virtualize.
        // See `build_timeline`. The list has to be this widget's direct child
        // and this widget the scroller's; a box or overlay between them breaks
        // scrolling.
        let clamp = adw::ClampScrollable::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&clamp));

        // A condensed bar keeps who and the actions in reach once the full
        // header has scrolled away.
        let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        bar.add_css_class("condensed-profile-bar");

        let bar_avatar = adw::Avatar::new(24, Some(display_name), true);
        if let Some(avatar_url) = &profile.avatar {
            crate::ui::avatar_cache::load_avatar(bar_avatar.clone(), avatar_url.clone());
        }
        bar.append(&bar_avatar);

        let bar_name = gtk4::Label::new(Some(display_name));
        bar_name.add_css_class("heading");
        bar_name.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        bar.append(&bar_name);

        let bar_spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        bar_spacer.set_hexpand(true);
        bar.append(&bar_spacer);

        let bar_follow = gtk4::Button::new();
        if !own_page {
            bar_follow.add_css_class("pill");
            Self::sync_follow_button(&bar_follow, follow_uri.borrow().is_some());
            let win = self.downgrade();
            let did = profile.did.clone();
            let uri_cell = follow_uri.clone();
            bar_follow.connect_clicked(move |btn| {
                let Some(win) = win.upgrade() else {
                    return;
                };
                if let Some(cb) = win.imp().follow_callback.borrow().as_ref() {
                    let following = uri_cell.borrow().is_some();
                    HangarWindow::sync_follow_button(btn, !following);
                    btn.set_sensitive(false);
                    cb(did.clone(), uri_cell.clone(), btn.downgrade());
                }
            });
            bar.append(&bar_follow);

            let bar_message = gtk4::Button::with_label("Message");
            bar_message.add_css_class("pill");
            let win = self.downgrade();
            let profile_for_message = profile.clone();
            bar_message.connect_clicked(move |btn| {
                let Some(win) = win.upgrade() else {
                    return;
                };
                if let Some(cb) = win.imp().message_callback.borrow().as_ref() {
                    btn.set_sensitive(false);
                    cb(profile_for_message.clone(), btn.downgrade());
                }
            });
            bar.append(&bar_message);
        }

        let revealer = gtk4::Revealer::new();
        revealer.set_transition_type(gtk4::RevealerTransitionType::SlideDown);
        revealer.set_valign(gtk4::Align::Start);
        revealer.set_child(Some(&bar));

        self.wire_feed_pagination(&scrolled, &feed_ctx);

        // Reveal once the full header is out of view. Re-sync the bar's
        // follow button on each toggle; the header's copy may have settled
        // a different state while the bar was hidden.
        let revealer_weak = revealer.downgrade();
        let bar_follow_weak = bar_follow.downgrade();
        let uri_cell = follow_uri.clone();
        let bar_own_page = own_page;
        scrolled.vadjustment().connect_value_changed(move |adj| {
            let Some(revealer) = revealer_weak.upgrade() else {
                return;
            };
            let past_header = adj.value() > 240.0;
            if revealer.reveals_child() != past_header {
                revealer.set_reveal_child(past_header);
                if past_header && !bar_own_page {
                    if let Some(btn) = bar_follow_weak.upgrade() {
                        if btn.is_sensitive() {
                            HangarWindow::sync_follow_button(&btn, uri_cell.borrow().is_some());
                        }
                    }
                }
            }
        });

        let list_overlay = gtk4::Overlay::new();
        list_overlay.set_vexpand(true);
        list_overlay.set_child(Some(&scrolled));
        list_overlay.add_overlay(&revealer);

        content_box.append(&list_overlay);

        // The tag is set by push_profile_page, which derives it from the DID so
        // two different profiles cannot collide.
        adw::NavigationPage::new(&content_box, display_name)
    }

    /// Build the mentions page
    fn build_mentions_page(&self) -> adw::NavigationPage {
        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let title = gtk4::Label::new(Some("Mentions"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));

        // Add window controls (close, minimize, maximize) to the end
        let window_controls = gtk4::WindowControls::new(gtk4::PackType::End);
        header.pack_end(&window_controls);

        content_box.append(&header);
        content_box.append(&self.build_mentions_list());

        let page = adw::NavigationPage::new(&content_box, "Mentions");
        page.set_tag(Some("mentions"));
        page
    }

    /// Build the mentions list widget
    fn build_mentions_list(&self) -> gtk4::Box {
        let mentions_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        mentions_box.set_vexpand(true);

        let overlay = gtk4::Overlay::new();
        overlay.set_vexpand(true);

        let model = gio::ListStore::new::<NotificationObject>();
        let factory = gtk4::SignalListItemFactory::new();

        factory.connect_setup(|_, item| {
            let row = MentionRow::new();
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() {
                list_item.set_child(Some(&row));
            }
        });

        let win = self.downgrade();
        factory.connect_bind(move |_, item| {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(notif_object) = list_item.item().and_downcast::<NotificationObject>()
                && let Some(notif) = notif_object.notification()
                && let Some(row) = list_item.child().and_downcast::<MentionRow>()
            {
                row.bind(&notif);
                // Connect profile click
                let profile = notif.author.clone();
                let w = win.downgrade();
                row.connect_profile_clicked(move |_| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .profile_clicked_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(profile.clone()));
                });
                // Connect post click, and clear it when there is no post: this
                // row is recycled, so leaving the handler alone would keep
                // whatever the previous notification put there.
                match notif.post.clone() {
                    Some(post) => {
                        let w = win.downgrade();
                        row.connect_clicked(move |_| {
                            let Some(w) = w.upgrade() else {
                                return;
                            };
                            w.imp()
                                .post_clicked_callback
                                .borrow()
                                .as_ref()
                                .map(|cb| cb(post.clone()));
                        });
                    }
                    None => row.clear_clicked(),
                }
            }
        });

        let selection = gtk4::NoSelection::new(Some(model.clone()));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.add_css_class("background");

        // Content width, and the scrollability the list needs to virtualize.
        // See `build_timeline`. The list has to be this widget's direct child
        // and this widget the scroller's; a box or overlay between them breaks
        // scrolling.
        let clamp = adw::ClampScrollable::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&clamp));
        overlay.set_child(Some(&scrolled));

        // Loading spinner
        let spinner = gtk4::Spinner::new();
        spinner.update_property(&[gtk4::accessible::Property::Label("Loading")]);
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        mentions_box.append(&overlay);

        let empty_state = Self::empty_state_page(
            "system-users-symbolic",
            "No mentions yet",
            "Replies, mentions, and quotes of you will appear here.",
        );
        mentions_box.append(&empty_state);
        let win = self.downgrade();
        let error_state = Self::error_state_page(move || {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(cb) = win.imp().nav_changed_callback.borrow().as_ref() {
                cb(crate::ui::sidebar::NavItem::Mentions);
            }
        });
        mentions_box.append(&error_state);
        self.imp().mentions_error_state.replace(Some(error_state));

        let skeleton = Self::skeleton_list();
        mentions_box.append(&skeleton);
        self.imp().mentions_skeleton.replace(Some(skeleton));

        self.imp().mentions_overlay.replace(Some(overlay.clone()));
        self.imp().mentions_empty_state.replace(Some(empty_state));

        let imp = self.imp();
        imp.mentions_model.replace(Some(model));
        imp.mentions_scrolled_window.replace(Some(scrolled.clone()));
        imp.mentions_spinner.replace(Some(spinner));

        // Infinite scroll
        let adj = scrolled.vadjustment();
        adj.connect_value_changed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |adj| {
                let value = adj.value();
                let upper = adj.upper();
                let page_size = adj.page_size();
                if value >= upper - page_size - 200.0
                    && let Some(cb) = win.imp().mentions_load_more_callback.borrow().as_ref()
                {
                    cb();
                }
            }
        ));

        mentions_box
    }

    /// Show the mentions page (top-level navigation, instant switch)
    pub fn show_mentions_page(&self) {
        self.switch_to_page("mentions");
    }

    /// Show the home/timeline page (top-level navigation, instant switch)
    pub fn show_home_page(&self) {
        self.switch_to_page("home");
    }

    /// Set notifications/mentions in the mentions list
    pub fn set_mentions(&self, notifications: Vec<Notification>) {
        let empty = notifications.is_empty();
        if let Some(model) = self.imp().mentions_model.borrow().as_ref() {
            model.remove_all();
            for notif in notifications {
                model.append(&NotificationObject::new(notif));
            }
        }
        let imp = self.imp();
        Self::apply_empty_state(&imp.mentions_overlay, &imp.mentions_empty_state, empty);
        if let Some(skeleton) = imp.mentions_skeleton.borrow().as_ref() {
            skeleton.set_visible(false);
        }
        if let Some(err) = imp.mentions_error_state.borrow().as_ref() {
            err.set_visible(false);
        }
    }

    /// Append more notifications to the mentions list
    pub fn append_mentions(&self, notifications: Vec<Notification>) {
        if let Some(model) = self.imp().mentions_model.borrow().as_ref() {
            for notif in notifications {
                model.append(&NotificationObject::new(notif));
            }
        }
    }

    /// Set loading state for mentions
    pub fn set_mentions_loading(&self, loading: bool) {
        let imp = self.imp();
        let empty = imp
            .mentions_model
            .borrow()
            .as_ref()
            .is_none_or(|m| m.n_items() == 0);
        // A first load gets skeleton rows; later pages keep the corner
        // spinner under the content they extend.
        if let Some(skeleton) = imp.mentions_skeleton.borrow().as_ref() {
            skeleton.set_visible(loading && empty);
        }
        if loading && empty {
            if let Some(overlay) = imp.mentions_overlay.borrow().as_ref() {
                overlay.set_visible(false);
            }
        }
        if let Some(spinner) = imp.mentions_spinner.borrow().as_ref() {
            spinner.set_visible(loading && !empty);
            spinner.set_spinning(loading && !empty);
        }
    }

    /// Set callback for loading more mentions
    pub fn set_mentions_load_more_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .mentions_load_more_callback
            .replace(Some(Box::new(callback)));
    }

    /// Build the activity page
    fn build_activity_page(&self) -> adw::NavigationPage {
        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let title = gtk4::Label::new(Some("Activity"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));

        // Add window controls (close, minimize, maximize) to the end
        let window_controls = gtk4::WindowControls::new(gtk4::PackType::End);
        header.pack_end(&window_controls);

        content_box.append(&header);
        content_box.append(&self.build_activity_list());

        let page = adw::NavigationPage::new(&content_box, "Activity");
        page.set_tag(Some("activity"));
        page
    }

    /// Build the activity list widget
    fn build_activity_list(&self) -> gtk4::Box {
        let activity_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        activity_box.set_vexpand(true);

        let overlay = gtk4::Overlay::new();
        overlay.set_vexpand(true);

        let model = gio::ListStore::new::<NotificationObject>();
        let factory = gtk4::SignalListItemFactory::new();

        factory.connect_setup(|_, item| {
            let row = ActivityRow::new();
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() {
                list_item.set_child(Some(&row));
            }
        });

        let win = self.downgrade();
        factory.connect_bind(move |_, item| {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(notif_object) = list_item.item().and_downcast::<NotificationObject>()
                && let Some(notif) = notif_object.notification()
                && let Some(row) = list_item.child().and_downcast::<ActivityRow>()
            {
                row.bind(&notif);
                // Connect profile click
                let profile = notif.author.clone();
                let w = win.downgrade();
                row.connect_profile_clicked(move |_| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .profile_clicked_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(profile.clone()));
                });
                // Connect post click, clearing it when there is none: a follow
                // notification bound into a recycled row would otherwise
                // inherit the previous occupant's post and open its thread.
                match notif.post.clone() {
                    Some(post) => {
                        let w = win.downgrade();
                        row.connect_clicked(move |_| {
                            let Some(w) = w.upgrade() else {
                                return;
                            };
                            w.imp()
                                .post_clicked_callback
                                .borrow()
                                .as_ref()
                                .map(|cb| cb(post.clone()));
                        });
                    }
                    None => row.clear_clicked(),
                }
            }
        });

        let selection = gtk4::NoSelection::new(Some(model.clone()));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.add_css_class("background");

        // Content width, and the scrollability the list needs to virtualize.
        // See `build_timeline`. The list has to be this widget's direct child
        // and this widget the scroller's; a box or overlay between them breaks
        // scrolling.
        let clamp = adw::ClampScrollable::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&clamp));
        overlay.set_child(Some(&scrolled));

        // Loading spinner
        let spinner = gtk4::Spinner::new();
        spinner.update_property(&[gtk4::accessible::Property::Label("Loading")]);
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        activity_box.append(&overlay);

        let empty_state = Self::empty_state_page(
            "preferences-system-notifications-symbolic",
            "No activity yet",
            "Likes, reposts, and new followers will appear here.",
        );
        activity_box.append(&empty_state);
        let win = self.downgrade();
        let error_state = Self::error_state_page(move || {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(cb) = win.imp().nav_changed_callback.borrow().as_ref() {
                cb(crate::ui::sidebar::NavItem::Activity);
            }
        });
        activity_box.append(&error_state);
        self.imp().activity_error_state.replace(Some(error_state));

        let skeleton = Self::skeleton_list();
        activity_box.append(&skeleton);
        self.imp().activity_skeleton.replace(Some(skeleton));

        self.imp().activity_overlay.replace(Some(overlay.clone()));
        self.imp().activity_empty_state.replace(Some(empty_state));

        let imp = self.imp();
        imp.activity_model.replace(Some(model));
        imp.activity_scrolled_window.replace(Some(scrolled.clone()));
        imp.activity_spinner.replace(Some(spinner));

        // Infinite scroll
        let adj = scrolled.vadjustment();
        adj.connect_value_changed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |adj| {
                let value = adj.value();
                let upper = adj.upper();
                let page_size = adj.page_size();
                if value >= upper - page_size - 200.0
                    && let Some(cb) = win.imp().activity_load_more_callback.borrow().as_ref()
                {
                    cb();
                }
            }
        ));

        activity_box
    }

    /// Show the activity page (top-level navigation, instant switch)
    pub fn show_activity_page(&self) {
        self.switch_to_page("activity");
    }

    /// Set notifications in the activity list
    pub fn set_activity(&self, notifications: Vec<Notification>) {
        let empty = notifications.is_empty();
        if let Some(model) = self.imp().activity_model.borrow().as_ref() {
            model.remove_all();
            for notif in notifications {
                model.append(&NotificationObject::new(notif));
            }
        }
        let imp = self.imp();
        Self::apply_empty_state(&imp.activity_overlay, &imp.activity_empty_state, empty);
        if let Some(skeleton) = imp.activity_skeleton.borrow().as_ref() {
            skeleton.set_visible(false);
        }
        if let Some(err) = imp.activity_error_state.borrow().as_ref() {
            err.set_visible(false);
        }
    }

    /// Append more notifications to the activity list
    pub fn append_activity(&self, notifications: Vec<Notification>) {
        if let Some(model) = self.imp().activity_model.borrow().as_ref() {
            for notif in notifications {
                model.append(&NotificationObject::new(notif));
            }
        }
    }

    /// Set loading state for activity
    pub fn set_activity_loading(&self, loading: bool) {
        let imp = self.imp();
        let empty = imp
            .activity_model
            .borrow()
            .as_ref()
            .is_none_or(|m| m.n_items() == 0);
        // A first load gets skeleton rows; later pages keep the corner
        // spinner under the content they extend.
        if let Some(skeleton) = imp.activity_skeleton.borrow().as_ref() {
            skeleton.set_visible(loading && empty);
        }
        if loading && empty {
            if let Some(overlay) = imp.activity_overlay.borrow().as_ref() {
                overlay.set_visible(false);
            }
        }
        if let Some(spinner) = imp.activity_spinner.borrow().as_ref() {
            spinner.set_visible(loading && !empty);
            spinner.set_spinning(loading && !empty);
        }
    }

    /// Set callback for loading more activity
    pub fn set_activity_load_more_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .activity_load_more_callback
            .replace(Some(Box::new(callback)));
    }

    /// Build the chat page
    fn build_chat_page(&self) -> adw::NavigationPage {
        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let title = gtk4::Label::new(Some("Messages"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));

        // Start a conversation without hunting down a profile first.
        let new_message_btn = gtk4::Button::from_icon_name("list-add-symbolic");
        new_message_btn.add_css_class("flat");
        new_message_btn.set_tooltip_text(Some("New Message"));
        new_message_btn.update_property(&[gtk4::accessible::Property::Label("New Message")]);
        let win = self.downgrade();
        new_message_btn.connect_clicked(move |_| {
            if let Some(win) = win.upgrade() {
                win.present_new_message_dialog();
            }
        });
        header.pack_start(&new_message_btn);

        // Add window controls (close, minimize, maximize) to the end
        let window_controls = gtk4::WindowControls::new(gtk4::PackType::End);
        header.pack_end(&window_controls);

        content_box.append(&header);
        content_box.append(&self.build_chat_list());

        let page = adw::NavigationPage::new(&content_box, "Messages");
        page.set_tag(Some("chat"));
        page
    }

    /// Build the chat conversation list widget
    fn build_chat_list(&self) -> gtk4::Box {
        let chat_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        chat_box.set_vexpand(true);

        let overlay = gtk4::Overlay::new();
        overlay.set_vexpand(true);

        let model = gio::ListStore::new::<ConversationObject>();
        let factory = gtk4::SignalListItemFactory::new();

        factory.connect_setup(|_, item| {
            let row = ConversationRow::new();
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() {
                list_item.set_child(Some(&row));
            }
        });

        let win = self.downgrade();
        factory.connect_bind(move |_, item| {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(convo_object) = list_item.item().and_downcast::<ConversationObject>()
                && let Some(convo) = convo_object.conversation()
                && let Some(row) = list_item.child().and_downcast::<ConversationRow>()
            {
                let my_did = win.imp().current_user_did.borrow();
                row.bind(&convo, my_did.as_deref());
                // Connect click
                let conversation = convo.clone();
                let w = win.downgrade();
                row.connect_clicked(move |_| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .conversation_clicked_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(conversation.clone()));
                });
            }
        });

        let selection = gtk4::NoSelection::new(Some(model.clone()));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.add_css_class("background");

        // Content width, and the scrollability the list needs to virtualize.
        // See `build_timeline`. The list has to be this widget's direct child
        // and this widget the scroller's; a box or overlay between them breaks
        // scrolling.
        let clamp = adw::ClampScrollable::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&clamp));
        overlay.set_child(Some(&scrolled));

        // Loading spinner
        let spinner = gtk4::Spinner::new();
        spinner.update_property(&[gtk4::accessible::Property::Label("Loading")]);
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        // Empty state (shown instead of the overlay when there are no conversations)
        let empty_state = adw::StatusPage::new();
        empty_state.set_icon_name(Some("chat-message-new-symbolic"));
        empty_state.set_title("No messages yet");
        empty_state.set_description(Some("Your direct messages will appear here."));
        empty_state.set_vexpand(true);
        empty_state.set_visible(false);

        chat_box.append(&overlay);
        chat_box.append(&empty_state);
        let win = self.downgrade();
        let error_state = Self::error_state_page(move || {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(cb) = win.imp().nav_changed_callback.borrow().as_ref() {
                cb(crate::ui::sidebar::NavItem::Chat);
            }
        });
        chat_box.append(&error_state);
        self.imp().chat_error_state.replace(Some(error_state));

        let skeleton = Self::skeleton_list();
        chat_box.append(&skeleton);
        self.imp().chat_skeleton.replace(Some(skeleton));

        let imp = self.imp();
        imp.chat_model.replace(Some(model));
        imp.chat_scrolled_window.replace(Some(scrolled.clone()));
        imp.chat_spinner.replace(Some(spinner));
        imp.chat_overlay.replace(Some(overlay));
        imp.chat_empty_state.replace(Some(empty_state));

        // Infinite scroll
        let adj = scrolled.vadjustment();
        let win = self.downgrade();
        adj.connect_value_changed(move |adj| {
            let Some(win) = win.upgrade() else {
                return;
            };
            let value = adj.value();
            let upper = adj.upper();
            let page_size = adj.page_size();
            if value >= upper - page_size - 200.0 {
                if let Some(cb) = win.imp().chat_load_more_callback.borrow().as_ref() {
                    cb();
                }
            }
        });

        chat_box
    }

    /// Show the chat page (top-level navigation, instant switch)
    pub fn show_chat_page(&self) {
        self.switch_to_page("chat");
    }

    /// Set conversations in the chat list
    pub fn set_conversations(&self, conversations: Vec<Conversation>) {
        let empty = conversations.is_empty();
        if let Some(model) = self.imp().chat_model.borrow().as_ref() {
            model.remove_all();
            for convo in conversations {
                model.append(&ConversationObject::new(convo));
            }
        }
        if let Some(overlay) = self.imp().chat_overlay.borrow().as_ref() {
            overlay.set_visible(!empty);
        }
        if let Some(empty_state) = self.imp().chat_empty_state.borrow().as_ref() {
            empty_state.set_visible(empty);
        }
        if let Some(err) = self.imp().chat_error_state.borrow().as_ref() {
            err.set_visible(false);
        }
        if let Some(skeleton) = self.imp().chat_skeleton.borrow().as_ref() {
            skeleton.set_visible(false);
        }
    }

    /// Append more conversations to the chat list
    pub fn append_conversations(&self, conversations: Vec<Conversation>) {
        if let Some(model) = self.imp().chat_model.borrow().as_ref() {
            for convo in conversations {
                model.append(&ConversationObject::new(convo));
            }
        }
    }

    /// Set loading state for chat
    pub fn set_chat_loading(&self, loading: bool) {
        let imp = self.imp();
        let empty = imp
            .chat_model
            .borrow()
            .as_ref()
            .is_none_or(|m| m.n_items() == 0);
        // A first load gets skeleton rows; later pages keep the corner
        // spinner under the content they extend.
        if let Some(skeleton) = imp.chat_skeleton.borrow().as_ref() {
            skeleton.set_visible(loading && empty);
        }
        if loading && empty {
            if let Some(overlay) = imp.chat_overlay.borrow().as_ref() {
                overlay.set_visible(false);
            }
        }
        if let Some(spinner) = imp.chat_spinner.borrow().as_ref() {
            spinner.set_visible(loading && !empty);
            spinner.set_spinning(loading && !empty);
        }
    }

    /// Set callback for loading more conversations
    pub fn set_chat_load_more_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .chat_load_more_callback
            .replace(Some(Box::new(callback)));
    }

    /// Set callback for when a conversation is clicked
    pub fn set_conversation_clicked_callback<F: Fn(Conversation) + 'static>(&self, callback: F) {
        self.imp()
            .conversation_clicked_callback
            .replace(Some(Box::new(callback)));
    }

    /// Push a conversation's message view onto the chat stack.
    ///
    /// A conversation already in the stack is popped back to instead of
    /// stacked twice, so a double click cannot open two copies.
    pub fn push_message_page(&self, conversation: &Conversation) -> Option<MessagePush> {
        let nav_view = self.imp().chat_nav_view.borrow().clone()?;

        let tag = format!("convo:{}", conversation.id);
        if let Some(existing) = nav_view.find_page(&tag) {
            nav_view.pop_to_tag(&tag);
            // The message page is the content box's last child; see the
            // push below.
            let page = existing
                .child()
                .and_then(|content| content.last_child())
                .and_downcast::<MessagePage>()?;
            return Some(MessagePush::PoppedBack(page));
        }

        let my_did = self.imp().current_user_did.borrow().clone();
        let page = MessagePage::new(conversation, my_did.as_deref());

        // Mentions in messages route like mentions everywhere else.
        let win = self.downgrade();
        page.set_mention_clicked_callback(move |handle| {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(cb) = win.imp().mention_clicked_callback.borrow().as_ref() {
                cb(handle);
            }
        });

        // Shared posts open their thread, on the chat stack.
        let win = self.downgrade();
        page.set_post_clicked_callback(move |post| {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(cb) = win.imp().post_clicked_callback.borrow().as_ref() {
                cb(post);
            }
        });

        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let title_text = page.conversation_title();
        let title = gtk4::Label::new(Some(&title_text));
        title.add_css_class("title");
        // A bare label's minimum width is its full text; a long display
        // name would push the window past the screen.
        title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        header.set_title_widget(Some(&title));

        let window_controls = gtk4::WindowControls::new(gtk4::PackType::End);
        header.pack_end(&window_controls);

        content_box.append(&header);
        content_box.append(&page);

        let nav_page = adw::NavigationPage::new(&content_box, &title_text);
        nav_page.set_tag(Some(&tag));
        nav_view.push(&nav_page);

        Some(MessagePush::Pushed(page))
    }

    /// Drop the unread badge on one conversation row after a read went
    /// through on the server.
    pub fn set_conversation_read(&self, convo_id: &str) {
        let model = self.imp().chat_model.borrow().clone();
        let Some(model) = model else {
            return;
        };
        for i in 0..model.n_items() {
            let Some(object) = model.item(i).and_downcast::<ConversationObject>() else {
                continue;
            };
            let Some(mut convo) = object.conversation() else {
                continue;
            };
            if convo.id != convo_id || convo.unread_count == 0 {
                continue;
            }
            convo.unread_count = 0;
            // Replacing the item is what makes the bound row repaint.
            model.splice(i, 1, &[ConversationObject::new(convo)]);
            break;
        }
    }

    /// Set callback for nav item changes
    pub fn set_nav_changed_callback<F: Fn(crate::ui::sidebar::NavItem) + 'static>(
        &self,
        callback: F,
    ) {
        self.imp()
            .nav_changed_callback
            .replace(Some(Box::new(callback)));
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            let win = self.downgrade();
            sidebar.connect_nav_changed(move |item| {
                let Some(win) = win.upgrade() else {
                    return;
                };
                win.imp()
                    .nav_changed_callback
                    .borrow()
                    .as_ref()
                    .map(|cb| cb(item));
            });
        }
    }

    /// Get sidebar reference
    pub fn sidebar(&self) -> Option<crate::ui::sidebar::Sidebar> {
        self.imp().sidebar.borrow().clone()
    }

    /// Set callback for when My Profile is clicked in the avatar popover
    pub fn set_my_profile_clicked_callback<F: Fn() + 'static>(&self, f: F) {
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            sidebar.connect_my_profile_clicked(f);
        }
    }

    /// Move the rail's highlight without dispatching a section switch;
    /// programmatic selection does not emit `row-activated`.
    pub fn select_nav(&self, item: crate::ui::sidebar::NavItem) {
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            sidebar.select_nav_item(item);
        }
    }

    /// Set callback for when Settings is clicked in the avatar popover
    pub fn set_settings_clicked_callback<F: Fn() + 'static>(&self, f: F) {
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            sidebar.connect_settings_clicked(f);
        }
    }

    /// Set callback for when About is clicked in the avatar popover
    pub fn set_about_clicked_callback<F: Fn() + 'static>(&self, f: F) {
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            sidebar.connect_about_clicked(f);
        }
    }

    /// Set callback for when Sign Out is clicked in the avatar popover
    pub fn set_sign_out_clicked_callback<F: Fn() + 'static>(&self, f: F) {
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            sidebar.connect_sign_out_clicked(f);
        }
    }

    /// Build the own profile page (for Profile tab in sidebar)
    fn build_own_profile_page(&self) -> adw::NavigationPage {
        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let title = gtk4::Label::new(Some("Profile"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));

        // Add window controls (close, minimize, maximize) to the end
        let window_controls = gtk4::WindowControls::new(gtk4::PackType::End);
        header.pack_end(&window_controls);

        content_box.append(&header);
        content_box.append(&self.build_own_profile_content());

        let page = adw::NavigationPage::new(&content_box, "Profile");
        page.set_tag(Some("own-profile"));
        page
    }

    /// Build the own profile content (header + tabs + posts)
    ///
    /// The banner/avatar/bio block still scrolls away with the posts, but it is
    /// row 0 of the list now instead of a sibling above it.
    ///
    /// The old shape was scroller, box, overlay, list. Two non-scrollable
    /// widgets in the way, so the list was handed its full natural height and
    /// stopped realizing rows a couple of hundred posts in. Nothing can sit
    /// between the scroller and the list, so the spinner overlay moved outside
    /// the scroller (matching Mentions, Activity, Likes and Search) and the
    /// header became a row.
    ///
    /// `GtkListView::set_header_factory` was measured and rejected. A section
    /// header stays pinned to the top of the viewport and drags the first post
    /// with it, which spends most of the page on content already scrolled past.
    fn build_own_profile_content(&self) -> gtk4::Box {
        let profile_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        profile_box.set_vexpand(true);

        // The spinner overlay wraps the scroller now. An overlay between the
        // scroller and the list breaks scrolling.
        let overlay = gtk4::Overlay::new();
        overlay.set_vexpand(true);

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

        // Everything above the posts, built once and kept. The imp references
        // the profile-load path writes into (banner, avatar, counts) point into
        // this tree, so it cannot be rebuilt per bind. The factory moves this
        // one instance between slots.
        let header_block = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Profile header section (banner, avatar, info, stats)
        let header_section = self.build_profile_header_section();
        header_block.append(&header_section);

        // Separator
        let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        header_block.append(&sep);

        // Posts list. A GtkFlattenListModel over two stores: a one-item store
        // for the header, then the posts. `profile_page_model` stays the posts
        // store, so `set_profile_posts` and `append_profile_posts` are
        // unchanged and cannot clear the header.
        let model = gio::ListStore::new::<PostObject>();

        // The same tabs and context as the drill-down pages. The DID is
        // empty until sign-in; `fetch_profile_posts` fills it.
        let feed_ctx = Rc::new(ProfileFeedCtx {
            did: RefCell::new(String::new()),
            filter: Cell::new("posts_and_author_threads"),
            cursor: RefCell::new(None),
            fetching: Cell::new(false),
            generation: Cell::new(0),
            model: model.clone(),
        });
        header_block.append(&self.build_profile_tabs(&feed_ctx));
        header_block.append(&Self::build_feed_empty_label(&model, true));
        self.imp()
            .own_profile_feed_ctx
            .replace(Some(feed_ctx.clone()));
        let header_marker = gio::ListStore::new::<gtk4::StringObject>();
        header_marker.append(&gtk4::StringObject::new("profile-header"));
        let sections = gio::ListStore::new::<gio::ListStore>();
        sections.append(&header_marker);
        sections.append(&model);
        let flattened = gtk4::FlattenListModel::new(Some(sections));

        let factory = gtk4::SignalListItemFactory::new();

        // Each slot can be either row. Both children are built in setup and
        // shown or hidden in bind. A hidden child adds nothing to a GtkBox's
        // size, so a post row is never padded out to the header's height. A
        // GtkStack is homogeneous by default and would do exactly that.
        factory.connect_setup(|_, item| {
            let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() else {
                return;
            };
            let slot = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            let header_host = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            header_host.set_visible(false);
            let post_row = PostRow::new();
            slot.append(&header_host);
            slot.append(&post_row);
            list_item.set_child(Some(&slot));
        });
        Self::release_video_on_unbind(&factory);

        // Take the header back out of the slot that held it, so the slot can
        // be recycled into a post row. Without this the header stays parented
        // to a slot GTK has already reused.
        //
        // A second `connect_unbind`; `release_video_on_unbind` is shared by
        // every feed and this is only this page's business. Both handlers run.
        let header_for_unbind = header_block.clone();
        factory.connect_unbind(move |_, item| {
            let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() else {
                return;
            };
            let Some(host) = header_for_unbind.parent().and_downcast::<gtk4::Box>() else {
                return;
            };
            // Only if it is *this* slot that is holding it.
            if host.parent() != list_item.child() {
                return;
            }
            host.remove(&header_for_unbind);
            host.set_visible(false);
        });

        let win = self.downgrade();
        let header_for_bind = header_block.clone();
        factory.connect_bind(move |_, item| {
            let Some(win) = win.upgrade() else {
                return;
            };

            let slot = item
                .downcast_ref::<gtk4::ListItem>()
                .and_then(|list_item| list_item.child())
                .and_downcast::<gtk4::Box>();
            let host = slot
                .as_ref()
                .and_then(|slot| slot.first_child())
                .and_downcast::<gtk4::Box>();

            // Row 0 is the header marker. Give this slot the header widget and
            // hide its post row.
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && list_item
                    .item()
                    .and_downcast::<gtk4::StringObject>()
                    .is_some()
                && let Some(host) = host.as_ref()
            {
                // GTK may bind a slot before unbinding the one it replaces, so
                // take the header off its previous host first. Reparenting a
                // widget that still has a parent is a warning.
                if let Some(previous) = header_for_bind.parent().and_downcast::<gtk4::Box>() {
                    previous.remove(&header_for_bind);
                    previous.set_visible(false);
                }
                host.append(&header_for_bind);
                host.set_visible(true);
                if let Some(post_row) = host.next_sibling() {
                    post_row.set_visible(false);
                }
                return;
            }

            // An ordinary post. Reclaim the header here too if this slot was
            // holding it; waiting on unbind would depend on GTK's ordering.
            if let Some(host) = host.as_ref() {
                if header_for_bind.parent().as_ref() == Some(host.upcast_ref::<gtk4::Widget>()) {
                    host.remove(&header_for_bind);
                }
                host.set_visible(false);
            }

            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(post_object) = list_item.item().and_downcast::<PostObject>()
                && let Some(post) = post_object.post()
                && let Some(post_row) = Self::post_row_of(list_item)
            {
                post_row.set_visible(true);
                post_row.bind(&post);
                post_row.set_list_position(list_item.position());
                let post_for_like = post.clone();
                let w = win.downgrade();
                post_row.connect_like_clicked(move |row, was_liked, like_uri| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    let mut post_with_state = post_for_like.clone();
                    post_with_state.viewer_like = if was_liked { like_uri } else { None };
                    let row_weak = row.downgrade();
                    w.imp()
                        .like_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_with_state, row_weak));
                });
                let post_for_repost = post.clone();
                let w = win.downgrade();
                post_row.connect_repost_clicked(move |row, was_reposted, repost_uri| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    let mut post_with_state = post_for_repost.clone();
                    post_with_state.viewer_repost = if was_reposted { repost_uri } else { None };
                    let row_weak = row.downgrade();
                    w.imp()
                        .repost_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_with_state, row_weak));
                });
                let post_for_quote = post.clone();
                let w = win.downgrade();
                post_row.connect_quote_clicked(move || {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .quote_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_for_quote.clone()));
                });
                let post_clone = post.clone();
                let w = win.downgrade();
                post_row.connect_reply_clicked(move || {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .reply_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_clone.clone()));
                });
                // Navigation callbacks for posts in profile
                let w = win.downgrade();
                post_row.set_post_clicked_callback(move |p| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .post_clicked_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(p));
                });
            }
        });

        // Over the flattened model, so row 0 is the header and row n+1 is
        // post n.
        let selection = gtk4::NoSelection::new(Some(flattened));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.add_css_class("background");

        // The list is the scroller's direct child, with nothing in between.
        // This page has never clamped its content width, and that is unchanged;
        // the banner still runs the full width of the window. Clamping it to
        // match the other-user profile would be an AdwClampScrollable in this
        // one line.
        scrolled.set_child(Some(&list_view));
        overlay.set_child(Some(&scrolled));

        // Loading spinner
        let spinner = gtk4::Spinner::new();
        spinner.update_property(&[gtk4::accessible::Property::Label("Loading")]);
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        profile_box.append(&overlay);

        // Store references
        let imp = self.imp();
        imp.profile_page_model.replace(Some(model));
        imp.profile_page_spinner.replace(Some(spinner));
        imp.profile_page_scrolled.replace(Some(scrolled.clone()));

        self.wire_feed_pagination(&scrolled, &feed_ctx);

        profile_box
    }

    /// Build the profile header section with banner, avatar, bio, and stats
    fn build_profile_header_section(&self) -> gtk4::Box {
        let header_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Banner: the placeholder color shows until a profile with a banner
        // image loads into the picture.
        let banner_overlay = gtk4::Overlay::new();
        banner_overlay.set_height_request(150);

        let banner_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        banner_box.add_css_class("profile-banner-placeholder");
        banner_box.set_overflow(gtk4::Overflow::Hidden);
        let banner_picture = gtk4::Picture::new();
        banner_picture.set_hexpand(true);
        banner_picture.set_vexpand(true);
        banner_picture.set_can_shrink(true);
        banner_picture.set_content_fit(gtk4::ContentFit::Cover);
        banner_picture.set_visible(false);
        banner_box.append(&banner_picture);
        banner_overlay.set_child(Some(&banner_box));

        // Avatar overlaid on banner, centered like the drill-down pages.
        let avatar = adw::Avatar::new(80, None, true);
        avatar.set_halign(gtk4::Align::Center);
        avatar.set_valign(gtk4::Align::End);
        avatar.set_margin_bottom(-40); // Overlap into content below
        banner_overlay.add_overlay(&avatar);

        header_box.append(&banner_overlay);

        // Info section with spacing for avatar overlap
        let info_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        info_box.set_margin_start(16);
        info_box.set_margin_end(16);
        info_box.set_margin_top(48); // Space for overlapping avatar
        info_box.set_margin_bottom(16);

        // Display name
        let name_label = gtk4::Label::new(None);
        name_label.add_css_class("title-1");
        name_label.set_halign(gtk4::Align::Center);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        info_box.append(&name_label);

        // Handle
        let handle_label = gtk4::Label::new(None);
        handle_label.add_css_class("dim-label");
        handle_label.set_halign(gtk4::Align::Center);
        handle_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        info_box.append(&handle_label);

        // Bio/description
        let bio_label = gtk4::Label::new(None);
        bio_label.set_wrap(true);
        bio_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        bio_label.set_halign(gtk4::Align::Center);
        bio_label.set_justify(gtk4::Justification::Center);
        bio_label.set_max_width_chars(60);
        bio_label.set_use_markup(true); // Bio links
        bio_label.set_visible(false); // Hidden until profile loads
        // Wired once here. update_profile_header runs twice per login,
        // cache then network, and would stack handlers.
        self.connect_bio_links(&bio_label);
        info_box.append(&bio_label);

        // Stats row
        let stats_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
        stats_box.set_halign(gtk4::Align::Center);
        stats_box.set_margin_top(8);

        // Followers, clickable. The gesture is wired once here; the click
        // reads current_profile so a later account switch needs no rewiring.
        let followers_box = Self::clickable_stat_box();
        let followers_count = gtk4::Label::new(Some("0"));
        followers_count.add_css_class("heading");
        followers_box.append(&followers_count);
        let followers_static = gtk4::Label::new(Some("followers"));
        followers_static.add_css_class("dim-label");
        followers_box.append(&followers_static);
        let win = self.downgrade();
        let followers_click = gtk4::GestureClick::new();
        followers_click.connect_released(move |_, _, _, _| {
            if let Some(win) = win.upgrade() {
                win.open_follow_list_for_current_profile(FollowListKind::Followers);
            }
        });
        followers_box.add_controller(followers_click);
        stats_box.append(&followers_box);

        // Following, same deal
        let following_box = Self::clickable_stat_box();
        let following_count = gtk4::Label::new(Some("0"));
        following_count.add_css_class("heading");
        following_box.append(&following_count);
        let following_static = gtk4::Label::new(Some("following"));
        following_static.add_css_class("dim-label");
        following_box.append(&following_static);
        let win = self.downgrade();
        let following_click = gtk4::GestureClick::new();
        following_click.connect_released(move |_, _, _, _| {
            if let Some(win) = win.upgrade() {
                win.open_follow_list_for_current_profile(FollowListKind::Following);
            }
        });
        following_box.add_controller(following_click);
        stats_box.append(&following_box);

        // Buttons need names before the first profile fills the counts in.
        followers_box
            .update_property(&[gtk4::accessible::Property::Label("Followers, opens list")]);
        following_box
            .update_property(&[gtk4::accessible::Property::Label("Following, opens list")]);

        // Posts count
        let posts_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let posts_count = gtk4::Label::new(Some("0"));
        posts_count.add_css_class("heading");
        posts_box.append(&posts_count);
        let posts_static = gtk4::Label::new(Some("posts"));
        posts_static.add_css_class("dim-label");
        posts_box.append(&posts_static);
        stats_box.append(&posts_box);

        info_box.append(&stats_box);

        // Edit Profile, this being the one profile that is yours.
        let edit_btn = gtk4::Button::with_label("Edit Profile");
        edit_btn.add_css_class("pill");
        edit_btn.set_halign(gtk4::Align::Center);
        edit_btn.set_margin_top(8);
        let win = self.downgrade();
        edit_btn.connect_clicked(move |_| {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(cb) = win.imp().edit_profile_callback.borrow().as_ref() {
                cb();
            }
        });
        info_box.append(&edit_btn);

        header_box.append(&info_box);

        // Store widget references
        let imp = self.imp();
        imp.profile_name_label.replace(Some(name_label));
        imp.profile_handle_label.replace(Some(handle_label));
        imp.profile_bio_label.replace(Some(bio_label));
        imp.profile_banner_picture.replace(Some(banner_picture));
        imp.profile_avatar.replace(Some(avatar));
        imp.profile_followers_label.replace(Some(followers_count));
        imp.profile_following_label.replace(Some(following_count));
        imp.profile_posts_label.replace(Some(posts_count));
        imp.profile_followers_box.replace(Some(followers_box));
        imp.profile_following_box.replace(Some(following_box));

        header_box
    }

    /// An empty stat box that announces itself as a button and shows a
    /// pointer, for follower and following counts that open their lists.
    fn clickable_stat_box() -> gtk4::Box {
        let stat_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(4)
            .accessible_role(gtk4::AccessibleRole::Button)
            .build();
        stat_box.set_cursor_from_name(Some("pointer"));
        stat_box
    }

    /// Route a click on an own-profile stat to the follow-list callback,
    /// carrying whichever profile the header currently shows.
    fn open_follow_list_for_current_profile(&self, kind: FollowListKind) {
        let profile = self.imp().current_profile.borrow().clone();
        let Some(profile) = profile else {
            return;
        };
        if let Some(cb) = self.imp().follow_list_clicked_callback.borrow().as_ref() {
            cb(profile, kind);
        }
    }

    /// A clickable follower or following stat for a pushed profile page.
    /// The page is rebuilt per push, so the count never needs updating.
    fn build_follow_stat(
        &self,
        profile: &Profile,
        kind: FollowListKind,
        count: Option<u32>,
    ) -> gtk4::Box {
        let noun = match kind {
            FollowListKind::Followers => "followers",
            FollowListKind::Following => "following",
        };

        let stat_box = Self::clickable_stat_box();
        if let Some(count) = count {
            let count_label = gtk4::Label::new(Some(&Self::format_count(count)));
            count_label.add_css_class("heading");
            stat_box.append(&count_label);
        }
        let noun_label = gtk4::Label::new(Some(noun));
        noun_label.add_css_class("dim-label");
        stat_box.append(&noun_label);

        let spoken = match count {
            Some(count) => format!("{} {}, opens list", Self::format_count(count), noun),
            None => format!("{}, opens list", kind.title()),
        };
        stat_box.update_property(&[gtk4::accessible::Property::Label(&spoken)]);

        let win = self.downgrade();
        let profile = profile.clone();
        let click = gtk4::GestureClick::new();
        click.connect_released(move |_, _, _, _| {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(cb) = win.imp().follow_list_clicked_callback.borrow().as_ref() {
                cb(profile.clone(), kind);
            }
        });
        stat_box.add_controller(click);

        stat_box
    }

    /// Route link activations on a bio label, matching post text: mentions
    /// open the profile in-app, hashtags do nothing yet, and anything else
    /// is a real URL for the browser, with a toast either way.
    fn connect_bio_links(&self, label: &gtk4::Label) {
        let win = self.downgrade();
        label.connect_activate_link(move |label, uri| {
            if let Some(handle) = uri.strip_prefix("bsky-mention://") {
                if let Some(win) = win.upgrade()
                    && let Some(cb) = win.imp().mention_clicked_callback.borrow().as_ref()
                {
                    cb(handle.to_string());
                }
                glib::Propagation::Stop
            } else if uri.starts_with("bsky-tag://") {
                // Styled only, like hashtags in posts.
                glib::Propagation::Stop
            } else {
                crate::ui::external::open_url(label, uri, "link");
                glib::Propagation::Stop
            }
        });
    }

    /// Update the profile page header with profile data
    pub fn update_profile_header(&self, profile: &Profile) {
        let imp = self.imp();
        imp.current_profile.replace(Some(profile.clone()));

        let display_name = profile.display_name.as_deref().unwrap_or(&profile.handle);

        // Banner, over the placeholder color when the profile has one
        if let Some(picture) = imp.profile_banner_picture.borrow().as_ref() {
            match &profile.banner {
                Some(url) => {
                    picture.set_visible(true);
                    crate::ui::avatar_cache::load_image_into_picture(picture.clone(), url.clone());
                }
                None => picture.set_visible(false),
            }
        }

        // Update name
        if let Some(label) = imp.profile_name_label.borrow().as_ref() {
            label.set_text(display_name);
        }

        // Update handle
        if let Some(label) = imp.profile_handle_label.borrow().as_ref() {
            label.set_text(&format!("@{}", profile.handle));
        }

        // Update bio
        if let Some(label) = imp.profile_bio_label.borrow().as_ref() {
            if let Some(bio) = profile.description.as_deref().filter(|b| !b.is_empty()) {
                crate::ui::rich_text::set_linkified(label, bio);
                label.set_visible(true);
            } else {
                label.set_visible(false);
            }
        }

        // Update avatar
        if let Some(avatar) = imp.profile_avatar.borrow().as_ref() {
            avatar.set_text(Some(display_name));
            if let Some(url) = &profile.avatar {
                crate::ui::avatar_cache::load_avatar(avatar.clone(), url.clone());
            }
        }

        // Update stats
        if let Some(label) = imp.profile_followers_label.borrow().as_ref() {
            let count = profile.followers_count.unwrap_or(0);
            label.set_text(&Self::format_count(count));
        }
        if let Some(label) = imp.profile_following_label.borrow().as_ref() {
            let count = profile.following_count.unwrap_or(0);
            label.set_text(&Self::format_count(count));
        }
        // The stat boxes read as buttons; keep what they say in step with
        // the counts they show.
        if let Some(stat_box) = imp.profile_followers_box.borrow().as_ref() {
            let spoken = format!(
                "{} followers, opens list",
                Self::format_count(profile.followers_count.unwrap_or(0))
            );
            stat_box.update_property(&[gtk4::accessible::Property::Label(&spoken)]);
        }
        if let Some(stat_box) = imp.profile_following_box.borrow().as_ref() {
            let spoken = format!(
                "{} following, opens list",
                Self::format_count(profile.following_count.unwrap_or(0))
            );
            stat_box.update_property(&[gtk4::accessible::Property::Label(&spoken)]);
        }
        if let Some(label) = imp.profile_posts_label.borrow().as_ref() {
            let count = profile.posts_count.unwrap_or(0);
            label.set_text(&Self::format_count(count));
        }
    }

    /// Format a count as "1K", "1.2M", etc.
    fn format_count(count: u32) -> String {
        if count >= 1_000_000 {
            format!("{:.1}M", count as f64 / 1_000_000.0)
        } else if count >= 1_000 {
            format!("{:.1}K", count as f64 / 1_000.0)
        } else {
            count.to_string()
        }
    }

    /// Set posts for the own profile page
    pub fn set_profile_posts(&self, posts: Vec<Post>) {
        if let Some(model) = self.imp().profile_page_model.borrow().as_ref() {
            model.remove_all();
            for post in posts {
                model.append(&PostObject::new(post));
            }
        }
    }

    /// Append more posts to the own profile page
    pub fn append_profile_posts(&self, posts: Vec<Post>) {
        if let Some(model) = self.imp().profile_page_model.borrow().as_ref() {
            for post in posts {
                model.append(&PostObject::new(post));
            }
        }
    }

    /// Show the profile page (top-level navigation, instant switch)
    pub fn show_profile_page(&self) {
        self.switch_to_page("profile");
    }

    /// Build the likes page
    fn build_likes_page(&self) -> adw::NavigationPage {
        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let title = gtk4::Label::new(Some("Likes"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));

        // Add window controls (close, minimize, maximize) to the end
        let window_controls = gtk4::WindowControls::new(gtk4::PackType::End);
        header.pack_end(&window_controls);

        content_box.append(&header);
        content_box.append(&self.build_likes_list());

        let page = adw::NavigationPage::new(&content_box, "Likes");
        page.set_tag(Some("likes"));
        page
    }

    /// Build the likes list widget
    fn build_likes_list(&self) -> gtk4::Box {
        let likes_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        likes_box.set_vexpand(true);

        let overlay = gtk4::Overlay::new();
        overlay.set_vexpand(true);

        let model = gio::ListStore::new::<PostObject>();
        let factory = gtk4::SignalListItemFactory::new();

        factory.connect_setup(|_, item| {
            let post_row = PostRow::new();
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() {
                list_item.set_child(Some(&post_row));
            }
        });
        Self::release_video_on_unbind(&factory);

        let win = self.downgrade();
        factory.connect_bind(move |_, item| {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(post_object) = list_item.item().and_downcast::<PostObject>()
                && let Some(post) = post_object.post()
                && let Some(post_row) = list_item.child().and_downcast::<PostRow>()
            {
                post_row.bind(&post);
                post_row.set_list_position(list_item.position());
                let post_for_like = post.clone();
                let w = win.downgrade();
                post_row.connect_like_clicked(move |row, was_liked, like_uri| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    let mut post_with_state = post_for_like.clone();
                    post_with_state.viewer_like = if was_liked { like_uri } else { None };
                    let row_weak = row.downgrade();
                    w.imp()
                        .like_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_with_state, row_weak));
                });
                let post_for_repost = post.clone();
                let w = win.downgrade();
                post_row.connect_repost_clicked(move |row, was_reposted, repost_uri| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    let mut post_with_state = post_for_repost.clone();
                    post_with_state.viewer_repost = if was_reposted { repost_uri } else { None };
                    let row_weak = row.downgrade();
                    w.imp()
                        .repost_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_with_state, row_weak));
                });
                let post_for_quote = post.clone();
                let w = win.downgrade();
                post_row.connect_quote_clicked(move || {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .quote_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_for_quote.clone()));
                });
                let post_clone = post.clone();
                let w = win.downgrade();
                post_row.connect_reply_clicked(move || {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .reply_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_clone.clone()));
                });
                // Navigation callbacks for posts in likes
                let w = win.downgrade();
                post_row.set_post_clicked_callback(move |p| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .post_clicked_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(p));
                });
            }
        });

        let selection = gtk4::NoSelection::new(Some(model.clone()));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.add_css_class("background");

        // Content width, and the scrollability the list needs to virtualize.
        // See `build_timeline`. The list has to be this widget's direct child
        // and this widget the scroller's; a box or overlay between them breaks
        // scrolling.
        let clamp = adw::ClampScrollable::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&clamp));
        overlay.set_child(Some(&scrolled));

        // Loading spinner
        let spinner = gtk4::Spinner::new();
        spinner.update_property(&[gtk4::accessible::Property::Label("Loading")]);
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        likes_box.append(&overlay);

        let empty_state = Self::empty_state_page(
            "emote-love-symbolic",
            "No likes yet",
            "Posts you like will appear here.",
        );
        likes_box.append(&empty_state);
        let win = self.downgrade();
        let error_state = Self::error_state_page(move || {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(cb) = win.imp().nav_changed_callback.borrow().as_ref() {
                cb(crate::ui::sidebar::NavItem::Likes);
            }
        });
        likes_box.append(&error_state);
        self.imp().likes_error_state.replace(Some(error_state));

        let skeleton = Self::skeleton_list();
        likes_box.append(&skeleton);
        self.imp().likes_skeleton.replace(Some(skeleton));

        self.imp().likes_overlay.replace(Some(overlay.clone()));
        self.imp().likes_empty_state.replace(Some(empty_state));

        // Store references
        let imp = self.imp();
        imp.likes_model.replace(Some(model));
        imp.likes_scrolled_window.replace(Some(scrolled.clone()));
        imp.likes_spinner.replace(Some(spinner));

        // Infinite scroll
        let adj = scrolled.vadjustment();
        let win = self.downgrade();
        adj.connect_value_changed(move |adj| {
            let Some(win) = win.upgrade() else {
                return;
            };
            let value = adj.value();
            let upper = adj.upper();
            let page_size = adj.page_size();
            if value >= upper - page_size - 200.0 {
                if let Some(cb) = win.imp().likes_load_more_callback.borrow().as_ref() {
                    cb();
                }
            }
        });

        likes_box
    }

    /// Show the likes page (top-level navigation, instant switch)
    pub fn show_likes_page(&self) {
        self.switch_to_page("likes");
    }

    /// Set liked posts in the likes list
    pub fn set_likes(&self, posts: Vec<Post>) {
        let empty = posts.is_empty();
        if let Some(model) = self.imp().likes_model.borrow().as_ref() {
            model.remove_all();
            for post in posts {
                model.append(&PostObject::new(post));
            }
        }
        let imp = self.imp();
        Self::apply_empty_state(&imp.likes_overlay, &imp.likes_empty_state, empty);
        if let Some(skeleton) = imp.likes_skeleton.borrow().as_ref() {
            skeleton.set_visible(false);
        }
        if let Some(err) = imp.likes_error_state.borrow().as_ref() {
            err.set_visible(false);
        }
    }

    /// Append more liked posts to the likes list
    pub fn append_likes(&self, posts: Vec<Post>) {
        if let Some(model) = self.imp().likes_model.borrow().as_ref() {
            for post in posts {
                model.append(&PostObject::new(post));
            }
        }
    }

    /// Set loading state for likes page
    pub fn set_likes_loading(&self, loading: bool) {
        let imp = self.imp();
        let empty = imp
            .likes_model
            .borrow()
            .as_ref()
            .is_none_or(|m| m.n_items() == 0);
        // A first load gets skeleton rows; later pages keep the corner
        // spinner under the content they extend.
        if let Some(skeleton) = imp.likes_skeleton.borrow().as_ref() {
            skeleton.set_visible(loading && empty);
        }
        if loading && empty {
            if let Some(overlay) = imp.likes_overlay.borrow().as_ref() {
                overlay.set_visible(false);
            }
        }
        if let Some(spinner) = imp.likes_spinner.borrow().as_ref() {
            spinner.set_visible(loading && !empty);
            spinner.set_spinning(loading && !empty);
        }
    }

    /// Set callback for loading more liked posts
    pub fn set_likes_load_more_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .likes_load_more_callback
            .replace(Some(Box::new(callback)));
    }

    // ======== Saved Posts Page ========

    /// Build the saved posts page
    fn build_bookmarks_page(&self) -> adw::NavigationPage {
        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let title = gtk4::Label::new(Some("Saved"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));

        // Add window controls (close, minimize, maximize) to the end
        let window_controls = gtk4::WindowControls::new(gtk4::PackType::End);
        header.pack_end(&window_controls);

        content_box.append(&header);
        content_box.append(&self.build_bookmarks_list());

        let page = adw::NavigationPage::new(&content_box, "Saved");
        page.set_tag(Some("bookmarks"));
        page
    }

    /// Build the saved posts list widget
    fn build_bookmarks_list(&self) -> gtk4::Box {
        let bookmarks_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        bookmarks_box.set_vexpand(true);

        let overlay = gtk4::Overlay::new();
        overlay.set_vexpand(true);

        let model = gio::ListStore::new::<PostObject>();
        let factory = gtk4::SignalListItemFactory::new();

        factory.connect_setup(|_, item| {
            let post_row = PostRow::new();
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() {
                list_item.set_child(Some(&post_row));
            }
        });
        Self::release_video_on_unbind(&factory);

        let win = self.downgrade();
        factory.connect_bind(move |_, item| {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(post_object) = list_item.item().and_downcast::<PostObject>()
                && let Some(post) = post_object.post()
                && let Some(post_row) = list_item.child().and_downcast::<PostRow>()
            {
                post_row.bind(&post);
                post_row.set_list_position(list_item.position());
                let post_for_like = post.clone();
                let w = win.downgrade();
                post_row.connect_like_clicked(move |row, was_liked, like_uri| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    let mut post_with_state = post_for_like.clone();
                    post_with_state.viewer_like = if was_liked { like_uri } else { None };
                    let row_weak = row.downgrade();
                    w.imp()
                        .like_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_with_state, row_weak));
                });
                let post_for_repost = post.clone();
                let w = win.downgrade();
                post_row.connect_repost_clicked(move |row, was_reposted, repost_uri| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    let mut post_with_state = post_for_repost.clone();
                    post_with_state.viewer_repost = if was_reposted { repost_uri } else { None };
                    let row_weak = row.downgrade();
                    w.imp()
                        .repost_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_with_state, row_weak));
                });
                let post_for_quote = post.clone();
                let w = win.downgrade();
                post_row.connect_quote_clicked(move || {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .quote_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_for_quote.clone()));
                });
                let post_clone = post.clone();
                let w = win.downgrade();
                post_row.connect_reply_clicked(move || {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .reply_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_clone.clone()));
                });
                // Navigation callbacks for saved posts
                let w = win.downgrade();
                post_row.set_post_clicked_callback(move |p| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .post_clicked_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(p));
                });
            }
        });

        let selection = gtk4::NoSelection::new(Some(model.clone()));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.add_css_class("background");

        // Content width, and the scrollability the list needs to virtualize.
        // See `build_timeline`. The list has to be this widget's direct child
        // and this widget the scroller's; a box or overlay between them breaks
        // scrolling.
        let clamp = adw::ClampScrollable::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&clamp));
        overlay.set_child(Some(&scrolled));

        // Loading spinner
        let spinner = gtk4::Spinner::new();
        spinner.update_property(&[gtk4::accessible::Property::Label("Loading")]);
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        // Empty state (shown instead of the overlay when nothing is saved)
        let empty_state = adw::StatusPage::new();
        empty_state.set_icon_name(Some("user-bookmarks-symbolic"));
        empty_state.set_title("Nothing saved yet");
        empty_state.set_description(Some("Save a post from its menu and it will show up here."));
        empty_state.set_vexpand(true);
        empty_state.set_visible(false);

        bookmarks_box.append(&overlay);
        bookmarks_box.append(&empty_state);
        let win = self.downgrade();
        let error_state = Self::error_state_page(move || {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(cb) = win.imp().nav_changed_callback.borrow().as_ref() {
                cb(crate::ui::sidebar::NavItem::Bookmarks);
            }
        });
        bookmarks_box.append(&error_state);
        self.imp().bookmarks_error_state.replace(Some(error_state));

        let skeleton = Self::skeleton_list();
        bookmarks_box.append(&skeleton);
        self.imp().bookmarks_skeleton.replace(Some(skeleton));

        // Store references
        let imp = self.imp();
        imp.bookmarks_model.replace(Some(model));
        imp.bookmarks_scrolled_window
            .replace(Some(scrolled.clone()));
        imp.bookmarks_spinner.replace(Some(spinner));
        imp.bookmarks_overlay.replace(Some(overlay));
        imp.bookmarks_empty_state.replace(Some(empty_state));

        // Infinite scroll
        let adj = scrolled.vadjustment();
        let win = self.downgrade();
        adj.connect_value_changed(move |adj| {
            let Some(win) = win.upgrade() else {
                return;
            };
            let value = adj.value();
            let upper = adj.upper();
            let page_size = adj.page_size();
            if value >= upper - page_size - 200.0 {
                if let Some(cb) = win.imp().bookmarks_load_more_callback.borrow().as_ref() {
                    cb();
                }
            }
        });

        bookmarks_box
    }

    /// Show the saved posts page (top-level navigation, instant switch)
    pub fn show_bookmarks_page(&self) {
        self.switch_to_page("bookmarks");
    }

    /// Set saved posts in the list, swapping in the empty state when there
    /// are none
    pub fn set_bookmarks(&self, posts: Vec<Post>) {
        let empty = posts.is_empty();
        if let Some(model) = self.imp().bookmarks_model.borrow().as_ref() {
            model.remove_all();
            for post in posts {
                model.append(&PostObject::new(post));
            }
        }
        if let Some(overlay) = self.imp().bookmarks_overlay.borrow().as_ref() {
            overlay.set_visible(!empty);
        }
        if let Some(empty_state) = self.imp().bookmarks_empty_state.borrow().as_ref() {
            empty_state.set_visible(empty);
        }
        if let Some(err) = self.imp().bookmarks_error_state.borrow().as_ref() {
            err.set_visible(false);
        }
        if let Some(skeleton) = self.imp().bookmarks_skeleton.borrow().as_ref() {
            skeleton.set_visible(false);
        }
    }

    /// Append more saved posts to the list
    pub fn append_bookmarks(&self, posts: Vec<Post>) {
        if let Some(model) = self.imp().bookmarks_model.borrow().as_ref() {
            for post in posts {
                model.append(&PostObject::new(post));
            }
        }
    }

    /// Take an unsaved post out of the Saved list right away, if the list is
    /// showing it. Bringing back the empty state when the last one goes.
    pub fn remove_bookmark(&self, uri: &str) {
        let Some(model) = self.imp().bookmarks_model.borrow().clone() else {
            return;
        };
        Self::remove_post_from_store(&model, uri);
        if model.n_items() == 0 {
            if let Some(overlay) = self.imp().bookmarks_overlay.borrow().as_ref() {
                overlay.set_visible(false);
            }
            if let Some(empty_state) = self.imp().bookmarks_empty_state.borrow().as_ref() {
                empty_state.set_visible(true);
            }
        }
    }

    /// Set loading state for the saved posts page
    pub fn set_bookmarks_loading(&self, loading: bool) {
        let imp = self.imp();
        let empty = imp
            .bookmarks_model
            .borrow()
            .as_ref()
            .is_none_or(|m| m.n_items() == 0);
        // A first load gets skeleton rows; later pages keep the corner
        // spinner under the content they extend.
        if let Some(skeleton) = imp.bookmarks_skeleton.borrow().as_ref() {
            skeleton.set_visible(loading && empty);
        }
        if loading && empty {
            if let Some(overlay) = imp.bookmarks_overlay.borrow().as_ref() {
                overlay.set_visible(false);
            }
        }
        if let Some(spinner) = imp.bookmarks_spinner.borrow().as_ref() {
            spinner.set_visible(loading && !empty);
            spinner.set_spinning(loading && !empty);
        }
    }

    /// Set callback for loading more saved posts
    pub fn set_bookmarks_load_more_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .bookmarks_load_more_callback
            .replace(Some(Box::new(callback)));
    }

    // ======== Search Page ========

    fn build_search_page(&self) -> adw::NavigationPage {
        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let title = gtk4::Label::new(Some("Search"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));

        // Add window controls (close, minimize, maximize) to the end
        let window_controls = gtk4::WindowControls::new(gtk4::PackType::End);
        header.pack_end(&window_controls);

        content_box.append(&header);
        content_box.append(&self.build_search_content());

        let page = adw::NavigationPage::new(&content_box, "Search");
        page.set_tag(Some("search"));
        page
    }

    /// Build the search content (search bar + results list)
    fn build_search_content(&self) -> gtk4::Box {
        let search_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        search_box.set_vexpand(true);

        // Search entry
        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Search…"));
        search_entry.set_margin_start(12);
        search_entry.set_margin_end(12);
        search_entry.set_margin_top(12);
        search_entry.set_margin_bottom(12);

        // Connect search activation (Enter key)
        let win = self.downgrade();
        search_entry.connect_activate(move |entry| {
            let Some(win) = win.upgrade() else {
                return;
            };
            let query = entry.text().to_string();
            if !query.is_empty() {
                if let Some(cb) = win.imp().search_callback.borrow().as_ref() {
                    cb(query);
                }
            }
        });

        search_box.append(&search_entry);

        // Store search entry reference
        self.imp().search_entry.replace(Some(search_entry));

        // One stack page per result kind. Posts and people paginate on
        // independent cursors, so each keeps its own list and scroller.
        let results_stack = adw::ViewStack::new();
        results_stack.set_vexpand(true);

        let switcher = adw::ViewSwitcher::new();
        switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
        switcher.set_stack(Some(&results_stack));
        switcher.set_halign(gtk4::Align::Center);
        switcher.set_margin_bottom(8);
        search_box.append(&switcher);

        // Post results list in overlay (for spinner)
        let overlay = gtk4::Overlay::new();
        overlay.set_vexpand(true);

        let model = gio::ListStore::new::<PostObject>();
        let factory = gtk4::SignalListItemFactory::new();

        factory.connect_setup(|_, item| {
            let post_row = PostRow::new();
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() {
                list_item.set_child(Some(&post_row));
            }
        });
        Self::release_video_on_unbind(&factory);

        let win = self.downgrade();
        factory.connect_bind(move |_, item| {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(post_object) = list_item.item().and_downcast::<PostObject>()
                && let Some(post) = post_object.post()
                && let Some(post_row) = list_item.child().and_downcast::<PostRow>()
            {
                post_row.bind(&post);
                post_row.set_list_position(list_item.position());
                let post_for_like = post.clone();
                let w = win.downgrade();
                post_row.connect_like_clicked(move |row, was_liked, like_uri| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    let mut post_with_state = post_for_like.clone();
                    post_with_state.viewer_like = if was_liked { like_uri } else { None };
                    let row_weak = row.downgrade();
                    w.imp()
                        .like_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_with_state, row_weak));
                });
                let post_for_repost = post.clone();
                let w = win.downgrade();
                post_row.connect_repost_clicked(move |row, was_reposted, repost_uri| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    let mut post_with_state = post_for_repost.clone();
                    post_with_state.viewer_repost = if was_reposted { repost_uri } else { None };
                    let row_weak = row.downgrade();
                    w.imp()
                        .repost_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_with_state, row_weak));
                });
                let post_for_quote = post.clone();
                let w = win.downgrade();
                post_row.connect_quote_clicked(move || {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .quote_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_for_quote.clone()));
                });
                let post_clone = post.clone();
                let w = win.downgrade();
                post_row.connect_reply_clicked(move || {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .reply_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_clone.clone()));
                });
                // Navigation callbacks for posts in search results
                let w = win.downgrade();
                post_row.set_post_clicked_callback(move |p| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .post_clicked_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(p));
                });
            }
        });

        let selection = gtk4::NoSelection::new(Some(model.clone()));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.add_css_class("background");

        // Content width, and the scrollability the list needs to virtualize.
        // See `build_timeline`. The list has to be this widget's direct child
        // and this widget the scroller's; a box or overlay between them breaks
        // scrolling.
        let clamp = adw::ClampScrollable::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&clamp));
        overlay.set_child(Some(&scrolled));

        // Loading spinner
        let spinner = gtk4::Spinner::new();
        spinner.update_property(&[gtk4::accessible::Property::Label("Loading")]);
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        // Inside the overlay: the stack owns this page, so the empty state
        // cannot be a sibling the way the other lists have one.
        let empty_state = Self::empty_state_page(
            "system-search-symbolic",
            "No posts found",
            "Nothing matched this search. Try different words.",
        );
        overlay.add_overlay(&empty_state);
        self.imp().search_empty_state.replace(Some(empty_state));

        results_stack.add_titled_with_icon(&overlay, Some("posts"), "Posts", "view-list-symbolic");

        // Store references
        let imp = self.imp();
        imp.search_model.replace(Some(model));
        imp.search_scrolled_window.replace(Some(scrolled.clone()));
        imp.search_spinner.replace(Some(spinner));

        // Infinite scroll
        let adj = scrolled.vadjustment();
        let win = self.downgrade();
        adj.connect_value_changed(move |adj| {
            let Some(win) = win.upgrade() else {
                return;
            };
            let value = adj.value();
            let upper = adj.upper();
            let page_size = adj.page_size();
            if value >= upper - page_size - 200.0 {
                if let Some(cb) = win.imp().search_load_more_callback.borrow().as_ref() {
                    cb();
                }
            }
        });

        self.build_search_people_page(&results_stack);
        search_box.append(&results_stack);

        search_box
    }

    /// Build the People page of the search results stack.
    ///
    /// The same virtualized chain as every feed: scroller over clamp
    /// scrollable over list view, nothing between. See `build_timeline`.
    fn build_search_people_page(&self, results_stack: &adw::ViewStack) {
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

        let win = self.downgrade();
        factory.connect_bind(move |_, item| {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(actor_object) = list_item.item().and_downcast::<ActorObject>()
                && let Some(profile) = actor_object.profile()
                && let Some(row) = list_item.child().and_downcast::<ActorRow>()
            {
                row.bind(&profile);
                row.set_bound_object(&actor_object);
                // Open the profile in-app on click
                let w = win.downgrade();
                row.set_activated_callback(move |profile| {
                    let Some(w) = w.upgrade() else {
                        return;
                    };
                    w.imp()
                        .profile_clicked_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(profile));
                });
            }
        });

        let selection = gtk4::NoSelection::new(Some(model.clone()));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.add_css_class("background");

        let clamp = adw::ClampScrollable::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&clamp));
        overlay.set_child(Some(&scrolled));

        // Loading spinner
        let spinner = gtk4::Spinner::new();
        spinner.update_property(&[gtk4::accessible::Property::Label("Loading")]);
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        let imp = self.imp();
        imp.search_people_model.replace(Some(model));
        imp.search_people_scrolled_window
            .replace(Some(scrolled.clone()));
        imp.search_people_spinner.replace(Some(spinner));

        // Infinite scroll
        let adj = scrolled.vadjustment();
        let win = self.downgrade();
        adj.connect_value_changed(move |adj| {
            let Some(win) = win.upgrade() else {
                return;
            };
            let value = adj.value();
            let upper = adj.upper();
            let page_size = adj.page_size();
            if value >= upper - page_size - 200.0 {
                if let Some(cb) = win.imp().search_people_load_more_callback.borrow().as_ref() {
                    cb();
                }
            }
        });

        // Same overlay placement as the posts page; see there.
        let empty_state = Self::empty_state_page(
            "system-search-symbolic",
            "No people found",
            "Nothing matched this search. Try a name or handle.",
        );

        // Suggestions fill the dead end when the network has some.
        let suggestions_column = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        suggestions_column.set_visible(false);
        let suggestions_title = gtk4::Label::new(Some("Suggested accounts"));
        suggestions_title.add_css_class("heading");
        suggestions_title.set_halign(gtk4::Align::Start);
        suggestions_title.set_margin_bottom(4);
        suggestions_column.append(&suggestions_title);
        let suggestions_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        suggestions_column.append(&suggestions_box);
        empty_state.set_child(Some(&suggestions_column));

        overlay.add_overlay(&empty_state);
        self.imp()
            .search_people_empty_state
            .replace(Some(empty_state));
        self.imp()
            .search_suggestions_box
            .replace(Some(suggestions_box));

        results_stack.add_titled_with_icon(
            &overlay,
            Some("people"),
            "People",
            "system-users-symbolic",
        );
    }

    /// Show the search page (top-level navigation, instant switch)
    pub fn show_search_page(&self) {
        self.switch_to_page("search");
    }

    /// Set search results in the search list
    pub fn set_search_results(&self, posts: Vec<Post>) {
        let empty = posts.is_empty();
        if let Some(model) = self.imp().search_model.borrow().as_ref() {
            model.remove_all();
            for post in posts {
                model.append(&PostObject::new(post));
            }
        }
        // Only a completed search lands here, so empty means no results,
        // not no search yet.
        if let Some(page) = self.imp().search_empty_state.borrow().as_ref() {
            page.set_visible(empty);
        }
    }

    /// Append more search results to the list
    pub fn append_search_results(&self, posts: Vec<Post>) {
        if let Some(model) = self.imp().search_model.borrow().as_ref() {
            for post in posts {
                model.append(&PostObject::new(post));
            }
        }
    }

    /// Set loading state for search page
    pub fn set_search_loading(&self, loading: bool) {
        if let Some(spinner) = self.imp().search_spinner.borrow().as_ref() {
            spinner.set_visible(loading);
            spinner.set_spinning(loading);
        }
    }

    /// Set callback for loading more search results
    pub fn set_search_load_more_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .search_load_more_callback
            .replace(Some(Box::new(callback)));
    }

    /// Set callback for when a search is submitted
    pub fn set_search_callback<F: Fn(String) + 'static>(&self, callback: F) {
        self.imp().search_callback.replace(Some(Box::new(callback)));
    }

    /// Set people results in the search list
    pub fn set_search_people_results(&self, profiles: Vec<Profile>) {
        let empty = profiles.is_empty();
        if let Some(model) = self.imp().search_people_model.borrow().as_ref() {
            model.remove_all();
            for profile in profiles {
                model.append(&ActorObject::new(profile));
            }
        }
        // See set_search_results.
        if let Some(page) = self.imp().search_people_empty_state.borrow().as_ref() {
            page.set_visible(empty);
        }
    }

    /// Append more people results to the list
    pub fn append_search_people_results(&self, profiles: Vec<Profile>) {
        if let Some(model) = self.imp().search_people_model.borrow().as_ref() {
            for profile in profiles {
                model.append(&ActorObject::new(profile));
            }
        }
    }

    /// Set loading state for the people page of search
    pub fn set_search_people_loading(&self, loading: bool) {
        if let Some(spinner) = self.imp().search_people_spinner.borrow().as_ref() {
            spinner.set_visible(loading);
            spinner.set_spinning(loading);
        }
    }

    /// Set callback for loading more people results
    pub fn set_search_people_load_more_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .search_people_load_more_callback
            .replace(Some(Box::new(callback)));
    }

    /// Clear search results, both pages
    pub fn clear_search_results(&self) {
        if let Some(model) = self.imp().search_model.borrow().as_ref() {
            model.remove_all();
        }
        if let Some(model) = self.imp().search_people_model.borrow().as_ref() {
            model.remove_all();
        }
        // A fresh search is not a search with no results.
        if let Some(page) = self.imp().search_empty_state.borrow().as_ref() {
            page.set_visible(false);
        }
        if let Some(page) = self.imp().search_people_empty_state.borrow().as_ref() {
            page.set_visible(false);
        }
        self.set_search_suggestions(vec![]);
    }

    /// The New Message picker: type a name, pick a person, chat.
    fn build_new_message_dialog(&self) -> adw::Dialog {
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        let header = adw::HeaderBar::new();
        let title = gtk4::Label::new(Some("New Message"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));
        content.append(&header);

        let entry = gtk4::SearchEntry::new();
        entry.set_placeholder_text(Some("Search people"));
        entry.set_margin_start(12);
        entry.set_margin_end(12);
        entry.set_margin_top(8);
        entry.set_margin_bottom(8);
        let win = self.downgrade();
        entry.connect_search_changed(move |entry| {
            let Some(win) = win.upgrade() else {
                return;
            };
            if let Some(cb) = win.imp().new_message_query_callback.borrow().as_ref() {
                cb(entry.text().to_string());
            }
        });
        content.append(&entry);

        let results = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&results));
        content.append(&scrolled);

        self.imp().new_message_results_box.replace(Some(results));

        let dialog = adw::Dialog::builder()
            .title("New Message")
            .content_width(420)
            .content_height(480)
            .child(&content)
            .build();
        // Stale state from the last opening would flash otherwise.
        self.imp().new_message_dialog.replace(Some(dialog.clone()));
        dialog
    }

    pub fn present_new_message_dialog(&self) {
        let dialog = self.build_new_message_dialog();
        dialog.present(Some(self));
    }

    /// People matching the picker's query; a row click hands the person to
    /// the chosen callback and the dialog steps aside.
    pub fn set_new_message_results(&self, profiles: Vec<Profile>) {
        let Some(rows) = self.imp().new_message_results_box.borrow().clone() else {
            return;
        };
        while let Some(child) = rows.first_child() {
            rows.remove(&child);
        }
        let win = self.downgrade();
        for profile in profiles {
            let row = ActorRow::new();
            row.bind(&profile);
            let w = win.clone();
            row.set_activated_callback(move |profile| {
                let Some(w) = w.upgrade() else {
                    return;
                };
                if let Some(dialog) = w.imp().new_message_dialog.borrow().as_ref() {
                    dialog.close();
                }
                if let Some(cb) = w.imp().new_message_chosen_callback.borrow().as_ref() {
                    cb(profile);
                }
            });
            rows.append(&row);
        }
    }

    pub fn set_new_message_query_callback<F: Fn(String) + 'static>(&self, callback: F) {
        self.imp()
            .new_message_query_callback
            .replace(Some(Box::new(callback)));
    }

    pub fn set_new_message_chosen_callback<F: Fn(Profile) + 'static>(&self, callback: F) {
        self.imp()
            .new_message_chosen_callback
            .replace(Some(Box::new(callback)));
    }

    /// Fill the empty people search with suggested accounts. An empty list
    /// clears them, leaving the plain no-results page.
    pub fn set_search_suggestions(&self, profiles: Vec<Profile>) {
        let Some(rows) = self.imp().search_suggestions_box.borrow().clone() else {
            return;
        };
        while let Some(child) = rows.first_child() {
            rows.remove(&child);
        }
        if let Some(column) = rows.parent() {
            column.set_visible(!profiles.is_empty());
        }
        let win = self.downgrade();
        for profile in profiles {
            let row = ActorRow::new();
            row.bind(&profile);
            let w = win.clone();
            row.set_activated_callback(move |profile| {
                let Some(w) = w.upgrade() else {
                    return;
                };
                if let Some(cb) = w.imp().profile_clicked_callback.borrow().as_ref() {
                    cb(profile);
                }
            });
            rows.append(&row);
        }
    }

    /// Focus the search entry
    pub fn focus_search_entry(&self) {
        if let Some(entry) = self.imp().search_entry.borrow().as_ref() {
            entry.grab_focus();
        }
    }

    /// One `set_X_load_failed` per rail feed; see `apply_load_failed`.
    pub fn set_timeline_load_failed(&self) {
        let imp = self.imp();
        let empty = imp
            .timeline_model
            .borrow()
            .as_ref()
            .is_none_or(|m| m.n_items() == 0);
        Self::apply_load_failed(
            empty,
            &imp.timeline_overlay,
            &imp.timeline_empty_state,
            &imp.timeline_error_state,
            &imp.timeline_skeleton,
        );
    }

    pub fn set_mentions_load_failed(&self) {
        let imp = self.imp();
        let empty = imp
            .mentions_model
            .borrow()
            .as_ref()
            .is_none_or(|m| m.n_items() == 0);
        Self::apply_load_failed(
            empty,
            &imp.mentions_overlay,
            &imp.mentions_empty_state,
            &imp.mentions_error_state,
            &imp.mentions_skeleton,
        );
    }

    pub fn set_activity_load_failed(&self) {
        let imp = self.imp();
        let empty = imp
            .activity_model
            .borrow()
            .as_ref()
            .is_none_or(|m| m.n_items() == 0);
        Self::apply_load_failed(
            empty,
            &imp.activity_overlay,
            &imp.activity_empty_state,
            &imp.activity_error_state,
            &imp.activity_skeleton,
        );
    }

    pub fn set_likes_load_failed(&self) {
        let imp = self.imp();
        let empty = imp
            .likes_model
            .borrow()
            .as_ref()
            .is_none_or(|m| m.n_items() == 0);
        Self::apply_load_failed(
            empty,
            &imp.likes_overlay,
            &imp.likes_empty_state,
            &imp.likes_error_state,
            &imp.likes_skeleton,
        );
    }

    pub fn set_bookmarks_load_failed(&self) {
        let imp = self.imp();
        let empty = imp
            .bookmarks_model
            .borrow()
            .as_ref()
            .is_none_or(|m| m.n_items() == 0);
        Self::apply_load_failed(
            empty,
            &imp.bookmarks_overlay,
            &imp.bookmarks_empty_state,
            &imp.bookmarks_error_state,
            &imp.bookmarks_skeleton,
        );
    }

    pub fn set_conversations_load_failed(&self) {
        let imp = self.imp();
        let empty = imp
            .chat_model
            .borrow()
            .as_ref()
            .is_none_or(|m| m.n_items() == 0);
        Self::apply_load_failed(
            empty,
            &imp.chat_overlay,
            &imp.chat_empty_state,
            &imp.chat_error_state,
            &imp.chat_skeleton,
        );
    }

    /// Show or clear the offline banner, saying so aloud either way.
    pub fn set_offline(&self, offline: bool) {
        let Some(banner) = self.imp().offline_banner.borrow().clone() else {
            return;
        };
        if banner.is_revealed() == offline {
            return;
        }
        banner.set_revealed(offline);
        self.announce(
            if offline {
                "You're offline"
            } else {
                "Back online"
            },
            gtk4::AccessibleAnnouncementPriority::Medium,
        );
    }

    /// Show a toast notification
    pub fn show_toast(&self, message: &str) {
        if let Some(overlay) = self.imp().toast_overlay.borrow().as_ref() {
            let toast = adw::Toast::new(message);
            toast.set_timeout(3); // 3 seconds
            overlay.add_toast(toast);
            // Toasts are how errors reach the user; say them aloud too.
            self.announce(message, gtk4::AccessibleAnnouncementPriority::Medium);
        }
    }

    /// Show a toast with an action button
    pub fn show_toast_with_action(
        &self,
        message: &str,
        button_label: &str,
        action: impl Fn() + 'static,
    ) {
        if let Some(overlay) = self.imp().toast_overlay.borrow().as_ref() {
            let toast = adw::Toast::new(message);
            toast.set_timeout(5); // 5 seconds for actionable toasts
            toast.set_button_label(Some(button_label));
            toast.connect_button_clicked(move |_| {
                action();
            });
            overlay.add_toast(toast);
        }
    }

    // ======== Post removal ========

    /// Take a deleted post out of every list that is showing it.
    ///
    /// Walks the widget tree rather than keeping a registry: a pushed profile
    /// page builds a model nothing else holds, and a thread page holds bare
    /// `PostRow`s in a box. A virtualized list is edited through its model
    /// only; a `PostRow` met outside any `GtkListView` is a thread row and
    /// comes straight out of its box.
    pub fn remove_post(&self, uri: &str) {
        let mut thread_rows = Vec::new();
        Self::collect_post_removals(self.upcast_ref::<gtk4::Widget>(), uri, &mut thread_rows);
        for row in thread_rows {
            if let Some(parent) = row.parent().and_downcast::<gtk4::Box>() {
                parent.remove(&row);
            }
        }
        self.pop_thread_pages_for(uri);
    }

    /// Close any open thread page rooted at a deleted post. Without this,
    /// deleting the root left the thread page up with nothing to show.
    fn pop_thread_pages_for(&self, uri: &str) {
        let tag = format!("thread:{uri}");
        for section in [
            "home",
            "mentions",
            "activity",
            "chat",
            "profile",
            "likes",
            "bookmarks",
            "search",
        ] {
            let Some(nav_view) = self.nav_view_named(section) else {
                continue;
            };
            let Some(page) = nav_view.find_page(&tag) else {
                continue;
            };
            // Pop to the page underneath, taking the thread page and anything
            // stacked on top of it off in one transition.
            let stack = nav_view.navigation_stack();
            let index = (0..stack.n_items())
                .find(|&i| stack.item(i).as_ref() == Some(page.upcast_ref::<glib::Object>()));
            let Some(index) = index else { continue };
            if index == 0 {
                continue;
            }
            if let Some(below) = stack.item(index - 1).and_downcast::<adw::NavigationPage>() {
                nav_view.pop_to_page(&below);
            }
        }
    }

    /// Remove `uri` from every post model under `widget`. Rows inside a
    /// `GtkListView` are pooled, so those subtrees are never descended into;
    /// matching rows found elsewhere are collected for the caller to unparent.
    fn collect_post_removals(widget: &gtk4::Widget, uri: &str, thread_rows: &mut Vec<PostRow>) {
        if let Some(list) = widget.downcast_ref::<gtk4::ListView>() {
            let model = list
                .model()
                .and_then(|m| m.downcast::<gtk4::NoSelection>().ok())
                .and_then(|ns| ns.model());
            if let Some(model) = model {
                if let Some(store) = model.downcast_ref::<gio::ListStore>() {
                    Self::remove_post_from_store(store, uri);
                } else if let Some(flat) = model.downcast_ref::<gtk4::FlattenListModel>() {
                    // The own profile page: a store of stores, with the posts
                    // in one of them and the header marker in another.
                    if let Some(sections) = flat.model() {
                        for i in 0..sections.n_items() {
                            if let Some(store) = sections.item(i).and_downcast::<gio::ListStore>() {
                                Self::remove_post_from_store(&store, uri);
                            }
                        }
                    }
                }
            }
            return;
        }
        if let Some(row) = widget.downcast_ref::<PostRow>() {
            if row.post_uri().as_deref() == Some(uri) {
                thread_rows.push(row.clone());
            }
            return;
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            Self::collect_post_removals(&c, uri, thread_rows);
        }
    }

    fn remove_post_from_store(store: &gio::ListStore, uri: &str) {
        let mut i = store.n_items();
        while i > 0 {
            i -= 1;
            let matches = store
                .item(i)
                .and_downcast::<PostObject>()
                .and_then(|o| o.post())
                .is_some_and(|p| p.uri == uri);
            if matches {
                store.remove(i);
            }
        }
    }

    // ======== Settings Page ========

    fn build_settings_page(&self) -> adw::ToolbarView {
        use crate::state::AppSettings;

        // Header bar with back button and "Settings" title
        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);
        header.set_title_widget(Some(&adw::WindowTitle::new("Settings", "")));

        // Back button
        let back_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
        back_btn.add_css_class("flat");
        back_btn.set_tooltip_text(Some("Back"));
        back_btn.update_property(&[gtk4::accessible::Property::Label("Back to previous page")]);
        let window_weak = self.downgrade();
        back_btn.connect_clicked(move |_| {
            if let Some(window) = window_weak.upgrade() {
                window.leave_settings();
            }
        });
        header.pack_start(&back_btn);

        let window_controls = gtk4::WindowControls::new(gtk4::PackType::End);
        header.pack_end(&window_controls);

        // Split layout: category list on the left, one page per category right
        let split_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        split_box.set_vexpand(true);

        // ---- Settings sidebar ----
        let sidebar_scrolled = gtk4::ScrolledWindow::new();
        sidebar_scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        sidebar_scrolled.set_vexpand(true);
        // Fixed width for the settings sidebar
        sidebar_scrolled.set_size_request(200, -1);

        let sidebar_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        sidebar_box.add_css_class("settings-sidebar");

        let sidebar_list = gtk4::ListBox::new();
        sidebar_list.set_selection_mode(gtk4::SelectionMode::Single);
        sidebar_list.add_css_class("navigation-sidebar");
        sidebar_list.update_property(&[gtk4::accessible::Property::Label("Settings categories")]);

        // ---- Content stack ----
        let content_stack = adw::ViewStack::new();
        content_stack.set_hexpand(true);
        content_stack.set_vexpand(true);

        // Load current settings once
        let current_settings = AppSettings::load();

        // Sidebar order.
        let pages = [
            self.build_settings_feed_page(&current_settings),
            self.build_settings_display_page(&current_settings),
            self.build_settings_accessibility_page(&current_settings),
            self.build_settings_account_page(),
        ];

        for page in &pages {
            // AdwPreferencesPage carries its own name, title and icon, so the
            // sidebar row and stack page come from one source. A page that
            // forgets set_name or set_icon_name would silently vanish from
            // both, so warn instead of skipping.
            let (Some(name), Some(icon_name)) = (page.name(), page.icon_name()) else {
                debug_assert!(
                    false,
                    "settings page \"{}\" has no name or no icon and cannot be shown",
                    page.title()
                );
                eprintln!(
                    "hangar: settings page \"{}\" has no name or no icon and was not added",
                    page.title()
                );
                continue;
            };
            let label = page.title();
            content_stack.add_titled_with_icon(page, Some(&name), &label, &icon_name);

            let row = gtk4::ListBoxRow::new();
            row.set_widget_name(&name);
            let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
            row_box.set_margin_top(8);
            row_box.set_margin_bottom(8);
            row_box.set_margin_start(12);
            row_box.set_margin_end(12);
            let icon = gtk4::Image::from_icon_name(&icon_name);
            icon.set_pixel_size(18);
            row_box.append(&icon);
            let lbl = gtk4::Label::new(Some(&label));
            lbl.set_halign(gtk4::Align::Start);
            row_box.append(&lbl);
            row.set_child(Some(&row_box));
            row.update_property(&[gtk4::accessible::Property::Label(&label)]);
            sidebar_list.append(&row);
        }

        sidebar_box.append(&sidebar_list);
        sidebar_scrolled.set_child(Some(&sidebar_box));

        // Separator between sidebar and content
        let separator = gtk4::Separator::new(gtk4::Orientation::Vertical);

        // Selection rather than activation, so arrowing through the categories
        // shows each one without needing a second keypress.
        let stack_ref = content_stack.clone();
        sidebar_list.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                let name = row.widget_name();
                stack_ref.set_visible_child_name(&name);
            }
        });

        // Select first row by default
        if let Some(first_row) = sidebar_list.row_at_index(0) {
            sidebar_list.select_row(Some(&first_row));
        }

        split_box.append(&sidebar_scrolled);
        split_box.append(&separator);
        split_box.append(&content_stack);

        // ToolbarView rather than a plain Box, so the header picks up the
        // shading that says the page under it has scrolled.
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&split_box));
        toolbar
    }

    /// Build the Feed category of the settings page
    fn build_settings_feed_page(
        &self,
        current_settings: &crate::state::AppSettings,
    ) -> adw::PreferencesPage {
        let page = adw::PreferencesPage::new();
        page.set_name(Some("feed"));
        page.set_title("Feed");
        page.set_icon_name(Some("view-list-symbolic"));

        let feed_group = adw::PreferencesGroup::new();
        feed_group.set_title("Timeline");
        feed_group.set_description(Some(
            "Controls what appears in your main timeline. Threads, profiles and likes are unaffected.",
        ));

        let hide_replies_row = adw::ActionRow::builder()
            .title("Hide Replies")
            .subtitle("Show only top-level posts, the way bsky.app does")
            .build();

        let hide_replies_switch = gtk4::Switch::new();
        hide_replies_switch.set_valign(gtk4::Align::Center);
        hide_replies_switch.set_active(current_settings.hide_replies_in_feed);
        hide_replies_switch.set_tooltip_text(Some("Toggle replies in the main feed"));
        hide_replies_switch
            .update_property(&[gtk4::accessible::Property::Label("Hide replies in feed")]);

        let window_weak = self.downgrade();
        hide_replies_switch.connect_state_set(move |_switch, state| {
            let mut settings = crate::state::AppSettings::load();
            settings.hide_replies_in_feed = state;
            if let Err(e) = settings.save() {
                eprintln!("Failed to save settings: {e}");
            }

            // `filter_feed_posts` runs as a batch enters the model, so writing
            // the file alone left the timeline as it was until the next fetch.
            // Refetch.
            if let Some(window) = window_weak.upgrade() {
                window.refresh_feed();
            }
            glib::Propagation::Proceed
        });

        hide_replies_row.add_suffix(&hide_replies_switch);
        hide_replies_row.set_activatable_widget(Some(&hide_replies_switch));
        feed_group.add(&hide_replies_row);

        page.add(&feed_group);

        // ---- Media ----
        let media_group = adw::PreferencesGroup::new();
        media_group.set_title("Media");
        media_group.set_description(Some(
            "Applies to the main timeline. Threads, profiles and search always wait for you to press play.",
        ));

        let autoplay_labels: Vec<&str> = crate::state::VideoAutoplay::ALL
            .iter()
            .map(|mode| mode.label())
            .collect();
        let autoplay_model = gtk4::StringList::new(&autoplay_labels);
        let autoplay_row = adw::ComboRow::builder()
            .title("Autoplay Videos")
            .subtitle("Play the video nearest the middle of the feed as you scroll")
            .model(&autoplay_model)
            .build();
        let selected = crate::state::VideoAutoplay::ALL
            .iter()
            .position(|mode| *mode == current_settings.video_autoplay)
            .unwrap_or(0);
        autoplay_row.set_selected(selected as u32);
        autoplay_row.update_property(&[gtk4::accessible::Property::Label(
            "Autoplay videos in the timeline",
        )]);

        let window_weak = self.downgrade();
        autoplay_row.connect_selected_notify(move |row| {
            let mode = crate::state::VideoAutoplay::ALL
                .get(row.selected() as usize)
                .copied()
                .unwrap_or_default();

            let mut settings = crate::state::AppSettings::load();
            settings.video_autoplay = mode;
            if let Err(e) = settings.save() {
                eprintln!("Failed to save settings: {e}");
            }

            // No refetch needed here, unlike Hide Replies. The feed
            // contents are the same and the director applies this live.
            if let Some(window) = window_weak.upgrade() {
                let director = window.imp().video_director.borrow().clone();
                if let Some(director) = director {
                    director.set_autoplay(mode);
                }
            }
        });

        media_group.add(&autoplay_row);
        page.add(&media_group);

        page
    }

    /// Build the Display category of the settings page
    fn build_settings_display_page(
        &self,
        current_settings: &crate::state::AppSettings,
    ) -> adw::PreferencesPage {
        use crate::state::FontSize;

        let page = adw::PreferencesPage::new();
        page.set_name(Some("display"));
        page.set_title("Display");
        page.set_icon_name(Some("preferences-desktop-appearance-symbolic"));

        // ---- Appearance section ----
        let appearance_group = adw::PreferencesGroup::new();
        appearance_group.set_title("Appearance");

        let scheme_model = gtk4::StringList::new(&["System", "Light", "Dark"]);
        let scheme_row = adw::ComboRow::builder()
            .title("Color Scheme")
            .model(&scheme_model)
            .build();

        let active_index = match current_settings.color_scheme {
            crate::state::ColorScheme::System => 0u32,
            crate::state::ColorScheme::Light => 1,
            crate::state::ColorScheme::Dark => 2,
        };
        scheme_row.set_selected(active_index);

        scheme_row.update_property(&[gtk4::accessible::Property::Label("Color scheme preference")]);

        let window_weak = self.downgrade();
        scheme_row.connect_selected_notify(move |row| {
            let scheme = match row.selected() {
                1 => crate::state::ColorScheme::Light,
                2 => crate::state::ColorScheme::Dark,
                _ => crate::state::ColorScheme::System,
            };

            let mut settings = crate::state::AppSettings::load();
            settings.color_scheme = scheme;
            if let Err(e) = settings.save() {
                eprintln!("Failed to save settings: {e}");
            }

            if let Some(window) = window_weak.upgrade() {
                window.apply_color_scheme(scheme);
            }
        });

        appearance_group.add(&scheme_row);
        page.add(&appearance_group);

        // ---- Post Text Size section ----
        let display_group = adw::PreferencesGroup::new();
        display_group.set_title("Post Text Size");
        display_group.set_description(Some("Applies to posts in your feed and the compose view."));

        // Text Size label row
        let size_label_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        size_label_row.set_margin_top(8);
        let size_title = gtk4::Label::new(Some("Size:"));
        size_title.set_halign(gtk4::Align::Start);
        size_title.add_css_class("heading");
        size_label_row.append(&size_title);
        let size_spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        size_spacer.set_hexpand(true);
        size_label_row.append(&size_spacer);
        let size_value_label = gtk4::Label::new(Some("Default"));
        size_value_label.add_css_class("dim-label");
        size_label_row.append(&size_value_label);

        // Slider
        let scale = gtk4::Scale::with_range(
            gtk4::Orientation::Horizontal,
            FontSize::MIN,
            FontSize::MAX,
            FontSize::STEP,
        );
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        scale.set_margin_top(4);
        scale.set_margin_bottom(4);

        // Add marks at each step
        for &step in FontSize::STEPS {
            scale.add_mark(step, gtk4::PositionType::Bottom, None);
        }

        // Load current font size
        scale.set_value(current_settings.font_size.scale_factor());
        size_value_label.set_text(current_settings.font_size.label());

        // ---- Preview post ----
        let preview_frame = gtk4::Frame::new(None);
        preview_frame.set_margin_top(16);
        preview_frame.add_css_class("card");

        let preview_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        preview_box.set_margin_top(12);
        preview_box.set_margin_bottom(12);
        preview_box.set_margin_start(12);
        preview_box.set_margin_end(12);
        preview_box.add_css_class("text-size-preview");

        // Preview avatar
        let preview_avatar = adw::Avatar::new(42, Some("Hangar"), true);
        let avatar_col = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        avatar_col.set_valign(gtk4::Align::Start);
        avatar_col.append(&preview_avatar);
        preview_box.append(&avatar_col);

        // Preview content
        let preview_content = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        preview_content.set_hexpand(true);

        // Preview header
        let preview_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        let preview_name = gtk4::Label::new(Some("Hangar for Bluesky"));
        preview_name.set_halign(gtk4::Align::Start);
        preview_name.add_css_class("preview-heading");
        preview_header.append(&preview_name);
        let preview_handle = gtk4::Label::new(Some("  @hangar.app"));
        preview_handle.add_css_class("dim-label");
        preview_handle.add_css_class("preview-caption");
        preview_header.append(&preview_handle);
        let preview_dot = gtk4::Label::new(Some("  \u{00b7}  "));
        preview_dot.add_css_class("dim-label");
        preview_dot.add_css_class("preview-caption");
        preview_header.append(&preview_dot);
        let preview_time = gtk4::Label::new(Some("2m"));
        preview_time.add_css_class("dim-label");
        preview_time.add_css_class("preview-caption");
        preview_header.append(&preview_time);
        preview_content.append(&preview_header);

        // Preview body text
        let preview_body = gtk4::Label::new(Some(
            "This is a preview of how posts will look at this text size. Adjust the slider above to find your preferred reading size.",
        ));
        preview_body.set_halign(gtk4::Align::Fill);
        preview_body.set_hexpand(true);
        preview_body.set_wrap(true);
        preview_body.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        preview_body.set_xalign(0.0);
        preview_body.add_css_class("preview-body");
        preview_content.append(&preview_body);

        // Preview action bar
        let preview_actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
        preview_actions.set_margin_top(8);
        let reply_icon = gtk4::Image::from_icon_name("mail-reply-sender-symbolic");
        reply_icon.add_css_class("dim-label");
        reply_icon.set_pixel_size(16);
        preview_actions.append(&reply_icon);
        let repost_icon = gtk4::Image::from_icon_name("media-playlist-repeat-symbolic");
        repost_icon.add_css_class("dim-label");
        repost_icon.set_pixel_size(16);
        preview_actions.append(&repost_icon);
        let like_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let like_icon = gtk4::Image::from_icon_name("emote-love-symbolic");
        like_icon.add_css_class("dim-label");
        like_icon.set_pixel_size(16);
        like_box.append(&like_icon);
        let like_count = gtk4::Label::new(Some("42"));
        like_count.add_css_class("dim-label");
        like_count.add_css_class("preview-caption");
        like_box.append(&like_count);
        preview_actions.append(&like_box);
        preview_content.append(&preview_actions);

        preview_box.append(&preview_content);
        preview_frame.set_child(Some(&preview_box));

        // Assemble the display group using a custom layout box
        let display_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        display_box.append(&size_label_row);
        display_box.append(&scale);
        display_box.append(&preview_frame);
        display_group.add(&display_box);
        page.add(&display_group);

        // Handle slider value changes
        let window_weak = self.downgrade();
        let label_ref = size_value_label.clone();
        scale.connect_value_changed(move |s| {
            let value = s.value();
            let snapped = (value / FontSize::STEP).round() * FontSize::STEP;
            let font_size = FontSize(snapped);

            label_ref.set_text(font_size.label());

            let mut settings = crate::state::AppSettings::load();
            settings.font_size = font_size;
            if let Err(e) = settings.save() {
                eprintln!("Failed to save settings: {e}");
            }

            if let Some(window) = window_weak.upgrade() {
                window.apply_font_size(font_size);
                if let Some(cb) = window.imp().font_size_changed_callback.borrow().as_ref() {
                    cb(font_size);
                }
            }
        });

        page
    }

    /// Build the Accessibility category of the settings page
    fn build_settings_accessibility_page(
        &self,
        current_settings: &crate::state::AppSettings,
    ) -> adw::PreferencesPage {
        let page = adw::PreferencesPage::new();
        page.set_name(Some("accessibility"));
        page.set_title("Accessibility");
        page.set_icon_name(Some("preferences-desktop-accessibility-symbolic"));

        // ---- Motion section ----
        let motion_group = adw::PreferencesGroup::new();
        motion_group.set_title("Motion");
        motion_group.set_description(Some(
            "System reduced-motion preference is always respected. This toggle forces reduced motion regardless of system setting.",
        ));

        // Reduce Motion toggle using ActionRow
        let reduce_motion_row = adw::ActionRow::builder()
            .title("Reduce Motion")
            .subtitle("Disable animations and transitions")
            .build();

        let reduce_motion_switch = gtk4::Switch::new();
        reduce_motion_switch.set_valign(gtk4::Align::Center);
        reduce_motion_switch.set_active(current_settings.reduce_motion);
        reduce_motion_switch.set_tooltip_text(Some("Toggle reduced motion"));
        reduce_motion_switch.update_property(&[gtk4::accessible::Property::Label("Reduce motion")]);

        let window_weak = self.downgrade();
        reduce_motion_switch.connect_state_set(move |_switch, state| {
            let mut settings = crate::state::AppSettings::load();
            settings.reduce_motion = state;
            if let Err(e) = settings.save() {
                eprintln!("Failed to save settings: {e}");
            }

            if let Some(window) = window_weak.upgrade() {
                window.apply_reduce_motion(state);
            }
            glib::Propagation::Proceed
        });

        reduce_motion_row.add_suffix(&reduce_motion_switch);
        reduce_motion_row.set_activatable_widget(Some(&reduce_motion_switch));
        motion_group.add(&reduce_motion_row);
        page.add(&motion_group);

        // Media
        let media_group = adw::PreferencesGroup::new();
        media_group.set_title("Media");

        let require_alt_row = adw::ActionRow::builder()
            .title("Require Alt Text")
            .subtitle("Posting waits until every attached image is described")
            .build();

        let require_alt_switch = gtk4::Switch::new();
        require_alt_switch.set_valign(gtk4::Align::Center);
        require_alt_switch.set_active(current_settings.require_alt_text);
        require_alt_switch
            .update_property(&[gtk4::accessible::Property::Label("Require alt text")]);

        require_alt_switch.connect_state_set(move |_switch, state| {
            let mut settings = crate::state::AppSettings::load();
            settings.require_alt_text = state;
            if let Err(e) = settings.save() {
                eprintln!("Failed to save settings: {e}");
            }
            // Compose dialogs read this when they open; nothing live to touch.
            glib::Propagation::Proceed
        });

        require_alt_row.add_suffix(&require_alt_switch);
        require_alt_row.set_activatable_widget(Some(&require_alt_switch));
        media_group.add(&require_alt_row);
        page.add(&media_group);

        page
    }

    /// Build the Account category of the settings page
    fn build_settings_account_page(&self) -> adw::PreferencesPage {
        let page = adw::PreferencesPage::new();
        page.set_name(Some("account"));
        page.set_title("Account");
        page.set_icon_name(Some("avatar-default-symbolic"));

        // ---- Content & Safety section ----
        let safety_group = adw::PreferencesGroup::new();
        safety_group.set_title("Content &amp; Safety");
        safety_group.set_description(Some("Manage content preferences on the Bluesky website."));

        let safety_row = adw::ActionRow::builder()
            .title("Content Filtering")
            .subtitle("Manage content labels and filters")
            .activatable(true)
            .build();
        safety_row.add_suffix(&gtk4::Image::from_icon_name("window-new-symbolic"));
        safety_row.update_property(&[gtk4::accessible::Property::Label(
            "Content Filtering (opens in browser)",
        )]);
        safety_row.connect_activated(|row| {
            crate::ui::external::open_url(
                row,
                "https://bsky.app/settings/content-and-media",
                "content filtering",
            );
        });
        safety_group.add(&safety_row);

        let moderation_row = adw::ActionRow::builder()
            .title("Moderation")
            .subtitle("Manage muted words, accounts, and lists")
            .activatable(true)
            .build();
        moderation_row.add_suffix(&gtk4::Image::from_icon_name("window-new-symbolic"));
        moderation_row.update_property(&[gtk4::accessible::Property::Label(
            "Moderation (opens in browser)",
        )]);
        moderation_row.connect_activated(|row| {
            crate::ui::external::open_url(
                row,
                "https://bsky.app/moderation",
                "moderation settings",
            );
        });
        safety_group.add(&moderation_row);
        page.add(&safety_group);

        // ---- Data section ----
        let data_group = adw::PreferencesGroup::new();
        data_group.set_title("Data");

        let cache_row = adw::ActionRow::builder()
            .title("Clear Cache")
            .subtitle("Remove cached posts, profiles, and images")
            .build();

        let cache_btn = gtk4::Button::with_label("Clear");
        cache_btn.add_css_class("destructive-action");
        cache_btn.set_valign(gtk4::Align::Center);
        cache_btn.set_tooltip_text(Some("Clear all cached data"));
        cache_btn.update_property(&[gtk4::accessible::Property::Label("Clear cache")]);

        let window_weak = self.downgrade();
        cache_btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            btn.set_label("Cleared");

            // Delete cache files from disk
            if let Some(window) = window_weak.upgrade() {
                if let Some(did) = window.imp().current_user_did.borrow().as_ref() {
                    if let Some(data_dir) = dirs::data_dir() {
                        let safe_did = did.replace(':', "_");
                        let cache_dir = data_dir.join("hangar").join(safe_did);
                        let _ = std::fs::remove_dir_all(&cache_dir);
                    }
                }
            }
        });

        cache_row.add_suffix(&cache_btn);
        data_group.add(&cache_row);
        page.add(&data_group);

        page
    }

    /// Show the settings page (top-level navigation, instant switch)
    pub fn show_settings_page(&self) {
        // Remember the current page for the back button, unless settings is
        // already showing: the Settings item stays clickable while the page is
        // up, and picking it twice recorded "settings" as the page to go back
        // to, so the back arrow did nothing.
        if let Some(stack) = self.imp().main_stack.borrow().as_ref() {
            if let Some(name) = stack.visible_child_name()
                && name != "settings"
            {
                self.imp().previous_page.replace(Some(name.to_string()));
            }
        }

        // Deselect sidebar nav since settings isn't a nav item
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            if let Some(nav_list) = sidebar.imp().nav_list.borrow().as_ref() {
                nav_list.unselect_all();
            }
        }

        // Not a section switch: whatever the user was reading has to be there
        // still when they come back out.
        self.show_page("settings");
    }

    /// Leave settings and return to the previous page
    fn leave_settings(&self) {
        let imp = self.imp();
        let previous = imp
            .previous_page
            .borrow()
            .clone()
            .unwrap_or("home".to_string());

        // Restore sidebar selection
        if let Some(sidebar) = imp.sidebar.borrow().as_ref() {
            let nav_item = match previous.as_str() {
                "home" => Some(crate::ui::sidebar::NavItem::Home),
                "mentions" => Some(crate::ui::sidebar::NavItem::Mentions),
                "activity" => Some(crate::ui::sidebar::NavItem::Activity),
                "chat" => Some(crate::ui::sidebar::NavItem::Chat),
                "profile" => Some(crate::ui::sidebar::NavItem::Profile),
                "likes" => Some(crate::ui::sidebar::NavItem::Likes),
                "bookmarks" => Some(crate::ui::sidebar::NavItem::Bookmarks),
                "search" => Some(crate::ui::sidebar::NavItem::Search),
                _ => Some(crate::ui::sidebar::NavItem::Home),
            };
            // Selecting a row programmatically does not emit `row-activated`,
            // which is what `connect_nav_changed` listens on, so this does not
            // loop back round into a section switch that would pop the stack.
            if let Some(item) = nav_item {
                sidebar.select_nav_item(item);
            }
        }

        // `show_page` here. Returning from settings is not a section switch,
        // so a thread or profile the user opened settings from has to survive
        // the trip; `switch_to_page` would unwind it.
        self.show_page(&previous);
    }

    /// Apply a font size setting by updating the dynamic CSS provider
    pub fn apply_font_size(&self, font_size: crate::state::FontSize) {
        let imp = self.imp();

        // Create CSS provider if it doesn't exist yet
        let provider = {
            let existing = imp.font_size_css_provider.borrow();
            if let Some(p) = existing.as_ref() {
                p.clone()
            } else {
                drop(existing);
                let p = gtk4::CssProvider::new();
                // Add to the default display so it applies globally
                gtk4::style_context_add_provider_for_display(
                    &gtk4::gdk::Display::default().expect("Could not get default display"),
                    &p,
                    gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
                );
                imp.font_size_css_provider.replace(Some(p.clone()));
                p
            }
        };

        let scale = font_size.scale_factor();

        // Scale only the text labels. Scaling the container causes horizontal
        // overflow from fixed-width child widgets such as the action buttons.
        let css = format!(
            r#"
.post-row label {{
    font-size: {body}em;
}}
.post-row .heading {{
    font-size: {heading}em;
    font-weight: 600;
}}
.post-row .caption,
.post-row .dim-label,
.post-row .action-count {{
    font-size: {caption}em;
}}
.activity-row label {{
    font-size: {body}em;
}}
.conversation-row label {{
    font-size: {body}em;
}}
.text-size-preview label {{
    font-size: {body}em;
}}
.text-size-preview .preview-heading {{
    font-size: {heading}em;
    font-weight: 600;
}}
.text-size-preview .preview-caption {{
    font-size: {caption}em;
}}
.text-size-preview .preview-body {{
    font-size: {body}em;
}}
.compose-text {{
    font-size: {body}em;
}}
"#,
            heading = 1.0 * scale,
            caption = 0.95 * scale,
            body = 1.0 * scale,
        );

        provider.load_from_string(&css);
    }

    /// Apply reduced motion setting by disabling GTK animations globally
    pub fn apply_reduce_motion(&self, enabled: bool) {
        // Disable GTK-level animations (NavigationView slides, stack transitions, etc.)
        if let Some(settings) = gtk4::Settings::default() {
            settings.set_gtk_enable_animations(!enabled);
        }

        // Also inject CSS to catch any CSS-driven animations/transitions
        let imp = self.imp();
        let provider = {
            let existing = imp.reduce_motion_css_provider.borrow();
            if let Some(p) = existing.as_ref() {
                p.clone()
            } else {
                drop(existing);
                let p = gtk4::CssProvider::new();
                gtk4::style_context_add_provider_for_display(
                    &gtk4::gdk::Display::default().expect("Could not get default display"),
                    &p,
                    gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
                );
                imp.reduce_motion_css_provider.replace(Some(p.clone()));
                p
            }
        };

        if enabled {
            provider.load_from_string("* { animation-duration: 0s; transition-duration: 0s; }");
        } else {
            provider.load_from_string("");
        }
    }

    /// Apply color scheme preference via Adwaita's StyleManager
    pub fn apply_color_scheme(&self, scheme: crate::state::ColorScheme) {
        let adw_scheme = match scheme {
            crate::state::ColorScheme::System => adw::ColorScheme::Default,
            crate::state::ColorScheme::Light => adw::ColorScheme::ForceLight,
            crate::state::ColorScheme::Dark => adw::ColorScheme::ForceDark,
        };
        adw::StyleManager::default().set_color_scheme(adw_scheme);
    }

    /// Set callback for when font size changes
    pub fn set_font_size_changed_callback<F: Fn(crate::state::FontSize) + 'static>(&self, f: F) {
        self.imp()
            .font_size_changed_callback
            .replace(Some(Box::new(f)));
    }
}

/// A row widget for displaying a mention/notification
mod mention_row {
    use super::*;
    use crate::atproto::Notification;
    use crate::ui::avatar_cache;

    mod imp {
        use super::*;
        use std::cell::RefCell;

        #[derive(Default)]
        pub struct MentionRow {
            pub avatar: RefCell<Option<adw::Avatar>>,
            pub name_label: RefCell<Option<gtk4::Label>>,
            pub handle_label: RefCell<Option<gtk4::Label>>,
            pub text_label: RefCell<Option<gtk4::Label>>,
            pub reason_label: RefCell<Option<gtk4::Label>>,
            pub time_label: RefCell<Option<gtk4::Label>>,
            pub profile_clicked_callback:
                RefCell<Option<Box<dyn Fn(&super::MentionRow) + 'static>>>,
            pub clicked_callback: RefCell<Option<Box<dyn Fn(&super::MentionRow) + 'static>>>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for MentionRow {
            const NAME: &'static str = "HangarMentionRow";
            type Type = super::MentionRow;
            type ParentType = gtk4::Box;
        }

        impl ObjectImpl for MentionRow {
            fn constructed(&self) {
                self.parent_constructed();
                let obj = self.obj();
                obj.setup_ui();
            }
        }

        impl WidgetImpl for MentionRow {}
        impl BoxImpl for MentionRow {}
    }

    glib::wrapper! {
        pub struct MentionRow(ObjectSubclass<imp::MentionRow>)
            @extends gtk4::Box, gtk4::Widget,
            @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Orientable;
    }

    impl MentionRow {
        pub fn new() -> Self {
            glib::Object::builder()
                .property("orientation", gtk4::Orientation::Vertical)
                .property("spacing", 0)
                .build()
        }

        fn setup_ui(&self) {
            self.add_css_class("mention-row");
            self.set_margin_start(12);
            self.set_margin_end(12);
            self.set_margin_top(8);
            self.set_margin_bottom(8);

            // Main content box (horizontal: avatar + content)
            let main_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);

            // Avatar with click gesture
            let avatar = adw::Avatar::new(48, None, true);
            avatar.set_cursor_from_name(Some("pointer"));
            let avatar_click = gtk4::GestureClick::new();
            let row_weak = self.downgrade();
            avatar_click.connect_released(move |gesture, _, _, _| {
                // Claim, or the row's own gesture also sees this release and one
                // click opens both the profile and the thread. Both bubble, so
                // the avatar gets the release first. Same rule as the quote card
                // in `post_row`.
                gesture.set_state(gtk4::EventSequenceState::Claimed);
                if let Some(row) = row_weak.upgrade() {
                    if let Some(cb) = row.imp().profile_clicked_callback.borrow().as_ref() {
                        cb(&row);
                    }
                }
            });
            avatar.add_controller(avatar_click);
            main_box.append(&avatar);

            // Content box (vertical: header + text)
            let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            content_box.set_hexpand(true);

            // Header: reason icon + name + handle + time
            let header_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);

            let reason_label = gtk4::Label::new(None);
            reason_label.add_css_class("dim-label");
            reason_label.add_css_class("caption");
            // Unrecognised reasons fall through to the raw string off the wire,
            // so this is not a closed set of short words we control.
            reason_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            header_box.append(&reason_label);

            let name_label = gtk4::Label::new(None);
            name_label.add_css_class("heading");
            name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            name_label.set_halign(gtk4::Align::Start);
            header_box.append(&name_label);

            let handle_label = gtk4::Label::new(None);
            handle_label.add_css_class("dim-label");
            handle_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            handle_label.set_halign(gtk4::Align::Start);
            header_box.append(&handle_label);

            let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            spacer.set_hexpand(true);
            header_box.append(&spacer);

            let time_label = gtk4::Label::new(None);
            time_label.add_css_class("dim-label");
            time_label.add_css_class("caption");
            header_box.append(&time_label);

            content_box.append(&header_box);

            // Text content
            let text_label = gtk4::Label::new(None);
            text_label.set_wrap(true);
            text_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
            text_label.set_xalign(0.0);
            text_label.set_max_width_chars(80);
            text_label.set_halign(gtk4::Align::Start);
            content_box.append(&text_label);

            main_box.append(&content_box);

            // Click gesture for the whole row
            let click = gtk4::GestureClick::new();
            let row_weak = self.downgrade();
            click.connect_released(move |_, _, _, _| {
                if let Some(row) = row_weak.upgrade() {
                    if let Some(cb) = row.imp().clicked_callback.borrow().as_ref() {
                        cb(&row);
                    }
                }
            });
            self.add_controller(click);

            self.append(&main_box);

            // Separator
            let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            sep.set_margin_top(8);
            self.append(&sep);

            let imp = self.imp();
            imp.avatar.replace(Some(avatar));
            imp.name_label.replace(Some(name_label));
            imp.handle_label.replace(Some(handle_label));
            imp.text_label.replace(Some(text_label));
            imp.reason_label.replace(Some(reason_label));
            imp.time_label.replace(Some(time_label));
        }

        pub fn bind(&self, notification: &Notification) {
            let imp = self.imp();

            // Avatar
            if let Some(avatar) = imp.avatar.borrow().as_ref() {
                let display_name = notification
                    .author
                    .display_name
                    .as_deref()
                    .unwrap_or(&notification.author.handle);
                avatar.set_text(Some(display_name));
                if let Some(url) = &notification.author.avatar {
                    avatar_cache::load_avatar(avatar.clone(), url.clone());
                }
            }

            // Name
            if let Some(label) = imp.name_label.borrow().as_ref() {
                let display_name = notification
                    .author
                    .display_name
                    .as_deref()
                    .unwrap_or(&notification.author.handle);
                label.set_text(display_name);
            }

            // Handle
            if let Some(label) = imp.handle_label.borrow().as_ref() {
                label.set_text(&format!("@{}", notification.author.handle));
            }

            // Reason indicator
            if let Some(label) = imp.reason_label.borrow().as_ref() {
                let reason_text = match notification.reason.as_str() {
                    "mention" => "mentioned you",
                    "reply" => "replied",
                    "quote" => "quoted you",
                    _ => &notification.reason,
                };
                label.set_text(reason_text);
            }

            // Text content
            if let Some(label) = imp.text_label.borrow().as_ref() {
                if let Some(post) = &notification.post {
                    label.set_text(&post.text);
                    label.set_visible(true);
                } else {
                    label.set_visible(false);
                }
            }

            // Time
            if let Some(label) = imp.time_label.borrow().as_ref() {
                label.set_text(&Self::format_relative_time(&notification.indexed_at));
            }
        }

        fn format_relative_time(indexed_at: &str) -> String {
            use chrono::{DateTime, Utc};

            let Ok(post_time) = DateTime::parse_from_rfc3339(indexed_at) else {
                return String::new();
            };

            let now = Utc::now();
            let post_utc = post_time.with_timezone(&Utc);
            let duration = now.signed_duration_since(post_utc);

            if duration.num_seconds() < 60 {
                "now".to_string()
            } else if duration.num_minutes() < 60 {
                format!("{}m", duration.num_minutes())
            } else if duration.num_hours() < 24 {
                format!("{}h", duration.num_hours())
            } else if duration.num_days() < 7 {
                format!("{}d", duration.num_days())
            } else {
                post_time.format("%b %d").to_string()
            }
        }

        pub fn connect_profile_clicked<F: Fn(&Self) + 'static>(&self, callback: F) {
            self.imp()
                .profile_clicked_callback
                .replace(Some(Box::new(callback)));
        }

        pub fn connect_clicked<F: Fn(&Self) + 'static>(&self, callback: F) {
            self.imp()
                .clicked_callback
                .replace(Some(Box::new(callback)));
        }

        /// Forget the row-click handler.
        ///
        /// `ListView` recycles rows, so a bind that leaves the handler alone
        /// inherits the previous occupant's. A notification with no post kept
        /// an earlier one's closure and opened that unrelated thread on click.
        pub fn clear_clicked(&self) {
            self.imp().clicked_callback.replace(None);
        }
    }

    impl Default for MentionRow {
        fn default() -> Self {
            Self::new()
        }
    }
}

use mention_row::MentionRow;

/// A row widget for displaying activity notifications (likes, follows, reposts, etc.)
mod activity_row {
    use super::*;
    use crate::atproto::Notification;
    use crate::ui::avatar_cache;

    mod imp {
        use super::*;
        use std::cell::RefCell;

        #[derive(Default)]
        pub struct ActivityRow {
            pub avatar: RefCell<Option<adw::Avatar>>,
            pub badge_icon: RefCell<Option<gtk4::Image>>,
            pub action_label: RefCell<Option<gtk4::Label>>,
            pub time_label: RefCell<Option<gtk4::Label>>,
            pub post_card: RefCell<Option<gtk4::Box>>,
            pub post_author_avatar: RefCell<Option<adw::Avatar>>,
            pub post_author_label: RefCell<Option<gtk4::Label>>,
            pub post_text_label: RefCell<Option<gtk4::Label>>,
            pub profile_clicked_callback:
                RefCell<Option<Box<dyn Fn(&super::ActivityRow) + 'static>>>,
            pub clicked_callback: RefCell<Option<Box<dyn Fn(&super::ActivityRow) + 'static>>>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for ActivityRow {
            const NAME: &'static str = "HangarActivityRow";
            type Type = super::ActivityRow;
            type ParentType = gtk4::Box;
        }

        impl ObjectImpl for ActivityRow {
            fn constructed(&self) {
                self.parent_constructed();
                let obj = self.obj();
                obj.setup_ui();
            }
        }

        impl WidgetImpl for ActivityRow {}
        impl BoxImpl for ActivityRow {}
    }

    glib::wrapper! {
        pub struct ActivityRow(ObjectSubclass<imp::ActivityRow>)
            @extends gtk4::Box, gtk4::Widget,
            @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Orientable;
    }

    impl ActivityRow {
        pub fn new() -> Self {
            glib::Object::builder()
                .property("orientation", gtk4::Orientation::Vertical)
                .property("spacing", 0)
                .build()
        }

        fn setup_ui(&self) {
            self.add_css_class("activity-row");
            self.set_margin_start(12);
            self.set_margin_end(12);
            self.set_margin_top(12);
            self.set_margin_bottom(12);

            // Main content box (horizontal: avatar + content)
            let main_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);

            // Avatar with badge overlay. Top-aligned: left to fill, a tall
            // row (embedded card) stretched the overlay and the badge sank
            // to the row's bottom, nowhere near the avatar.
            let avatar_overlay = gtk4::Overlay::new();
            avatar_overlay.set_valign(gtk4::Align::Start);
            let avatar = adw::Avatar::new(48, None, true);
            avatar_overlay.set_child(Some(&avatar));

            // Badge icon (heart for likes, person+ for follows, etc.)
            let badge_icon = gtk4::Image::new();
            badge_icon.set_pixel_size(20);
            badge_icon.set_halign(gtk4::Align::End);
            badge_icon.set_valign(gtk4::Align::End);
            badge_icon.add_css_class("activity-badge");
            avatar_overlay.add_overlay(&badge_icon);

            // Click gesture for avatar
            avatar_overlay.set_cursor_from_name(Some("pointer"));
            let avatar_click = gtk4::GestureClick::new();
            let row_weak = self.downgrade();
            avatar_click.connect_released(move |gesture, _, _, _| {
                // See `MentionRow::setup_ui`: without this claim the row's own
                // gesture answers the same release and the click both opens
                // the profile and opens the post.
                gesture.set_state(gtk4::EventSequenceState::Claimed);
                if let Some(row) = row_weak.upgrade() {
                    if let Some(cb) = row.imp().profile_clicked_callback.borrow().as_ref() {
                        cb(&row);
                    }
                }
            });
            avatar_overlay.add_controller(avatar_click);
            main_box.append(&avatar_overlay);

            // Content box (vertical: action text + embedded post card)
            let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            content_box.set_hexpand(true);

            // Header: action label + time
            let header_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);

            let action_label = gtk4::Label::new(None);
            action_label.set_wrap(true);
            action_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
            action_label.set_xalign(0.0);
            action_label.set_hexpand(true);
            action_label.set_halign(gtk4::Align::Start);
            header_box.append(&action_label);

            let time_label = gtk4::Label::new(None);
            time_label.add_css_class("dim-label");
            time_label.add_css_class("caption");
            time_label.set_valign(gtk4::Align::Start);
            header_box.append(&time_label);

            content_box.append(&header_box);

            // Embedded post card (for likes/reposts/quotes)
            let post_card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            post_card.add_css_class("card");
            post_card.set_margin_top(4);

            let card_inner = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            card_inner.set_margin_start(12);
            card_inner.set_margin_end(12);
            card_inner.set_margin_top(8);
            card_inner.set_margin_bottom(8);

            // Post author row
            let post_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            let post_author_avatar = adw::Avatar::new(24, None, true);
            post_header.append(&post_author_avatar);

            let post_author_label = gtk4::Label::new(None);
            post_author_label.add_css_class("caption");
            post_author_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            post_header.append(&post_author_label);

            // No date in the card: the row already carries the event time,
            // and a reply's card repeated it to the pixel.
            card_inner.append(&post_header);

            // Post text
            let post_text_label = gtk4::Label::new(None);
            post_text_label.set_wrap(true);
            post_text_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
            post_text_label.set_xalign(0.0);
            post_text_label.set_max_width_chars(80);
            post_text_label.set_halign(gtk4::Align::Start);
            card_inner.append(&post_text_label);

            post_card.append(&card_inner);
            post_card.set_visible(false); // Hidden by default
            content_box.append(&post_card);

            main_box.append(&content_box);

            // Click gesture for the whole row
            let click = gtk4::GestureClick::new();
            let row_weak = self.downgrade();
            click.connect_released(move |_, _, _, _| {
                if let Some(row) = row_weak.upgrade() {
                    if let Some(cb) = row.imp().clicked_callback.borrow().as_ref() {
                        cb(&row);
                    }
                }
            });
            self.add_controller(click);

            self.append(&main_box);

            // Separator
            let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            sep.set_margin_top(12);
            self.append(&sep);

            let imp = self.imp();
            imp.avatar.replace(Some(avatar));
            imp.badge_icon.replace(Some(badge_icon));
            imp.action_label.replace(Some(action_label));
            imp.time_label.replace(Some(time_label));
            imp.post_card.replace(Some(post_card));
            imp.post_author_avatar.replace(Some(post_author_avatar));
            imp.post_author_label.replace(Some(post_author_label));
            imp.post_text_label.replace(Some(post_text_label));
        }

        pub fn bind(&self, notification: &Notification) {
            let imp = self.imp();

            let display_name = notification
                .author
                .display_name
                .as_deref()
                .unwrap_or(&notification.author.handle);

            // Avatar
            if let Some(avatar) = imp.avatar.borrow().as_ref() {
                avatar.set_text(Some(display_name));
                if let Some(url) = &notification.author.avatar {
                    avatar_cache::load_avatar(avatar.clone(), url.clone());
                }
            }

            // Badge icon based on reason. The like badge matches the like
            // button; emblem-favorite went missing from newer icon themes
            // and rendered as the broken-image glyph.
            let icon_name = match notification.reason.as_str() {
                "like" => "emote-love-symbolic",
                "repost" => "media-playlist-repeat-symbolic",
                "follow" => "system-users-symbolic",
                "mention" => "chat-message-new-symbolic",
                "reply" => "mail-reply-sender-symbolic",
                "quote" => "edit-copy-symbolic",
                _ => "dialog-information-symbolic",
            };

            if let Some(badge) = imp.badge_icon.borrow().as_ref() {
                badge.set_icon_name(Some(icon_name));
                // Add color class based on type
                badge.remove_css_class("liked");
                badge.remove_css_class("reposted");
                badge.remove_css_class("followed");
                match notification.reason.as_str() {
                    "like" => badge.add_css_class("liked"),
                    "repost" => badge.add_css_class("reposted"),
                    "follow" => badge.add_css_class("followed"),
                    _ => {}
                }
            }

            // Action label, Pango markup so the name is bold
            if let Some(label) = imp.action_label.borrow().as_ref() {
                let action_suffix = match notification.reason.as_str() {
                    "like" => "liked your post",
                    "repost" => "reposted your post",
                    "follow" => "followed you",
                    "mention" => "mentioned you",
                    "reply" => "replied to your post",
                    "quote" => "quoted your post",
                    other => other,
                };
                // Escape display name for markup and make it bold
                let escaped_name = glib::markup_escape_text(display_name);
                let markup = format!("<b>{}</b> {}", escaped_name, action_suffix);
                label.set_use_markup(true);
                label.set_label(&markup);
            }

            // Time
            if let Some(label) = imp.time_label.borrow().as_ref() {
                label.set_text(&Self::format_relative_time(&notification.indexed_at));
            }

            // Show post card if there's a post
            if let Some(post) = &notification.post {
                if let Some(card) = imp.post_card.borrow().as_ref() {
                    card.set_visible(true);
                }

                // Post author avatar
                if let Some(avatar) = imp.post_author_avatar.borrow().as_ref() {
                    let post_author_name = post
                        .author
                        .display_name
                        .as_deref()
                        .unwrap_or(&post.author.handle);
                    avatar.set_text(Some(post_author_name));
                    if let Some(url) = &post.author.avatar {
                        avatar_cache::load_avatar(avatar.clone(), url.clone());
                    }
                }

                // Post author label
                if let Some(label) = imp.post_author_label.borrow().as_ref() {
                    let post_author_name = post
                        .author
                        .display_name
                        .as_deref()
                        .unwrap_or(&post.author.handle);
                    label.set_text(&format!("{} @{}", post_author_name, post.author.handle));
                }

                // Post text
                if let Some(label) = imp.post_text_label.borrow().as_ref() {
                    label.set_text(&post.text);
                }
            } else {
                if let Some(card) = imp.post_card.borrow().as_ref() {
                    card.set_visible(false);
                }
            }
        }

        fn format_relative_time(indexed_at: &str) -> String {
            use chrono::{DateTime, Utc};

            let Ok(post_time) = DateTime::parse_from_rfc3339(indexed_at) else {
                return String::new();
            };

            let now = Utc::now();
            let post_utc = post_time.with_timezone(&Utc);
            let duration = now.signed_duration_since(post_utc);

            if duration.num_seconds() < 60 {
                "now".to_string()
            } else if duration.num_minutes() < 60 {
                format!("{}m", duration.num_minutes())
            } else if duration.num_hours() < 24 {
                format!("{}h", duration.num_hours())
            } else if duration.num_days() < 7 {
                format!("{}d", duration.num_days())
            } else {
                post_time.format("%b %d").to_string()
            }
        }

        pub fn connect_profile_clicked<F: Fn(&Self) + 'static>(&self, callback: F) {
            self.imp()
                .profile_clicked_callback
                .replace(Some(Box::new(callback)));
        }

        pub fn connect_clicked<F: Fn(&Self) + 'static>(&self, callback: F) {
            self.imp()
                .clicked_callback
                .replace(Some(Box::new(callback)));
        }

        /// Forget the row-click handler. See [`super::MentionRow::clear_clicked`]:
        /// a follow notification, which has no post, otherwise inherits the
        /// handler of whatever was bound to this recycled row before it.
        pub fn clear_clicked(&self) {
            self.imp().clicked_callback.replace(None);
        }
    }

    impl Default for ActivityRow {
        fn default() -> Self {
            Self::new()
        }
    }
}

use activity_row::ActivityRow;

/// A row widget for displaying a chat conversation
mod conversation_row {
    use super::*;
    use crate::atproto::Conversation;
    use crate::ui::avatar_cache;

    mod imp {
        use super::*;
        use std::cell::RefCell;

        #[derive(Default)]
        pub struct ConversationRow {
            pub avatar: RefCell<Option<adw::Avatar>>,
            pub name_label: RefCell<Option<gtk4::Label>>,
            pub preview_label: RefCell<Option<gtk4::Label>>,
            pub time_label: RefCell<Option<gtk4::Label>>,
            pub unread_badge: RefCell<Option<gtk4::Label>>,
            pub clicked_callback: RefCell<Option<Box<dyn Fn(&super::ConversationRow) + 'static>>>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for ConversationRow {
            const NAME: &'static str = "HangarConversationRow";
            type Type = super::ConversationRow;
            type ParentType = gtk4::Box;
        }

        impl ObjectImpl for ConversationRow {
            fn constructed(&self) {
                self.parent_constructed();
                let obj = self.obj();
                obj.setup_ui();
            }
        }

        impl WidgetImpl for ConversationRow {}
        impl BoxImpl for ConversationRow {}
    }

    glib::wrapper! {
        pub struct ConversationRow(ObjectSubclass<imp::ConversationRow>)
            @extends gtk4::Box, gtk4::Widget,
            @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Orientable;
    }

    impl ConversationRow {
        pub fn new() -> Self {
            glib::Object::builder()
                .property("orientation", gtk4::Orientation::Vertical)
                .property("spacing", 0)
                .build()
        }

        fn setup_ui(&self) {
            self.add_css_class("conversation-row");
            self.set_margin_start(12);
            self.set_margin_end(12);
            self.set_margin_top(8);
            self.set_margin_bottom(8);

            // Main content box (horizontal: avatar + content)
            let main_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);

            // Avatar
            let avatar = adw::Avatar::new(48, None, true);
            main_box.append(&avatar);

            // Content box (vertical: name + preview)
            let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            content_box.set_hexpand(true);

            // Header row: name + time
            let header_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);

            let name_label = gtk4::Label::new(None);
            name_label.add_css_class("heading");
            name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            name_label.set_halign(gtk4::Align::Start);
            name_label.set_hexpand(true);
            header_box.append(&name_label);

            let time_label = gtk4::Label::new(None);
            time_label.add_css_class("dim-label");
            time_label.add_css_class("caption");
            header_box.append(&time_label);

            content_box.append(&header_box);

            // Preview row: message preview + unread badge
            let preview_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);

            let preview_label = gtk4::Label::new(None);
            preview_label.add_css_class("dim-label");
            preview_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            preview_label.set_halign(gtk4::Align::Start);
            preview_label.set_hexpand(true);
            preview_label.set_max_width_chars(50);
            preview_box.append(&preview_label);

            let unread_badge = gtk4::Label::new(None);
            unread_badge.add_css_class("accent");
            unread_badge.add_css_class("caption");
            unread_badge.set_visible(false);
            preview_box.append(&unread_badge);

            content_box.append(&preview_box);

            main_box.append(&content_box);

            // Click gesture
            let click = gtk4::GestureClick::new();
            let row_weak = self.downgrade();
            click.connect_released(move |_, _, _, _| {
                if let Some(row) = row_weak.upgrade() {
                    if let Some(cb) = row.imp().clicked_callback.borrow().as_ref() {
                        cb(&row);
                    }
                }
            });
            self.add_controller(click);

            self.append(&main_box);

            // Separator
            let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            sep.set_margin_top(8);
            self.append(&sep);

            let imp = self.imp();
            imp.avatar.replace(Some(avatar));
            imp.name_label.replace(Some(name_label));
            imp.preview_label.replace(Some(preview_label));
            imp.time_label.replace(Some(time_label));
            imp.unread_badge.replace(Some(unread_badge));
        }

        pub fn bind(&self, conversation: &Conversation, my_did: Option<&str>) {
            let imp = self.imp();

            // The other participants, with ourselves filtered out
            let other_member = conversation
                .members
                .iter()
                .find(|m| my_did.is_none_or(|did| m.did != did))
                .or_else(|| conversation.members.first());

            // Avatar
            if let Some(avatar) = imp.avatar.borrow().as_ref() {
                if let Some(member) = other_member {
                    let display_name = member.display_name.as_deref().unwrap_or(&member.handle);
                    avatar.set_text(Some(display_name));
                    if let Some(url) = &member.avatar {
                        avatar_cache::load_avatar(avatar.clone(), url.clone());
                    }
                } else {
                    avatar.set_text(Some("?"));
                }
            }

            // Name
            if let Some(label) = imp.name_label.borrow().as_ref() {
                if let Some(member) = other_member {
                    let display_name = member.display_name.as_deref().unwrap_or(&member.handle);
                    label.set_text(display_name);
                } else {
                    label.set_text("Unknown");
                }
            }

            // Preview (last message)
            if let Some(label) = imp.preview_label.borrow().as_ref() {
                if let Some(last_msg) = &conversation.last_message {
                    label.set_text(&last_msg.text);
                    label.set_visible(true);
                } else {
                    label.set_visible(false);
                }
            }

            // Time
            if let Some(label) = imp.time_label.borrow().as_ref() {
                if let Some(last_msg) = &conversation.last_message {
                    label.set_text(&Self::format_relative_time(&last_msg.sent_at));
                } else {
                    label.set_text("");
                }
            }

            // Unread badge
            if let Some(badge) = imp.unread_badge.borrow().as_ref() {
                if conversation.unread_count > 0 {
                    if conversation.unread_count > 99 {
                        badge.set_text("99+");
                    } else {
                        badge.set_text(&conversation.unread_count.to_string());
                    }
                    badge.set_visible(true);
                } else {
                    badge.set_visible(false);
                }
            }
        }

        fn format_relative_time(sent_at: &str) -> String {
            use chrono::{DateTime, Utc};

            let Ok(time) = DateTime::parse_from_rfc3339(sent_at) else {
                return String::new();
            };

            let now = Utc::now();
            let time_utc = time.with_timezone(&Utc);
            let duration = now.signed_duration_since(time_utc);

            if duration.num_seconds() < 60 {
                "now".to_string()
            } else if duration.num_minutes() < 60 {
                format!("{}m", duration.num_minutes())
            } else if duration.num_hours() < 24 {
                format!("{}h", duration.num_hours())
            } else if duration.num_days() < 7 {
                format!("{}d", duration.num_days())
            } else {
                time.format("%b %d").to_string()
            }
        }

        pub fn connect_clicked<F: Fn(&Self) + 'static>(&self, callback: F) {
            self.imp()
                .clicked_callback
                .replace(Some(Box::new(callback)));
        }
    }

    impl Default for ConversationRow {
        fn default() -> Self {
            Self::new()
        }
    }
}

use conversation_row::ConversationRow;

#[cfg(test)]
mod tests {
    use super::activity_row::ActivityRow;
    use super::mention_row::MentionRow;
    use super::*;
    use crate::atproto::{Notification, Profile};
    use std::cell::Cell;
    use std::rc::Rc;

    fn notification(with_post: bool) -> Notification {
        let author = Profile::minimal(
            "did:plc:test".into(),
            "someone.bsky.social".into(),
            Some("Someone".into()),
            None,
        );
        Notification {
            uri: "at://did:plc:test/app.bsky.feed.post/n".into(),
            cid: "cid".into(),
            reason: if with_post { "mention" } else { "follow" }.into(),
            indexed_at: "2026-01-01T00:00:00Z".into(),
            is_read: false,
            post: with_post.then(|| Post {
                uri: "at://did:plc:test/app.bsky.feed.post/p".into(),
                cid: "cid".into(),
                author: author.clone(),
                text: "hello".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                indexed_at: "2026-01-01T00:00:00Z".into(),
                like_count: None,
                repost_count: None,
                reply_count: None,
                embed: None,
                viewer_like: None,
                viewer_repost: None,
                viewer_bookmarked: None,
                repost_reason: None,
                reply_context: None,
            }),
            author,
        }
    }

    /// The smallest `Post` that binds; it has no embed and no counts.
    fn a_post(id: &str) -> Post {
        Post {
            uri: format!("at://did:plc:test/app.bsky.feed.post/{id}"),
            cid: "cid".into(),
            author: Profile::minimal(
                "did:plc:test".into(),
                "someone.bsky.social".into(),
                Some("Someone".into()),
                None,
            ),
            text: format!("post {id}"),
            created_at: "2026-01-01T00:00:00Z".into(),
            indexed_at: "2026-01-01T00:00:00Z".into(),
            like_count: None,
            repost_count: None,
            reply_count: None,
            embed: None,
            viewer_like: None,
            viewer_repost: None,
            viewer_bookmarked: None,
            repost_reason: None,
            reply_context: None,
        }
    }

    /// Every click gesture on `widget`, as its propagation phase.
    fn click_phases(widget: &gtk4::Widget) -> Vec<gtk4::PropagationPhase> {
        let list = widget.observe_controllers();
        (0..list.n_items())
            .filter_map(|i| list.item(i))
            .filter_map(|obj| obj.downcast::<gtk4::EventController>().ok())
            .filter(|c| c.type_().name() == "GtkGestureClick")
            .map(|c| c.propagation_phase())
            .collect()
    }

    fn walk(widget: &gtk4::Widget, depth: usize, out: &mut Vec<(usize, gtk4::Widget)>) {
        out.push((depth, widget.clone()));
        let mut child = widget.first_child();
        while let Some(c) = child {
            walk(&c, depth + 1, out);
            child = c.next_sibling();
        }
    }

    /// One click on a notification avatar must not also open the thread.
    ///
    /// Two bubbling gestures, avatar and row. Bubble is walked
    /// descendants-first, so the avatar's is reached first and is the one that
    /// claims. The check is structural: the avatar's gesture must sit strictly
    /// deeper and nothing between may capture. Whether `set_state(Claimed)` is
    /// called needs synthesised input, which GTK4 does not expose.
    #[test]
    fn a_notification_avatar_does_not_also_trigger_the_row() {
        crate::ui::with_gtk(a_notification_avatar_does_not_also_trigger_the_row_body);
    }

    fn a_notification_avatar_does_not_also_trigger_the_row_body() {
        for row in [
            MentionRow::new().upcast::<gtk4::Widget>(),
            ActivityRow::new().upcast::<gtk4::Widget>(),
        ] {
            let name = row.type_().name().to_string();

            assert_eq!(
                click_phases(&row),
                vec![gtk4::PropagationPhase::Bubble],
                "{name}: the row's own gesture must bubble, or it would answer \
                 the release before anything inside it"
            );

            let mut widgets = Vec::new();
            walk(&row, 0, &mut widgets);
            let inner: Vec<_> = widgets
                .iter()
                .filter(|(depth, w)| *depth > 0 && !click_phases(w).is_empty())
                .collect();
            assert_eq!(
                inner.len(),
                1,
                "{name}: exactly one click target inside the row (the avatar)"
            );
            assert_eq!(
                click_phases(&inner[0].1),
                vec![gtk4::PropagationPhase::Bubble],
                "{name}: the avatar's gesture bubbles too, so the deeper one is \
                 reached first and is the one that gets to claim"
            );
            assert!(
                inner[0].1.cursor().is_some(),
                "{name}: the avatar is clickable, so it says so"
            );
        }
    }

    /// A recycled row must not inherit the previous notification's handler.
    ///
    /// `ListView` reuses row widgets, and the bind path only installed the
    /// row-click handler when the notification had a post. A follow, which has
    /// none, therefore kept whatever some earlier notification had left behind
    /// and opened that unrelated thread.
    #[test]
    fn rebinding_a_notification_row_forgets_the_previous_post() {
        crate::ui::with_gtk(rebinding_a_notification_row_forgets_the_previous_post_body);
    }

    fn rebinding_a_notification_row_forgets_the_previous_post_body() {
        let fired = Rc::new(Cell::new(0));

        let row = MentionRow::new();
        row.bind(&notification(true));
        let count = fired.clone();
        row.connect_clicked(move |_| count.set(count.get() + 1));
        assert!(row.imp().clicked_callback.borrow().is_some());

        // Rebound to a notification with no post, the way the factory does it.
        row.bind(&notification(false));
        row.clear_clicked();
        assert!(
            row.imp().clicked_callback.borrow().is_none(),
            "the previous notification's post must not still be clickable"
        );
        assert_eq!(fired.get(), 0);

        let row = ActivityRow::new();
        row.bind(&notification(true));
        row.connect_clicked(|_| {});
        row.bind(&notification(false));
        row.clear_clicked();
        assert!(row.imp().clicked_callback.borrow().is_none());
    }

    /// A closed window must actually drop.
    ///
    /// window -> list view -> factory -> `bind` closure is a cycle if the
    /// closure holds the window strongly, and then `VideoDirector::drop` never
    /// runs and its election source outlives the window.
    #[test]
    fn a_closed_window_takes_its_video_director_with_it() {
        crate::ui::with_gtk(a_closed_window_takes_its_video_director_with_it_body);
    }

    fn a_closed_window_takes_its_video_director_with_it_body() {
        // No application: `setup_ui` runs from `constructed` either way, and
        // this keeps the only reference to the window in this function.
        let window: HangarWindow = glib::Object::builder().build();
        assert!(
            crate::ui::inline_video::director().is_some(),
            "the window did not build a director, so this proves nothing"
        );

        let weak = window.downgrade();
        window.destroy();
        drop(window);

        assert!(
            weak.upgrade().is_none(),
            "a closed window was still alive: something the window owns is \
             holding it strongly"
        );
        assert!(
            crate::ui::inline_video::director().is_none(),
            "the director outlived its window, so its election source did too"
        );
    }

    /// The own profile page's chain, and where its header lives.
    ///
    /// Two things, neither visible from outside once the page is built:
    ///
    /// 1. the `GtkListView` is the `GtkScrolledWindow`'s direct child. The page
    ///    used to be scroller, box, overlay, list, and the scroller cannot
    ///    forward scrolling through either of the middle two, so the list was
    ///    allocated its full natural height and stopped virtualizing.
    /// 2. the banner/avatar/bio block is row 0 of the list, which is what keeps
    ///    it scrolling with the posts. `profile_page_model` stays the posts
    ///    store, so post n is row n + 1 and `set_profile_posts` never has to
    ///    know about the header.
    #[test]
    fn the_own_profile_header_is_the_first_row_of_a_directly_scrollable_list() {
        crate::ui::with_gtk(
            the_own_profile_header_is_the_first_row_of_a_directly_scrollable_list_body,
        );
    }

    fn the_own_profile_header_is_the_first_row_of_a_directly_scrollable_list_body() {
        let window: HangarWindow = glib::Object::builder().build();

        let scrolled = window
            .imp()
            .profile_page_scrolled
            .borrow()
            .clone()
            .expect("the own profile page built a scroller");

        // 1. Nothing between the scroller and the list.
        let child = scrolled.child().expect("the scroller has a child");
        let list = child.downcast::<gtk4::ListView>().unwrap_or_else(|w| {
            panic!(
                "the own profile scroller's direct child is a {} — anything but the \
                 GtkListView here stops the list virtualizing",
                w.type_().name()
            )
        });

        // The posts store is what the load path appends to. Starts empty.
        let posts = window
            .imp()
            .profile_page_model
            .borrow()
            .clone()
            .expect("the own profile page built a posts model");
        assert_eq!(posts.n_items(), 0, "the posts store did not start empty");

        let rows = list.model().expect("the list has a model");
        assert_eq!(
            rows.n_items(),
            1,
            "with no posts the list should still hold exactly one row: the header"
        );

        // 2. Row 0 is the header marker, and posts land after it.
        assert!(
            rows.item(0).and_downcast::<gtk4::StringObject>().is_some(),
            "row 0 is not the header marker, so the page header is not in the list"
        );

        window.set_profile_posts(vec![a_post("one"), a_post("two"), a_post("three")]);
        assert_eq!(
            posts.n_items(),
            3,
            "set_profile_posts did not fill the posts store"
        );
        assert_eq!(
            rows.n_items(),
            4,
            "the flattened model should be the header plus three posts"
        );
        assert!(
            rows.item(0).and_downcast::<gtk4::StringObject>().is_some(),
            "set_profile_posts clobbered the header row"
        );
        assert!(
            rows.item(1).and_downcast::<PostObject>().is_some(),
            "the first post is not directly after the header"
        );

        // Replacing the posts must not take the header with them.
        window.set_profile_posts(vec![a_post("only")]);
        assert_eq!(
            rows.n_items(),
            2,
            "a reload left the model the wrong length"
        );
        assert!(
            rows.item(0).and_downcast::<gtk4::StringObject>().is_some(),
            "reloading the profile removed the header row"
        );

        window.destroy();
    }

    // ---------------------------------------------------------------------
    // The scroller-to-list chain, on the pages this app actually builds
    // ---------------------------------------------------------------------

    /// The first `GtkListView` at or under `widget`.
    fn find_list_view(widget: &gtk4::Widget) -> Option<gtk4::ListView> {
        if let Ok(list) = widget.clone().downcast::<gtk4::ListView>() {
            return Some(list);
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = find_list_view(&current) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }

    /// The first `GtkScrolledWindow` at or under `widget`.
    fn find_scrolled_window(widget: &gtk4::Widget) -> Option<gtk4::ScrolledWindow> {
        if let Ok(scrolled) = widget.clone().downcast::<gtk4::ScrolledWindow>() {
            return Some(scrolled);
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = find_scrolled_window(&current) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }

    /// Assert that `scrolled` can drive the list underneath it.
    ///
    /// Two checks. First the shape: no `GtkViewport` between the scroller and
    /// the list. `GtkScrolledWindow` interposes one for a child that is not a
    /// `GtkScrollable`, and a viewport allocates its child the child's natural
    /// height, so the list is sized as though it were fully on screen.
    ///
    /// Then the mechanism: the list's `vadjustment` has to be the scroller's
    /// own `GtkAdjustment` object. `AdwClampScrollable` forwards its
    /// adjustments to its child, so the one object reaches the list. With a
    /// viewport interposed the viewport takes the scroller's adjustment and the
    /// list keeps its default, and scrolling never reaches it.
    ///
    /// `child.is::<gtk4::Scrollable>()` would prove nothing here, since
    /// `GtkViewport` is itself a `GtkScrollable`. Measured on this window: with
    /// `AdwClampScrollable` the chain is scroller, clamp, list and the two
    /// adjustments are one pointer; with `AdwClamp` it is scroller, viewport,
    /// clamp, list and they are two.
    fn assert_scroller_drives_its_list(page: &str, scrolled: &gtk4::ScrolledWindow) {
        use gtk4::prelude::ScrollableExt;

        let child = scrolled
            .child()
            .unwrap_or_else(|| panic!("{page}: the scroller has no child at all"));

        assert!(
            !child.is::<gtk4::Viewport>(),
            "{page}: the scroller's direct child is a GtkViewport, which GTK only \
             interposes when it is handed a child that is not a GtkScrollable. \
             An AdwClamp does that; an AdwClampScrollable does not. The viewport \
             allocates the list its full natural height, so the list believes it \
             is entirely on screen and stops recycling rows — the feed goes blank \
             a couple of hundred posts in and every realized row holds its video \
             slot for the session."
        );

        let list = find_list_view(&child)
            .unwrap_or_else(|| panic!("{page}: no GtkListView anywhere under the scroller"));

        let list_adjustment = list
            .vadjustment()
            .unwrap_or_else(|| panic!("{page}: the list has no vadjustment"));

        assert!(
            list_adjustment == scrolled.vadjustment(),
            "{page}: the list's vadjustment is not the scroller's — they are two \
             different GtkAdjustment objects, so scrolling the page never reaches \
             the list and the list never learns which rows to realize. This is \
             what a non-scrollable widget between the scroller and the list does; \
             the scroller's child here is a {}.",
            child.type_().name()
        );
    }

    /// Every feed page this window builds, and the one it builds on demand.
    ///
    /// The `AdwClamp` to `AdwClampScrollable` swaps only count at the
    /// call sites, and a clamp built inside a test says nothing about what
    /// `window.rs` uses. So this walks the trees the app constructs: eight
    /// feeds off the window's own fields, plus the other-user profile page,
    /// which is built per profile. Reverting any one of the nine turns this
    /// red on that page's name.
    ///
    /// The pages that keep a plain `AdwClamp` are covered by
    /// [`the_thread_page_keeps_the_viewport_its_box_of_rows_needs`].
    #[test]
    fn every_feed_the_app_builds_hands_its_list_straight_to_the_scroller() {
        crate::ui::with_gtk(every_feed_the_app_builds_hands_its_list_straight_to_the_scroller_body);
    }

    fn every_feed_the_app_builds_hands_its_list_straight_to_the_scroller_body() {
        let window: HangarWindow = glib::Object::builder().build();
        let imp = window.imp();

        let mut feeds: Vec<(&str, gtk4::ScrolledWindow)> = Vec::new();
        for (page, cell) in [
            ("timeline", &imp.scrolled_window),
            ("mentions", &imp.mentions_scrolled_window),
            ("activity", &imp.activity_scrolled_window),
            ("chat", &imp.chat_scrolled_window),
            ("likes", &imp.likes_scrolled_window),
            ("bookmarks", &imp.bookmarks_scrolled_window),
            ("search", &imp.search_scrolled_window),
            ("search people", &imp.search_people_scrolled_window),
        ] {
            let scrolled = cell
                .borrow()
                .clone()
                .unwrap_or_else(|| panic!("the {page} page did not build a scroller"));
            feeds.push((page, scrolled));
        }

        // The other-user profile page is built per profile, so there is no
        // field to read. Build one and walk it.
        let profile = Profile::minimal(
            "did:plc:other".into(),
            "other.bsky.social".into(),
            Some("Other".into()),
            None,
        );
        let profile_page = window.build_profile_page(&profile, vec![a_post("one")], None);
        let profile_content = profile_page
            .child()
            .expect("the other-user profile page has content");
        let profile_scrolled = find_scrolled_window(&profile_content)
            .expect("the other-user profile page has a scroller");
        feeds.push(("other-user profile", profile_scrolled));

        assert_eq!(
            feeds.len(),
            9,
            "this test is meant to cover all nine virtualized feeds"
        );

        for (page, scrolled) in &feeds {
            assert_scroller_drives_its_list(page, scrolled);
        }

        // The own profile page has no clamp; its list is the scroller's direct
        // child. The same two properties still have to hold.
        let own_profile = imp
            .profile_page_scrolled
            .borrow()
            .clone()
            .expect("the own profile page built a scroller");
        assert_scroller_drives_its_list("own profile", &own_profile);

        window.destroy();
    }

    /// The thread page keeps its `GtkViewport`.
    ///
    /// Its content is a hand-built `GtkBox` of `PostRow`s, so there is nothing
    /// to virtualize and nothing to forward adjustments to. An
    /// `AdwClampScrollable` would bind its adjustments onto a `GtkBox` that has
    /// no such properties and the page would stop scrolling.
    ///
    /// Asserted so the exception stays deliberate. If the thread page ever
    /// becomes a list, this is the test that says so.
    #[test]
    fn the_thread_page_keeps_the_viewport_its_box_of_rows_needs() {
        crate::ui::with_gtk(the_thread_page_keeps_the_viewport_its_box_of_rows_needs_body);
    }

    fn the_thread_page_keeps_the_viewport_its_box_of_rows_needs_body() {
        let window: HangarWindow = glib::Object::builder().build();

        let page = window.build_thread_page(&a_post("main"), vec![a_post("reply")]);
        let content = page.child().expect("the thread page has content");
        let scrolled = find_scrolled_window(&content).expect("the thread page has a scroller");
        let child = scrolled.child().expect("the thread scroller has a child");

        assert!(
            child.is::<gtk4::Viewport>(),
            "the thread page's scroller child is a {} rather than the GtkViewport \
             its GtkBox of rows needs. An AdwClampScrollable here binds its \
             adjustments onto a widget that has none and the page stops scrolling; \
             if this page has become a GtkListView, this test should be replaced \
             by an entry in the feed table above, not deleted.",
            child.type_().name()
        );
        assert!(
            find_list_view(&child).is_none(),
            "the thread page has become a GtkListView, so it should be virtualized \
             and covered by the feed table above rather than kept on a viewport"
        );

        window.destroy();
    }

    /// Both profile pages linkify the bio, and hostile text in it must not
    /// blank the label.
    #[test]
    fn profile_bios_come_out_linkified() {
        crate::ui::with_gtk(profile_bios_come_out_linkified_body);
    }

    fn profile_bios_come_out_linkified_body() {
        let bio = "hi @user.bsky.social & <you>, see https://example.com";

        let window: HangarWindow = glib::Object::builder().build();

        let mut profile = Profile::minimal(
            "did:plc:bio".into(),
            "bio.bsky.social".into(),
            Some("Bio".into()),
            None,
        );
        profile.description = Some(bio.into());

        // The own profile header. The app updates it twice per login, cache
        // then network, so run it twice here.
        window.update_profile_header(&profile);
        window.update_profile_header(&profile);
        let label = window
            .imp()
            .profile_bio_label
            .borrow()
            .clone()
            .expect("the profile header built a bio label");
        assert_eq!(
            label.text().as_str(),
            bio,
            "the bio must survive `&` and `<`"
        );
        assert!(
            label.uses_markup(),
            "the bio fell back to plain text; its links are dead"
        );

        // The pushed profile page.
        let page = window.build_profile_page(&profile, vec![a_post("one")], None);
        let content = page.child().expect("the profile page has content");
        let mut widgets = Vec::new();
        walk(&content, 0, &mut widgets);
        let bio_label = widgets
            .iter()
            .filter_map(|(_, w)| w.downcast_ref::<gtk4::Label>())
            .find(|l| l.text().as_str() == bio)
            .expect("the pushed profile page shows the bio");
        assert!(
            bio_label.uses_markup(),
            "the pushed page's bio fell back to plain text"
        );

        window.destroy();
    }

    /// People results land in their own model, replace on a new search, and
    /// clear together with the post results.
    #[test]
    fn search_people_results_follow_the_search_lifecycle() {
        crate::ui::with_gtk(search_people_results_follow_the_search_lifecycle_body);
    }

    fn search_people_results_follow_the_search_lifecycle_body() {
        let window: HangarWindow = glib::Object::builder().build();
        let imp = window.imp();

        let someone = |handle: &str| {
            Profile::minimal(
                format!("did:plc:{handle}"),
                format!("{handle}.bsky.social"),
                None,
                None,
            )
        };

        window.set_search_people_results(vec![someone("a"), someone("b")]);
        window.append_search_people_results(vec![someone("c")]);

        let model = imp
            .search_people_model
            .borrow()
            .clone()
            .expect("the search page built a people model");
        assert_eq!(model.n_items(), 3);

        window.set_search_people_results(vec![someone("d")]);
        assert_eq!(model.n_items(), 1, "a new search replaces the people list");

        window.clear_search_results();
        assert_eq!(model.n_items(), 0);
        let posts_model = imp
            .search_model
            .borrow()
            .clone()
            .expect("the search page built a posts model");
        assert_eq!(posts_model.n_items(), 0);

        window.destroy();
    }

    /// The Saved page swaps between its list and its empty state as posts
    /// come and go, including when unsaving empties the list in place.
    #[test]
    fn the_saved_page_swaps_between_list_and_empty_state() {
        crate::ui::with_gtk(the_saved_page_swaps_between_list_and_empty_state_body);
    }

    fn the_saved_page_swaps_between_list_and_empty_state_body() {
        let window: HangarWindow = glib::Object::builder().build();
        let imp = window.imp();

        let empty_state = imp
            .bookmarks_empty_state
            .borrow()
            .clone()
            .expect("the Saved page built an empty state");
        let overlay = imp
            .bookmarks_overlay
            .borrow()
            .clone()
            .expect("the Saved page built its list overlay");
        let model = imp
            .bookmarks_model
            .borrow()
            .clone()
            .expect("the Saved page built a model");

        // Nothing saved: the empty state speaks, the list stands down.
        window.set_bookmarks(vec![]);
        assert!(empty_state.get_visible());
        assert!(!overlay.get_visible());

        window.set_bookmarks(vec![a_post("first"), a_post("second")]);
        window.append_bookmarks(vec![a_post("third")]);
        assert_eq!(model.n_items(), 3);
        assert!(!empty_state.get_visible());
        assert!(overlay.get_visible());

        // Unsaving takes rows out one at a time; the last one leaving brings
        // the empty state back.
        window.remove_bookmark(&a_post("third").uri);
        window.remove_bookmark(&a_post("first").uri);
        assert_eq!(model.n_items(), 1);
        assert!(
            !empty_state.get_visible(),
            "one post still saved, the list stays"
        );
        window.remove_bookmark(&a_post("second").uri);
        assert_eq!(model.n_items(), 0);
        assert!(empty_state.get_visible());
        assert!(!overlay.get_visible());

        window.destroy();
    }

    /// Deleting a post has to take it out of everything showing it.
    ///
    /// The stored models, a pushed profile page whose model nothing else
    /// holds, and a thread page whose rows sit in a plain box. A thread page
    /// rooted at the deleted post pops entirely. Everything not deleted has
    /// to survive, the own profile header row included.
    #[test]
    fn remove_post_takes_the_post_out_of_every_list_showing_it() {
        crate::ui::with_gtk(remove_post_takes_the_post_out_of_every_list_showing_it_body);
    }

    fn remove_post_takes_the_post_out_of_every_list_showing_it_body() {
        let window: HangarWindow = glib::Object::builder().build();
        let imp = window.imp();

        window.set_posts(vec![a_post("doomed"), a_post("kept")]);
        window.set_likes(vec![a_post("doomed")]);
        window.set_bookmarks(vec![a_post("doomed"), a_post("kept")]);
        window.set_search_results(vec![a_post("kept"), a_post("doomed")]);
        window.set_profile_posts(vec![a_post("doomed"), a_post("kept")]);

        let profile = Profile::minimal(
            "did:plc:other".into(),
            "other.bsky.social".into(),
            Some("Other".into()),
            None,
        );
        window.push_profile_page(&profile, vec![a_post("doomed"), a_post("kept")], None);
        // A thread rooted elsewhere that shows the doomed post as a parent.
        window.push_thread_page(&a_post("kept"), vec![a_post("doomed"), a_post("kept")]);
        // The thread rooted at the doomed post itself, on top of it.
        window.push_thread_page(&a_post("doomed"), vec![a_post("doomed"), a_post("kept")]);

        let doomed = a_post("doomed").uri;
        window.remove_post(&doomed);

        // Let the pop settle; the unmapped view may finish through idles.
        let ctx = glib::MainContext::default();
        while ctx.iteration(false) {}

        let uris = |model: &gio::ListStore| -> Vec<String> {
            (0..model.n_items())
                .filter_map(|i| model.item(i).and_downcast::<PostObject>())
                .filter_map(|o| o.post())
                .map(|p| p.uri)
                .collect()
        };
        for (page, cell) in [
            ("timeline", &imp.timeline_model),
            ("likes", &imp.likes_model),
            ("bookmarks", &imp.bookmarks_model),
            ("search", &imp.search_model),
            ("own profile", &imp.profile_page_model),
        ] {
            let model = cell
                .borrow()
                .clone()
                .unwrap_or_else(|| panic!("the {page} page has a model"));
            assert!(
                !uris(&model).contains(&doomed),
                "{page}: the deleted post is still in the model"
            );
        }
        assert_eq!(
            uris(&imp.timeline_model.borrow().clone().unwrap()),
            vec![a_post("kept").uri],
            "the timeline lost more than the deleted post"
        );

        // The own profile page keeps its header row.
        let own_list = find_list_view(
            &imp.profile_page_scrolled
                .borrow()
                .clone()
                .expect("own profile scroller")
                .upcast::<gtk4::Widget>(),
        )
        .expect("own profile list");
        assert!(
            own_list
                .model()
                .and_then(|m| m.item(0))
                .and_downcast::<gtk4::StringObject>()
                .is_some(),
            "removing a post must not take the profile header row with it"
        );

        // The pushed profile page's own model.
        let nav_view = imp
            .home_nav_view
            .borrow()
            .clone()
            .expect("the home section has a navigation view");
        let profile_page = nav_view
            .find_page("profile:did:plc:other")
            .expect("the pushed profile page is in the stack");
        let profile_list = find_list_view(&profile_page.clone().upcast::<gtk4::Widget>())
            .expect("the pushed profile page has a list");
        // Header marker store first, posts store second; see
        // `build_profile_page`.
        let profile_model = profile_list
            .model()
            .and_then(|m| m.downcast::<gtk4::NoSelection>().ok())
            .and_then(|ns| ns.model())
            .and_downcast::<gtk4::FlattenListModel>()
            .and_then(|flat| flat.model())
            .and_then(|sections| sections.item(1))
            .and_downcast::<gio::ListStore>()
            .expect("the pushed profile list flattens a header and a post store");
        assert_eq!(
            uris(&profile_model),
            vec![a_post("kept").uri],
            "the pushed profile page kept the wrong posts"
        );

        // The thread page rooted at the deleted post closes.
        assert!(
            nav_view.find_page(&format!("thread:{doomed}")).is_none(),
            "deleting the root of an open thread must pop its page"
        );

        // A thread rooted elsewhere stays; the deleted row leaves its box.
        let thread_page = nav_view
            .find_page(&format!("thread:{}", a_post("kept").uri))
            .expect("the surviving thread page is in the stack");
        let mut widgets = Vec::new();
        walk(
            &thread_page.clone().upcast::<gtk4::Widget>(),
            0,
            &mut widgets,
        );
        let thread_uris: Vec<String> = widgets
            .iter()
            .filter_map(|(_, w)| w.downcast_ref::<PostRow>())
            .filter_map(|row| row.post_uri())
            .collect();
        assert_eq!(
            thread_uris,
            vec![a_post("kept").uri],
            "the thread page should hold exactly the surviving post"
        );

        window.destroy();
    }

    /// Follow lists tag by account and side, and revisiting one pops back
    /// to it instead of stacking a second copy.
    #[test]
    fn follow_list_pages_tag_by_account_and_side() {
        crate::ui::with_gtk(follow_list_pages_tag_by_account_and_side_body);
    }

    fn follow_list_pages_tag_by_account_and_side_body() {
        let window: HangarWindow = glib::Object::builder().build();

        let alice = Profile::minimal(
            "did:plc:alice".into(),
            "alice.bsky.social".into(),
            None,
            None,
        );
        let bob = Profile::minimal("did:plc:bob".into(), "bob.bsky.social".into(), None, None);

        assert!(matches!(
            window.push_follow_list_page(&alice, FollowListKind::Followers),
            Some(FollowListPush::Pushed(_))
        ));
        assert!(
            matches!(
                window.push_follow_list_page(&alice, FollowListKind::Following),
                Some(FollowListPush::Pushed(_))
            ),
            "the two sides for one account are separate pages"
        );
        assert!(matches!(
            window.push_follow_list_page(&bob, FollowListKind::Followers),
            Some(FollowListPush::Pushed(_))
        ));

        let nav_view = window
            .imp()
            .home_nav_view
            .borrow()
            .clone()
            .expect("the home section has a navigation view");
        for tag in [
            "followers:did:plc:alice",
            "following:did:plc:alice",
            "followers:did:plc:bob",
        ] {
            assert!(nav_view.find_page(tag).is_some(), "{tag} is in the stack");
        }

        assert!(
            matches!(
                window.push_follow_list_page(&alice, FollowListKind::Followers),
                Some(FollowListPush::PoppedBack(_))
            ),
            "an already-open list is popped back to instead of repushed"
        );
        // Let the pop settle; the unmapped views may finish through idles.
        let ctx = glib::MainContext::default();
        while ctx.iteration(false) {}

        assert_eq!(
            nav_view.visible_page().and_then(|p| p.tag()).as_deref(),
            Some("followers:did:plc:alice")
        );
        assert!(
            nav_view.find_page("following:did:plc:alice").is_none(),
            "popping back drops the pages that were stacked above"
        );

        window.destroy();
    }

    /// A pushed profile page offers both follow lists as real click targets,
    /// with the count shown when the profile brought one.
    #[test]
    fn a_pushed_profile_page_offers_its_follow_lists() {
        crate::ui::with_gtk(a_pushed_profile_page_offers_its_follow_lists_body);
    }

    fn a_pushed_profile_page_offers_its_follow_lists_body() {
        let window: HangarWindow = glib::Object::builder().build();

        let mut profile = Profile::minimal(
            "did:plc:counted".into(),
            "counted.bsky.social".into(),
            None,
            None,
        );
        profile.followers_count = Some(1200);
        // following_count stays None: the target must still be clickable.
        window.push_profile_page(&profile, vec![], None);

        let nav_view = window
            .imp()
            .home_nav_view
            .borrow()
            .clone()
            .expect("the home section has a navigation view");
        let page = nav_view
            .find_page("profile:did:plc:counted")
            .expect("the profile page is in the stack");

        let mut widgets = Vec::new();
        walk(&page.clone().upcast::<gtk4::Widget>(), 0, &mut widgets);
        let stat_box_for = |noun: &str| -> gtk4::Widget {
            widgets
                .iter()
                .find_map(|(_, w)| {
                    let label = w.downcast_ref::<gtk4::Label>()?;
                    (label.text() == noun).then(|| label.parent())?
                })
                .unwrap_or_else(|| panic!("no {noun} stat on the page"))
        };

        for noun in ["followers", "following"] {
            let stat_box = stat_box_for(noun);
            assert!(
                !click_phases(&stat_box).is_empty(),
                "the {noun} stat takes clicks"
            );
            assert!(
                stat_box.cursor().is_some(),
                "the {noun} stat shows a pointer"
            );
        }

        // The known count is shown; the missing one leaves just the word.
        let followers_box = stat_box_for("followers");
        assert_eq!(
            followers_box
                .first_child()
                .and_downcast::<gtk4::Label>()
                .map(|l| l.text().to_string()),
            Some("1.2K".into())
        );
        let following_box = stat_box_for("following");
        assert!(
            following_box
                .first_child()
                .and_downcast::<gtk4::Label>()
                .is_some_and(|l| l.text() == "following"),
            "a missing count leaves the word as the whole stat"
        );

        window.destroy();
    }

    /// A failed first load swaps in the error page, whose Try Again re-runs
    /// the section's own fetch path; content arriving clears it, and a list
    /// that already has content ignores the failure beyond its toast.
    #[test]
    fn failed_first_loads_offer_try_again() {
        crate::ui::with_gtk(failed_first_loads_offer_try_again_body);
    }

    fn failed_first_loads_offer_try_again_body() {
        let window: HangarWindow = glib::Object::builder().build();
        let imp = window.imp();

        let retried: Rc<RefCell<Vec<crate::ui::sidebar::NavItem>>> = Rc::new(RefCell::new(vec![]));
        let sink = retried.clone();
        window.set_nav_changed_callback(move |item| {
            sink.borrow_mut().push(item);
        });
        let refreshed: Rc<RefCell<u32>> = Rc::default();
        let sink = refreshed.clone();
        window.set_refresh_callback(move || {
            *sink.borrow_mut() += 1;
        });

        // Mentions: the nav-routed retry.
        window.set_mentions_load_failed();
        let error = imp.mentions_error_state.borrow().clone().unwrap();
        assert!(error.get_visible(), "an empty failed list shows the error");
        assert!(!imp.mentions_overlay.borrow().clone().unwrap().get_visible());
        assert!(
            !imp.mentions_empty_state
                .borrow()
                .clone()
                .unwrap()
                .get_visible()
        );

        let try_again = error
            .child()
            .and_downcast::<gtk4::Button>()
            .expect("the error page holds Try Again");
        try_again.emit_clicked();
        assert_eq!(
            retried.borrow().as_slice(),
            [crate::ui::sidebar::NavItem::Mentions],
            "Try Again re-opens the section"
        );

        window.set_mentions(vec![notification(true)]);
        assert!(!error.get_visible(), "content clears the error page");
        assert!(imp.mentions_overlay.borrow().clone().unwrap().get_visible());

        // With content listed, a later failure keeps the list on screen.
        window.set_mentions_load_failed();
        assert!(!error.get_visible(), "content on screen outranks the error");

        // Timeline: the refresh-routed retry.
        window.set_timeline_load_failed();
        let error = imp.timeline_error_state.borrow().clone().unwrap();
        assert!(error.get_visible());
        error
            .child()
            .and_downcast::<gtk4::Button>()
            .unwrap()
            .emit_clicked();
        assert_eq!(*refreshed.borrow(), 1, "the feed retries through refresh");

        // The remaining four share the machinery; prove they are wired.
        window.set_activity_load_failed();
        assert!(
            imp.activity_error_state
                .borrow()
                .clone()
                .unwrap()
                .get_visible()
        );
        window.set_likes_load_failed();
        assert!(
            imp.likes_error_state
                .borrow()
                .clone()
                .unwrap()
                .get_visible()
        );
        window.set_bookmarks_load_failed();
        assert!(
            imp.bookmarks_error_state
                .borrow()
                .clone()
                .unwrap()
                .get_visible()
        );
        window.set_conversations_load_failed();
        assert!(imp.chat_error_state.borrow().clone().unwrap().get_visible());

        window.destroy();
    }

    /// The rail compacts on narrow widths through the width property, not
    /// an AdwBreakpoint: a breakpoint drops the window's minimum size and
    /// pages clipped at the edge instead of adapting.
    #[test]
    fn the_rail_compacts_by_width_without_a_breakpoint() {
        crate::ui::with_gtk(the_rail_compacts_by_width_without_a_breakpoint_body);
    }

    fn the_rail_compacts_by_width_without_a_breakpoint_body() {
        let window: HangarWindow = glib::Object::builder().build();
        let sidebar = window.imp().sidebar.borrow().clone().unwrap();
        let label_visible = || {
            sidebar
                .imp()
                .nav_text_labels
                .borrow()
                .first()
                .map(|l| l.get_visible())
                .unwrap()
        };

        window.set_default_size(500, 800);
        assert!(!label_visible(), "narrow windows drop the captions");
        window.set_default_size(900, 800);
        assert!(label_visible(), "room brings them back");

        window.destroy();
    }

    /// The feed selector is a GMenu over a stateful action: items carry
    /// the feed URI as target, the state marks the current one, and
    /// activating routes the chosen feed to the app.
    #[test]
    fn the_feed_menu_selects_by_action() {
        crate::ui::with_gtk(the_feed_menu_selects_by_action_body);
    }

    fn the_feed_menu_selects_by_action_body() {
        let window: HangarWindow = glib::Object::builder().build();

        let chosen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
        let sink = chosen.clone();
        window.set_feed_changed_callback(move |feed| {
            sink.borrow_mut().push(feed.uri);
        });

        let feed = |uri: &str, name: &str| SavedFeed {
            feed_type: "feed".into(),
            uri: uri.into(),
            display_name: name.into(),
            description: None,
            pinned: true,
        };
        window.set_saved_feeds(vec![
            feed("", "Following"),
            feed("at://did:plc:x/app.bsky.feed.generator/hot", "Hot"),
        ]);

        let menu = window.imp().feed_menu.borrow().clone().unwrap();
        assert_eq!(menu.n_items(), 2, "one menu item per saved feed");

        window.set_current_feed_name("Hot", "at://did:plc:x/app.bsky.feed.generator/hot");
        let action = window.imp().feed_action.borrow().clone().unwrap();
        assert_eq!(
            action.state().and_then(|s| s.get::<String>()).as_deref(),
            Some("at://did:plc:x/app.bsky.feed.generator/hot"),
            "the state is what paints the radio dot"
        );

        gtk4::prelude::WidgetExt::activate_action(&window, "feeds.select", Some(&"".to_variant()))
            .expect("the action group is on the window");
        assert_eq!(
            chosen.borrow().as_slice(),
            [""],
            "activation hands the chosen feed to the app"
        );

        window.destroy();
    }

    /// A first load shows skeleton rows; a later page keeps the corner
    /// spinner; content, emptiness, or failure all clear the skeletons.
    #[test]
    fn first_loads_show_skeletons_until_the_verdict() {
        crate::ui::with_gtk(first_loads_show_skeletons_until_the_verdict_body);
    }

    fn first_loads_show_skeletons_until_the_verdict_body() {
        let window: HangarWindow = glib::Object::builder().build();
        let imp = window.imp();

        let skeleton = imp.mentions_skeleton.borrow().clone().unwrap();
        let overlay = imp.mentions_overlay.borrow().clone().unwrap();
        let spinner = imp.mentions_spinner.borrow().clone().unwrap();

        window.set_mentions_loading(true);
        assert!(skeleton.get_visible(), "an empty first load shows shapes");
        assert!(!overlay.get_visible());
        assert!(!spinner.get_visible(), "no spinner on top of the shapes");

        window.set_mentions(vec![notification(true)]);
        assert!(!skeleton.get_visible(), "content clears the shapes");
        assert!(overlay.get_visible());

        window.set_mentions_loading(true);
        assert!(
            !skeleton.get_visible(),
            "a later page keeps the content on screen"
        );
        assert!(spinner.get_visible(), "and marks it with the spinner");
        window.set_mentions_loading(false);
        assert!(!spinner.get_visible());

        // A failed first load hands over to the error page.
        window.set_mentions(vec![]);
        window.set_mentions_loading(true);
        assert!(skeleton.get_visible());
        window.set_mentions_load_failed();
        assert!(
            !skeleton.get_visible(),
            "the error page replaces the shapes"
        );
        assert!(
            imp.mentions_error_state
                .borrow()
                .clone()
                .unwrap()
                .get_visible()
        );

        window.destroy();
    }

    /// The offline banner follows connectivity and only moves on change.
    #[test]
    fn the_offline_banner_follows_connectivity() {
        crate::ui::with_gtk(the_offline_banner_follows_connectivity_body);
    }

    fn the_offline_banner_follows_connectivity_body() {
        let window: HangarWindow = glib::Object::builder().build();
        let banner = window.imp().offline_banner.borrow().clone().unwrap();

        assert!(!banner.is_revealed(), "online until told otherwise");
        window.set_offline(true);
        assert!(banner.is_revealed());
        window.set_offline(false);
        assert!(!banner.is_revealed());

        window.destroy();
    }

    /// A dead-end people search fills with suggested accounts, and a fresh
    /// search clears them with the rest of the verdict.
    #[test]
    fn an_empty_people_search_offers_suggestions() {
        crate::ui::with_gtk(an_empty_people_search_offers_suggestions_body);
    }

    fn an_empty_people_search_offers_suggestions_body() {
        let window: HangarWindow = glib::Object::builder().build();
        let imp = window.imp();

        window.set_search_people_results(vec![]);
        let rows = imp.search_suggestions_box.borrow().clone().unwrap();
        assert!(rows.first_child().is_none(), "nothing suggested yet");

        window.set_search_suggestions(vec![Profile::minimal(
            "did:plc:suggested".into(),
            "suggested.bsky.social".into(),
            None,
            None,
        )]);
        assert!(rows.first_child().and_downcast::<ActorRow>().is_some());
        assert!(
            rows.parent().unwrap().get_visible(),
            "the suggestions column shows with content"
        );

        window.clear_search_results();
        assert!(rows.first_child().is_none(), "a fresh search starts clean");
        assert!(!rows.parent().unwrap().get_visible());

        window.destroy();
    }

    /// The New Message picker reports what is typed and lists the people
    /// handed back.
    #[test]
    fn the_new_message_picker_searches_and_lists() {
        crate::ui::with_gtk(the_new_message_picker_searches_and_lists_body);
    }

    fn the_new_message_picker_searches_and_lists_body() {
        let window: HangarWindow = glib::Object::builder().build();

        let queries: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
        let sink = queries.clone();
        window.set_new_message_query_callback(move |q| {
            sink.borrow_mut().push(q);
        });

        let dialog = window.build_new_message_dialog();
        // The dialog parents its child at present time; walk the content.
        let content = dialog.child().expect("the picker has content");
        let mut widgets = Vec::new();
        walk(&content, 0, &mut widgets);
        let entry = widgets
            .iter()
            .find_map(|(_, w)| w.downcast_ref::<gtk4::SearchEntry>().cloned())
            .expect("the picker has a search entry");

        entry.set_text("pete");
        entry.emit_by_name::<()>("search-changed", &[]);
        assert_eq!(queries.borrow().as_slice(), ["pete"]);

        window.set_new_message_results(vec![Profile::minimal(
            "did:plc:pete".into(),
            "pete.bsky.social".into(),
            None,
            None,
        )]);
        let rows = window
            .imp()
            .new_message_results_box
            .borrow()
            .clone()
            .unwrap();
        assert!(rows.first_child().and_downcast::<ActorRow>().is_some());

        window.destroy();
    }

    /// Edit Profile on the own page reaches the app through its callback.
    #[test]
    fn edit_profile_reports_the_click() {
        crate::ui::with_gtk(edit_profile_reports_the_click_body);
    }

    fn edit_profile_reports_the_click_body() {
        let window: HangarWindow = glib::Object::builder().build();
        let clicked: Rc<RefCell<u32>> = Rc::default();
        let sink = clicked.clone();
        window.set_edit_profile_callback(move || {
            *sink.borrow_mut() += 1;
        });

        let mut widgets = Vec::new();
        walk(&window.clone().upcast::<gtk4::Widget>(), 0, &mut widgets);
        let btn = widgets
            .iter()
            .find_map(|(_, w)| {
                let b = w.downcast_ref::<gtk4::Button>()?;
                (b.label().as_deref() == Some("Edit Profile")).then(|| b.clone())
            })
            .expect("the own page offers Edit Profile");
        btn.emit_clicked();
        assert_eq!(*clicked.borrow(), 1);

        window.destroy();
    }

    /// The own profile page shares the drill-down tab machinery: the same
    /// four tabs, and a context whose DID waits for sign-in.
    #[test]
    fn the_own_profile_shares_the_tab_machinery() {
        crate::ui::with_gtk(the_own_profile_shares_the_tab_machinery_body);
    }

    fn the_own_profile_shares_the_tab_machinery_body() {
        let window: HangarWindow = glib::Object::builder().build();

        let ctx = window
            .own_profile_feed_ctx()
            .expect("the own page built its feed context");
        assert!(
            ctx.did.borrow().is_empty(),
            "the DID waits for sign-in to fill it"
        );

        let asked: Rc<RefCell<Vec<(String, bool)>>> = Rc::new(RefCell::new(vec![]));
        let sink = asked.clone();
        window.set_profile_tab_callback(move |ctx, first| {
            sink.borrow_mut()
                .push((ctx.filter.get().to_string(), first));
        });

        // No drill-down page is pushed, so the only tabs are the own page's.
        let mut widgets = Vec::new();
        walk(&window.clone().upcast::<gtk4::Widget>(), 0, &mut widgets);
        let replies = widgets
            .iter()
            .find_map(|(_, w)| {
                let t = w.downcast_ref::<gtk4::ToggleButton>()?;
                (t.label().as_deref() == Some("Replies")).then(|| t.clone())
            })
            .expect("the own page has the tabs");
        replies.set_active(true);
        assert_eq!(
            asked.borrow().as_slice(),
            [("posts_with_replies".to_string(), true)]
        );

        // Sign-in points the context at the account and starts over.
        ctx.did.replace("did:plc:me".into());
        ctx.begin_refresh();
        assert_eq!(ctx.listed(), 0);
        ctx.append_posts(vec![a_post("mine")]);
        assert_eq!(ctx.listed(), 1, "the own page finally paginates via ctx");

        window.destroy();
    }

    /// The profile tabs ask the app for their own feed: switching clears
    /// the list, hands over the tab's filter as a first page, and bumps
    /// the generation so a fetch from the abandoned tab lands nowhere.
    #[test]
    fn profile_tabs_fetch_their_own_feed() {
        crate::ui::with_gtk(profile_tabs_fetch_their_own_feed_body);
    }

    fn profile_tabs_fetch_their_own_feed_body() {
        let window: HangarWindow = glib::Object::builder().build();

        type Asked = (String, bool, u64);
        let asked: Rc<RefCell<Vec<Asked>>> = Rc::new(RefCell::new(vec![]));
        let ctx_cell: Rc<RefCell<Option<Rc<ProfileFeedCtx>>>> = Rc::new(RefCell::new(None));
        let sink = asked.clone();
        let stash = ctx_cell.clone();
        window.set_profile_tab_callback(move |ctx, first| {
            sink.borrow_mut()
                .push((ctx.filter.get().to_string(), first, ctx.generation.get()));
            stash.borrow_mut().replace(ctx);
        });

        let profile =
            Profile::minimal("did:plc:tabs".into(), "tabs.bsky.social".into(), None, None);
        window.push_profile_page(&profile, vec![a_post("p1")], Some("cursor-1".into()));

        let nav_view = window.imp().home_nav_view.borrow().clone().unwrap();
        let page = nav_view.find_page("profile:did:plc:tabs").unwrap();
        let mut widgets = Vec::new();
        walk(&page.upcast::<gtk4::Widget>(), 0, &mut widgets);
        let tab = |label: &str| -> gtk4::ToggleButton {
            widgets
                .iter()
                .find_map(|(_, w)| {
                    let t = w.downcast_ref::<gtk4::ToggleButton>()?;
                    (t.label().as_deref() == Some(label)).then(|| t.clone())
                })
                .unwrap_or_else(|| panic!("no {label} tab"))
        };

        assert!(tab("Posts").is_active(), "Posts is the opening tab");
        assert_eq!(
            tab("Posts").parent().unwrap().halign(),
            gtk4::Align::Center,
            "the selector sits centered under the centered header"
        );
        assert!(
            asked.borrow().is_empty(),
            "the opening tab rides the pushed posts, no refetch"
        );

        tab("Replies").set_active(true);
        assert_eq!(
            asked.borrow().as_slice(),
            [("posts_with_replies".to_string(), true, 1)],
            "a switch asks for the tab's filter as a first page"
        );
        let ctx = ctx_cell.borrow().clone().unwrap();
        assert_eq!(ctx.listed(), 0, "the switch cleared the old tab's posts");
        assert!(ctx.cursor.borrow().is_none(), "and its cursor");

        // The app lands a page; the model carries it.
        ctx.append_posts(vec![a_post("r1")]);
        assert_eq!(ctx.listed(), 1);

        tab("Media").set_active(true);
        assert_eq!(asked.borrow().len(), 2);
        assert_eq!(
            asked.borrow()[1],
            ("posts_with_media".to_string(), true, 2),
            "each switch bumps the generation"
        );
        assert_eq!(ctx.listed(), 0, "the replies page left with its tab");

        tab("Videos").set_active(true);
        assert_eq!(
            asked.borrow()[2],
            ("posts_with_video".to_string(), true, 3),
            "the Videos tab rides the lexicon's video filter"
        );

        // The Hide Replies feed setting must never reach profile tabs:
        // this path filters nothing, so a reply lands regardless of it.
        let mut reply = a_post("a-reply");
        let someone = Profile::minimal(
            "did:plc:someone".into(),
            "someone.bsky.social".into(),
            None,
            None,
        );
        reply.reply_context = Some(crate::atproto::ReplyContext {
            parent_author: someone.clone(),
            root_author: someone,
        });
        ctx.append_posts(vec![reply]);
        assert_eq!(
            ctx.listed(),
            1,
            "profile tabs bypass the timeline's reply filter"
        );

        window.destroy();
    }

    /// A pushed profile page holds one list whose first row is the header
    /// marker, so the whole page scrolls; the condensed bar waits unrevealed
    /// on top of the scroller.
    #[test]
    fn the_profile_page_scrolls_whole_with_a_waiting_bar() {
        crate::ui::with_gtk(the_profile_page_scrolls_whole_with_a_waiting_bar_body);
    }

    fn the_profile_page_scrolls_whole_with_a_waiting_bar_body() {
        let window: HangarWindow = glib::Object::builder().build();

        let profile = Profile::minimal(
            "did:plc:scrolls".into(),
            "scrolls.bsky.social".into(),
            None,
            None,
        );
        window.push_profile_page(&profile, vec![a_post("p1"), a_post("p2")], None);

        let nav_view = window.imp().home_nav_view.borrow().clone().unwrap();
        let page = nav_view.find_page("profile:did:plc:scrolls").unwrap();

        let list = find_list_view(&page.clone().upcast::<gtk4::Widget>())
            .expect("the profile page has its list");
        let flattened = list
            .model()
            .and_then(|m| m.downcast::<gtk4::NoSelection>().ok())
            .and_then(|ns| ns.model())
            .and_downcast::<gtk4::FlattenListModel>()
            .expect("the header rides the list as its first row");
        assert_eq!(flattened.n_items(), 3, "marker plus two posts");
        assert!(
            flattened
                .item(0)
                .and_downcast::<gtk4::StringObject>()
                .is_some(),
            "row 0 is the header marker"
        );

        let mut widgets = Vec::new();
        walk(&page.clone().upcast::<gtk4::Widget>(), 0, &mut widgets);
        let revealer = widgets
            .iter()
            .find_map(|(_, w)| {
                let r = w.downcast_ref::<gtk4::Revealer>()?;
                r.child()?
                    .has_css_class("condensed-profile-bar")
                    .then(|| r.clone())
            })
            .expect("the condensed bar sits in a revealer");
        assert!(
            !revealer.reveals_child(),
            "the bar waits for the header to scroll away"
        );

        window.destroy();
    }

    /// Back pops the visible stack one page and idles at the root. The
    /// window state save and restore run without the schema installed,
    /// which is what every bare cargo run looks like.
    #[test]
    fn back_pops_one_page_and_state_saving_survives_no_schema() {
        crate::ui::with_gtk(back_pops_one_page_and_state_saving_survives_no_schema_body);
    }

    fn back_pops_one_page_and_state_saving_survives_no_schema_body() {
        let window: HangarWindow = glib::Object::builder().build();

        let profile =
            Profile::minimal("did:plc:back".into(), "back.bsky.social".into(), None, None);
        window.push_profile_page(&profile, vec![], None);
        let nav_view = window.imp().home_nav_view.borrow().clone().unwrap();
        assert!(nav_view.find_page("profile:did:plc:back").is_some());

        window.go_back();
        assert!(
            nav_view.find_page("profile:did:plc:back").is_none(),
            "back pops the pushed page"
        );
        window.go_back();

        // No gschema in the test environment; both must no-op cleanly.
        window.restore_window_state();
        window.save_window_state();

        window.destroy();
    }

    /// The shortcuts reference lists every accelerator the application
    /// registers: the general four plus back and the eight sections.
    #[test]
    fn the_shortcuts_window_lists_the_registered_accelerators() {
        crate::ui::with_gtk(the_shortcuts_window_lists_the_registered_accelerators_body);
    }

    fn the_shortcuts_window_lists_the_registered_accelerators_body() {
        let overlay = HangarWindow::build_help_overlay();
        let mut widgets = Vec::new();
        walk(&overlay.clone().upcast::<gtk4::Widget>(), 0, &mut widgets);
        // The window mirrors every entry into its search view, so count
        // distinct titles rather than widgets.
        let titles: std::collections::HashSet<String> = widgets
            .iter()
            .filter(|(_, w)| w.is::<gtk4::ShortcutsShortcut>())
            .map(|(_, w)| w.property::<String>("title"))
            .collect();
        assert_eq!(
            titles.len(),
            19,
            "4 general + back + 7 sections + 7 post keys"
        );
        assert!(titles.contains("Refresh"));
        assert!(titles.contains("Saved"));
        assert!(titles.contains("Search"), "one entry carries both keys");
        assert!(titles.contains("Like"), "the post keys are listed");
        overlay.destroy();
    }

    /// Every list swaps to its empty state when a load turns out empty and
    /// back when content arrives. Search only counts a completed search:
    /// clearing for a fresh one hides the no-results page again.
    #[test]
    fn every_empty_list_swaps_to_its_status_page() {
        crate::ui::with_gtk(every_empty_list_swaps_to_its_status_page_body);
    }

    fn every_empty_list_swaps_to_its_status_page_body() {
        let window: HangarWindow = glib::Object::builder().build();
        let imp = window.imp();

        let check = |name: &str,
                     overlay: &RefCell<Option<gtk4::Overlay>>,
                     page: &RefCell<Option<adw::StatusPage>>,
                     set_empty: &dyn Fn(),
                     set_full: &dyn Fn()| {
            let overlay = overlay
                .borrow()
                .clone()
                .unwrap_or_else(|| panic!("{name} overlay"));
            let page = page
                .borrow()
                .clone()
                .unwrap_or_else(|| panic!("{name} empty state"));
            set_empty();
            assert!(page.get_visible(), "{name}: empty shows the status page");
            assert!(!overlay.get_visible(), "{name}: empty hides the list");
            set_full();
            assert!(!page.get_visible(), "{name}: content hides the status page");
            assert!(overlay.get_visible(), "{name}: content shows the list");
        };

        let w = &window;
        check(
            "timeline",
            &imp.timeline_overlay,
            &imp.timeline_empty_state,
            &|| w.set_posts(vec![]),
            &|| w.set_posts(vec![a_post("t1")]),
        );
        check(
            "mentions",
            &imp.mentions_overlay,
            &imp.mentions_empty_state,
            &|| w.set_mentions(vec![]),
            &|| w.set_mentions(vec![notification(true)]),
        );
        check(
            "activity",
            &imp.activity_overlay,
            &imp.activity_empty_state,
            &|| w.set_activity(vec![]),
            &|| w.set_activity(vec![notification(false)]),
        );
        check(
            "likes",
            &imp.likes_overlay,
            &imp.likes_empty_state,
            &|| w.set_likes(vec![]),
            &|| w.set_likes(vec![a_post("l1")]),
        );

        // Search: the pages live inside the results stack, so only the
        // status page toggles.
        let posts_empty = imp.search_empty_state.borrow().clone().unwrap();
        let people_empty = imp.search_people_empty_state.borrow().clone().unwrap();
        assert!(!posts_empty.get_visible(), "no search yet, no verdict");

        window.set_search_results(vec![]);
        window.set_search_people_results(vec![]);
        assert!(posts_empty.get_visible());
        assert!(people_empty.get_visible());

        window.clear_search_results();
        assert!(
            !posts_empty.get_visible(),
            "a fresh search resets the verdict"
        );
        assert!(!people_empty.get_visible());

        window.set_search_results(vec![a_post("s1")]);
        window.set_search_people_results(vec![Profile::minimal(
            "did:plc:found".into(),
            "found.bsky.social".into(),
            None,
            None,
        )]);
        assert!(!posts_empty.get_visible());
        assert!(!people_empty.get_visible());

        window.destroy();
    }

    /// New posts arriving while the reader is at the top just appear;
    /// the banner and the Home badge stay quiet. An unscrolled window is
    /// always at the top, which was exactly the reported quirk: a banner
    /// with nowhere to go after switching back to the feed tab.
    #[test]
    fn new_posts_at_the_top_skip_the_banner() {
        crate::ui::with_gtk(new_posts_at_the_top_skip_the_banner_body);
    }

    fn new_posts_at_the_top_skip_the_banner_body() {
        let window: HangarWindow = glib::Object::builder().build();

        window.set_posts(vec![a_post("one")]);
        window.insert_posts_at_top(vec![a_post("two"), a_post("three")]);
        window.show_new_posts_banner(2);

        let banner = window.imp().new_posts_banner.borrow().clone().unwrap();
        assert!(!banner.is_visible(), "nothing to announce at the top");
        let sidebar = window.imp().sidebar.borrow().clone().unwrap();
        assert_eq!(
            sidebar.badge_count(crate::ui::sidebar::NavItem::Home),
            0,
            "posts in view are not unseen"
        );

        window.destroy();
    }

    /// The stylesheet must parse clean. GTK only warns at runtime, so a
    /// bad selector would otherwise ship as silently dropped rules.
    #[test]
    fn the_stylesheet_parses_without_errors() {
        crate::ui::with_gtk(the_stylesheet_parses_without_errors_body);
    }

    fn the_stylesheet_parses_without_errors_body() {
        let provider = gtk4::CssProvider::new();
        let errors: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
        let sink = errors.clone();
        provider.connect_parsing_error(move |_, section, error| {
            sink.borrow_mut().push(format!("{section}: {error}"));
        });
        provider.load_from_string(include_str!("style.css"));
        assert!(errors.borrow().is_empty(), "{:?}", errors.borrow());
    }

    /// The follow button starts from viewer state, hands the callback the
    /// DID and the URI cell, shows the hoped-for state while waiting, and
    /// stays off your own page.
    #[test]
    fn the_follow_button_tracks_state_and_skips_your_own_page() {
        crate::ui::with_gtk(the_follow_button_tracks_state_and_skips_your_own_page_body);
    }

    fn the_follow_button_tracks_state_and_skips_your_own_page_body() {
        let window: HangarWindow = glib::Object::builder().build();

        type Cell = Rc<RefCell<Option<String>>>;
        let calls: Rc<RefCell<Vec<(String, Cell)>>> = Rc::new(RefCell::new(vec![]));
        let sink = calls.clone();
        window.set_follow_callback(move |did, cell, _btn| {
            sink.borrow_mut().push((did, cell));
        });

        let follow_button_on = |tag: &str| -> gtk4::Button {
            let nav_view = window.imp().home_nav_view.borrow().clone().unwrap();
            let page = nav_view.find_page(tag).expect("page is in the stack");
            let mut widgets = Vec::new();
            walk(&page.upcast::<gtk4::Widget>(), 0, &mut widgets);
            widgets
                .iter()
                .find_map(|(_, w)| {
                    let btn = w.downcast_ref::<gtk4::Button>()?;
                    matches!(btn.label().as_deref(), Some("Follow" | "Following"))
                        .then(|| btn.clone())
                })
                .expect("the page has a follow button")
        };

        // Not followed yet, and they follow us.
        let mut stranger = Profile::minimal(
            "did:plc:stranger".into(),
            "stranger.bsky.social".into(),
            None,
            None,
        );
        stranger.viewer_followed_by = Some("at://did:plc:stranger/app.bsky.graph.follow/1".into());
        window.push_profile_page(&stranger, vec![], None);

        let btn = follow_button_on("profile:did:plc:stranger");
        assert_eq!(btn.label().as_deref(), Some("Follow"));
        assert!(btn.has_css_class("suggested-action"));
        {
            let nav_view = window.imp().home_nav_view.borrow().clone().unwrap();
            let page = nav_view.find_page("profile:did:plc:stranger").unwrap();
            let mut widgets = Vec::new();
            walk(&page.upcast::<gtk4::Widget>(), 0, &mut widgets);
            assert!(
                widgets.iter().any(|(_, w)| w
                    .downcast_ref::<gtk4::Label>()
                    .is_some_and(|l| l.text() == "Follows you")),
                "their follow of us is shown"
            );
        }

        // Click: the callback gets the DID and the empty cell, the button
        // shows Following and locks until the app reports back.
        btn.emit_clicked();
        assert_eq!(calls.borrow().len(), 1);
        let (did, cell) = calls.borrow()[0].clone();
        assert_eq!(did, "did:plc:stranger");
        assert!(cell.borrow().is_none());
        assert_eq!(btn.label().as_deref(), Some("Following"));
        assert!(!btn.has_css_class("suggested-action"));
        assert!(!btn.is_sensitive());

        // The app answers with the record URI and unlocks the button.
        cell.replace(Some("at://did:plc:me/app.bsky.graph.follow/9".into()));
        HangarWindow::sync_follow_button(&btn, true);
        assert!(btn.is_sensitive());

        // Second click asks to unfollow: same cell, now holding the URI.
        btn.emit_clicked();
        assert_eq!(calls.borrow().len(), 2);
        let (_, cell) = calls.borrow()[1].clone();
        assert_eq!(
            cell.borrow().as_deref(),
            Some("at://did:plc:me/app.bsky.graph.follow/9")
        );
        assert_eq!(btn.label().as_deref(), Some("Follow"));

        // Someone already followed starts from Following.
        let mut followed = Profile::minimal(
            "did:plc:followed".into(),
            "followed.bsky.social".into(),
            None,
            None,
        );
        followed.viewer_following = Some("at://did:plc:me/app.bsky.graph.follow/2".into());
        window.push_profile_page(&followed, vec![], None);
        let btn = follow_button_on("profile:did:plc:followed");
        assert_eq!(btn.label().as_deref(), Some("Following"));
        assert!(!btn.has_css_class("suggested-action"));

        // Your own page carries no follow button.
        window.set_current_user_did("did:plc:me");
        let me = Profile::minimal("did:plc:me".into(), "me.bsky.social".into(), None, None);
        window.push_profile_page(&me, vec![], None);
        let nav_view = window.imp().home_nav_view.borrow().clone().unwrap();
        let page = nav_view.find_page("profile:did:plc:me").unwrap();
        let mut widgets = Vec::new();
        walk(&page.upcast::<gtk4::Widget>(), 0, &mut widgets);
        assert!(
            !widgets.iter().any(|(_, w)| w
                .downcast_ref::<gtk4::Button>()
                .is_some_and(|b| matches!(b.label().as_deref(), Some("Follow" | "Following")))),
            "no follow button on your own page"
        );

        window.destroy();
    }

    /// The Message button hands the shown profile over once and locks
    /// while the availability check is out, and stays off your own page.
    #[test]
    fn the_message_button_asks_once_and_locks() {
        crate::ui::with_gtk(the_message_button_asks_once_and_locks_body);
    }

    fn the_message_button_asks_once_and_locks_body() {
        let window: HangarWindow = glib::Object::builder().build();

        let asked: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
        let sink = asked.clone();
        window.set_message_callback(move |profile, _btn| {
            sink.borrow_mut().push(profile.did);
        });

        let message_button_on = |tag: &str| -> Option<gtk4::Button> {
            let nav_view = window.imp().home_nav_view.borrow().clone().unwrap();
            let page = nav_view.find_page(tag).expect("page is in the stack");
            let mut widgets = Vec::new();
            walk(&page.upcast::<gtk4::Widget>(), 0, &mut widgets);
            widgets.iter().find_map(|(_, w)| {
                let btn = w.downcast_ref::<gtk4::Button>()?;
                (btn.label().as_deref() == Some("Message")).then(|| btn.clone())
            })
        };

        let stranger = Profile::minimal(
            "did:plc:stranger".into(),
            "stranger.bsky.social".into(),
            None,
            None,
        );
        window.push_profile_page(&stranger, vec![], None);

        let btn = message_button_on("profile:did:plc:stranger").expect("a Message button");
        btn.emit_clicked();
        assert_eq!(asked.borrow().as_slice(), ["did:plc:stranger"]);
        assert!(!btn.is_sensitive(), "locked until the check answers");

        window.set_current_user_did("did:plc:me");
        let me = Profile::minimal("did:plc:me".into(), "me.bsky.social".into(), None, None);
        window.push_profile_page(&me, vec![], None);
        assert!(
            message_button_on("profile:did:plc:me").is_none(),
            "no Message button on your own page"
        );

        window.destroy();
    }

    /// The own-profile stats route to the follow-list callback with the
    /// shown profile and the side that was clicked, and only once a
    /// profile is actually loaded.
    #[test]
    fn the_own_profile_stats_ask_for_the_matching_list() {
        crate::ui::with_gtk(the_own_profile_stats_ask_for_the_matching_list_body);
    }

    fn the_own_profile_stats_ask_for_the_matching_list_body() {
        let window: HangarWindow = glib::Object::builder().build();
        let imp = window.imp();

        let opened: Rc<RefCell<Vec<(String, FollowListKind)>>> = Rc::new(RefCell::new(vec![]));
        let sink = opened.clone();
        window.set_follow_list_clicked_callback(move |profile, kind| {
            sink.borrow_mut().push((profile.did, kind));
        });

        // Nothing loaded yet: a click has no list to open.
        window.open_follow_list_for_current_profile(FollowListKind::Followers);
        assert!(opened.borrow().is_empty());

        let me = Profile::minimal("did:plc:me".into(), "me.bsky.social".into(), None, None);
        window.update_profile_header(&me);
        window.open_follow_list_for_current_profile(FollowListKind::Followers);
        window.open_follow_list_for_current_profile(FollowListKind::Following);
        assert_eq!(
            *opened.borrow(),
            vec![
                ("did:plc:me".to_string(), FollowListKind::Followers),
                ("did:plc:me".to_string(), FollowListKind::Following),
            ]
        );

        // And the boxes themselves are wired as click targets.
        for (name, cell) in [
            ("followers", &imp.profile_followers_box),
            ("following", &imp.profile_following_box),
        ] {
            let stat_box = cell
                .borrow()
                .clone()
                .unwrap_or_else(|| panic!("the header stored its {name} box"));
            let widget = stat_box.upcast::<gtk4::Widget>();
            assert!(
                !click_phases(&widget).is_empty(),
                "the {name} box takes clicks"
            );
            assert!(widget.cursor().is_some(), "the {name} box shows a pointer");
        }

        window.destroy();
    }
}
