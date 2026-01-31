// SPDX-License-Identifier: MPL-2.0

use super::post_row::PostRow;
use super::sidebar::Sidebar;
use crate::atproto::{Notification, Post, SavedFeed};
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;
use std::cell::RefCell;

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

use crate::atproto::Profile;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct HangarWindow {
        pub sidebar: RefCell<Option<Sidebar>>,
        // Main content stack for top-level pages (Home, Mentions, etc.)
        pub main_stack: RefCell<Option<gtk4::Stack>>,
        // NavigationView for Home section (for thread/profile drill-down)
        pub home_nav_view: RefCell<Option<adw::NavigationView>>,
        // NavigationView for Mentions section (for thread/profile drill-down)
        pub mentions_nav_view: RefCell<Option<adw::NavigationView>>,
        pub timeline_model: RefCell<Option<gio::ListStore>>,
        pub load_more_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub refresh_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub like_callback: RefCell<Option<Box<dyn Fn(Post, glib::WeakRef<PostRow>) + 'static>>>,
        pub repost_callback: RefCell<Option<Box<dyn Fn(Post, glib::WeakRef<PostRow>) + 'static>>>,
        pub quote_callback: RefCell<Option<Box<dyn Fn(Post) + 'static>>>,
        pub reply_callback: RefCell<Option<Box<dyn Fn(Post) + 'static>>>,
        pub compose_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub loading_spinner: RefCell<Option<gtk4::Spinner>>,
        pub new_posts_banner: RefCell<Option<gtk4::Button>>,
        pub new_posts_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub scrolled_window: RefCell<Option<gtk4::ScrolledWindow>>,
        pub feed_btn_label: RefCell<Option<gtk4::Label>>,
        pub feed_popover: RefCell<Option<gtk4::Popover>>,
        pub feed_list_box: RefCell<Option<gtk4::ListBox>>,
        pub feed_changed_callback: RefCell<Option<Box<dyn Fn(SavedFeed) + 'static>>>,
        pub saved_feeds: RefCell<Vec<SavedFeed>>,
        pub current_feed_uri: RefCell<String>,
        // Navigation callbacks
        pub post_clicked_callback: RefCell<Option<Box<dyn Fn(Post) + 'static>>>,
        pub profile_clicked_callback: RefCell<Option<Box<dyn Fn(Profile) + 'static>>>,
        pub nav_changed_callback:
            RefCell<Option<Box<dyn Fn(crate::ui::sidebar::NavItem) + 'static>>>,
        // Mentions page state
        pub mentions_model: RefCell<Option<gio::ListStore>>,
        pub mentions_load_more_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub mentions_scrolled_window: RefCell<Option<gtk4::ScrolledWindow>>,
        pub mentions_spinner: RefCell<Option<gtk4::Spinner>>,
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
        @implements gio::ActionGroup, gio::ActionMap;
}

impl HangarWindow {
    pub fn new(app: &adw::Application) -> Self {
        glib::Object::builder()
            .property("application", app)
            .property("default-width", 1000)
            .property("default-height", 700)
            .property("title", "Hangar")
            .build()
    }

    fn setup_ui(&self) {
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
        let timeline_page = self.build_timeline_page();
        home_nav_view.add(&timeline_page);
        main_stack.add_named(&home_nav_view, Some("home"));

        // Mentions section: NavigationView for thread/profile drill-down
        let mentions_nav_view = adw::NavigationView::new();
        let mentions_page = self.build_mentions_page();
        mentions_nav_view.add(&mentions_page);
        main_stack.add_named(&mentions_nav_view, Some("mentions"));

        main_box.append(&main_stack);

        self.set_content(Some(&main_box));

        let imp = self.imp();
        imp.sidebar.replace(Some(sidebar));
        imp.main_stack.replace(Some(main_stack));
        imp.home_nav_view.replace(Some(home_nav_view));
        imp.mentions_nav_view.replace(Some(mentions_nav_view));

        // Add keyboard shortcuts
        self.setup_shortcuts();
    }

    fn setup_shortcuts(&self) {
        let controller = gtk4::ShortcutController::new();
        controller.set_scope(gtk4::ShortcutScope::Managed);

        // F5 to refresh
        let refresh_action = gtk4::CallbackAction::new(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, _| {
                if let Some(cb) = window.imp().refresh_callback.borrow().as_ref() {
                    cb();
                }
                glib::Propagation::Stop
            }
        ));
        let f5_shortcut = gtk4::Shortcut::new(
            gtk4::ShortcutTrigger::parse_string("F5"),
            Some(refresh_action.clone()),
        );
        controller.add_shortcut(f5_shortcut);

        // Ctrl+R to refresh
        let ctrl_r_shortcut = gtk4::Shortcut::new(
            gtk4::ShortcutTrigger::parse_string("<Control>r"),
            Some(refresh_action),
        );
        controller.add_shortcut(ctrl_r_shortcut);

        self.add_controller(controller);
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

        // Create popover with feed list
        let popover = gtk4::Popover::new();
        popover.set_has_arrow(false);
        popover.set_autohide(true);
        popover.add_css_class("menu");
        popover.set_position(gtk4::PositionType::Bottom);
        popover.set_offset(0, 8); // Push it down a bit to avoid overlapping header

        let popover_content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Header
        let header_label = gtk4::Label::new(Some("Select a Feed"));
        header_label.add_css_class("title-4");
        header_label.set_margin_top(12);
        header_label.set_margin_bottom(8);
        popover_content.append(&header_label);

        let feed_list = gtk4::ListBox::new();
        feed_list.set_selection_mode(gtk4::SelectionMode::None);

        // Connect row activation - close popover first, then fire callback
        let popover_weak = popover.downgrade();
        feed_list.connect_row_activated(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, row| {
                // Close popover first to avoid widget state issues
                if let Some(pop) = popover_weak.upgrade() {
                    pop.popdown();
                }

                let index = row.index() as usize;
                let feed = {
                    let feeds = window.imp().saved_feeds.borrow();
                    feeds.get(index).cloned()
                };
                if let Some(feed) = feed {
                    if let Some(cb) = window.imp().feed_changed_callback.borrow().as_ref() {
                        cb(feed);
                    }
                }
            }
        ));

        popover_content.append(&feed_list);
        popover.set_child(Some(&popover_content));
        feed_menu_btn.set_popover(Some(&popover));

        header.set_title_widget(Some(&feed_menu_btn));

        // Store references
        self.imp().feed_btn_label.replace(Some(title_label));
        self.imp().feed_popover.replace(Some(popover));
        self.imp().feed_list_box.replace(Some(feed_list));

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

        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.set_tooltip_text(Some("Close"));
        close_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                window.close();
            }
        ));
        header.pack_end(&close_btn);

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

        factory.connect_bind(glib::clone!(
            @strong self as win => move |_, item| {
                if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                    && let Some(post_object) = list_item.item().and_downcast::<PostObject>()
                    && let Some(post) = post_object.post()
                    && let Some(post_row) = list_item.child().and_downcast::<PostRow>()
                {
                    post_row.bind(&post);
                    // Like callback receives (post_row, was_liked, like_uri) captured before toggle
                    let post_for_like = post.clone();
                    let w = win.clone();
                    post_row.connect_like_clicked(move |row, was_liked, like_uri| {
                        let mut post_with_state = post_for_like.clone();
                        // If it was liked, we're unliking, so pass the like_uri
                        // If it wasn't liked, we're liking, so pass None
                        post_with_state.viewer_like = if was_liked { like_uri } else { None };
                        let row_weak = row.downgrade();
                        w.imp().like_callback.borrow().as_ref().map(|cb| cb(post_with_state, row_weak));
                    });
                    // Repost callback receives (post_row, was_reposted, repost_uri) captured before toggle
                    let post_for_repost = post.clone();
                    let w = win.clone();
                    post_row.connect_repost_clicked(move |row, was_reposted, repost_uri| {
                        let mut post_with_state = post_for_repost.clone();
                        post_with_state.viewer_repost = if was_reposted { repost_uri } else { None };
                        let row_weak = row.downgrade();
                        w.imp().repost_callback.borrow().as_ref().map(|cb| cb(post_with_state, row_weak));
                    });
                    let post_for_quote = post.clone();
                    let w = win.clone();
                    post_row.connect_quote_clicked(move || {
                        w.imp().quote_callback.borrow().as_ref().map(|cb| cb(post_for_quote.clone()));
                    });
                    let post_clone = post.clone();
                    let w = win.clone();
                    post_row.connect_reply_clicked(move || {
                        w.imp().reply_callback.borrow().as_ref().map(|cb| cb(post_clone.clone()));
                    });
                    // Navigation callbacks
                    let w = win.clone();
                    post_row.set_post_clicked_callback(move |p| {
                        w.imp().post_clicked_callback.borrow().as_ref().map(|cb| cb(p));
                    });
                    let w = win.clone();
                    post_row.set_profile_clicked_callback(move |profile| {
                        w.imp().profile_clicked_callback.borrow().as_ref().map(|cb| cb(profile));
                    });
                }
            }
        ));

        let selection = gtk4::NoSelection::new(Some(model.clone()));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.add_css_class("background");

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&list_view));
        overlay.set_child(Some(&scrolled));

        // "N new posts" banner
        let new_posts_btn = gtk4::Button::with_label("New posts");
        new_posts_btn.add_css_class("suggested-action");
        new_posts_btn.add_css_class("pill");
        new_posts_btn.add_css_class("new-posts-banner");
        new_posts_btn.set_halign(gtk4::Align::Center);
        new_posts_btn.set_valign(gtk4::Align::Start);
        new_posts_btn.set_margin_top(12);
        new_posts_btn.set_visible(false);
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
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        timeline_box.append(&overlay);

        let imp = self.imp();
        imp.timeline_model.replace(Some(model));
        imp.loading_spinner.replace(Some(spinner));
        imp.new_posts_banner.replace(Some(new_posts_btn));
        imp.scrolled_window.replace(Some(scrolled.clone()));

        let adj = scrolled.vadjustment();
        adj.connect_value_changed(glib::clone!(
            @weak self as win => move |adj| {
                let value = adj.value();
                let upper = adj.upper();
                let page_size = adj.page_size();
                if value >= upper - page_size - 200.0 {
                    if let Some(cb) = win.imp().load_more_callback.borrow().as_ref() {
                        cb();
                    }
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

    pub fn set_compose_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .compose_callback
            .replace(Some(Box::new(callback)));
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            let win = self.clone();
            sidebar.connect_compose_clicked(move || {
                win.imp().compose_callback.borrow().as_ref().map(|cb| cb());
            });
        }
    }

    pub fn set_posts(&self, posts: Vec<Post>) {
        if let Some(model) = self.imp().timeline_model.borrow().as_ref() {
            model.remove_all();

            for post in posts {
                let post_object = PostObject::new(post);
                model.append(&post_object);
            }
        }
    }

    pub fn append_posts(&self, posts: Vec<Post>) {
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
        if let Some(banner) = self.imp().new_posts_banner.borrow().as_ref() {
            let label = if count == 1 {
                "1 new post".to_string()
            } else if count > 99 {
                "99+ new posts".to_string()
            } else {
                format!("{} new posts", count)
            };
            banner.set_label(&label);
            banner.set_visible(true);
        }
    }

    pub fn hide_new_posts_banner(&self) {
        if let Some(banner) = self.imp().new_posts_banner.borrow().as_ref() {
            banner.set_visible(false);
        }
    }

    pub fn scroll_to_top(&self) {
        if let Some(scrolled) = self.imp().scrolled_window.borrow().as_ref() {
            let adj = scrolled.vadjustment();
            adj.set_value(0.0);
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
            // Insert at the beginning
            for (i, post) in posts.into_iter().enumerate() {
                let post_object = PostObject::new(post);
                model.insert(i as u32, &post_object);
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

    /// Rebuild the feed list UI (called when feeds change or selection changes)
    fn rebuild_feed_list(&self) {
        let Some(list_box) = self.imp().feed_list_box.borrow().as_ref().cloned() else {
            return;
        };

        let feeds = self.imp().saved_feeds.borrow();
        let current_uri = self.imp().current_feed_uri.borrow();

        // Clear existing rows
        while let Some(child) = list_box.first_child() {
            list_box.remove(&child);
        }

        // Add rows for each feed
        for feed in feeds.iter() {
            let row = adw::ActionRow::new();
            row.set_title(&feed.display_name);

            // Show truncated description if available
            if let Some(desc) = &feed.description {
                // Truncate long descriptions
                let truncated = if desc.len() > 60 {
                    format!("{}...", &desc[..57])
                } else {
                    desc.clone()
                };
                row.set_subtitle(&truncated);
                row.set_subtitle_lines(1);
            }

            // Add checkmark for selected feed
            let is_selected = feed.uri == *current_uri;
            if is_selected {
                let check = gtk4::Image::from_icon_name("object-select-symbolic");
                check.add_css_class("accent");
                row.add_suffix(&check);
            }

            row.set_activatable(true);
            list_box.append(&row);
        }
    }

    /// Update the feed selector button label and selection state
    pub fn set_current_feed_name(&self, name: &str, uri: &str) {
        if let Some(label) = self.imp().feed_btn_label.borrow().as_ref() {
            label.set_text(name);
        }
        self.imp().current_feed_uri.replace(uri.to_string());
        self.rebuild_feed_list();
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

    /// Push a thread view page onto the current section's navigation stack
    pub fn push_thread_page(&self, post: &Post, thread_posts: Vec<Post>) {
        let nav_view = self.current_nav_view();
        let Some(nav_view) = nav_view else {
            return;
        };

        let page = self.build_thread_page(post, thread_posts);
        nav_view.push(&page);
    }

    /// Push a profile view page onto the current section's navigation stack
    pub fn push_profile_page(&self, profile: &Profile, posts: Vec<Post>) {
        let nav_view = self.current_nav_view();
        let Some(nav_view) = nav_view else {
            return;
        };

        let page = self.build_profile_page(profile, posts);
        nav_view.push(&page);
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
            _ => return,
        };

        if let Some(nav_view) = self.current_nav_view() {
            nav_view.pop_to_tag(root_tag);
        }
    }

    /// Get the NavigationView for the currently visible stack page
    fn current_nav_view(&self) -> Option<adw::NavigationView> {
        let stack = self.imp().main_stack.borrow();
        let stack = stack.as_ref()?;
        let visible_name = stack.visible_child_name()?;

        match visible_name.as_str() {
            "home" => self.imp().home_nav_view.borrow().clone(),
            "mentions" => self.imp().mentions_nav_view.borrow().clone(),
            _ => self.imp().home_nav_view.borrow().clone(),
        }
    }

    /// Switch to a top-level page (Home, Mentions, etc.) - no animation
    pub fn switch_to_page(&self, page_name: &str) {
        if let Some(stack) = self.imp().main_stack.borrow().as_ref() {
            stack.set_visible_child_name(page_name);
        }
    }

    /// Build a thread view page
    fn build_thread_page(&self, main_post: &Post, posts: Vec<Post>) -> adw::NavigationPage {
        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let title = gtk4::Label::new(Some("Thread"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));

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

        // Create scrollable content
        let scroll_content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Helper to create a post list section
        let create_post_list = |win: &Self, posts_to_show: Vec<Post>| -> gtk4::ListView {
            let model = gio::ListStore::new::<PostObject>();
            let factory = gtk4::SignalListItemFactory::new();

            factory.connect_setup(|_, item| {
                let post_row = PostRow::new();
                if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() {
                    list_item.set_child(Some(&post_row));
                }
            });

            let w = win.clone();
            factory.connect_bind(move |_, item| {
                if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                    && let Some(post_object) = list_item.item().and_downcast::<PostObject>()
                    && let Some(post) = post_object.post()
                    && let Some(post_row) = list_item.child().and_downcast::<PostRow>()
                {
                    post_row.bind(&post);
                    let post_for_like = post.clone();
                    let win = w.clone();
                    post_row.connect_like_clicked(move |row, was_liked, like_uri| {
                        let mut post_with_state = post_for_like.clone();
                        post_with_state.viewer_like = if was_liked { like_uri } else { None };
                        let row_weak = row.downgrade();
                        win.imp()
                            .like_callback
                            .borrow()
                            .as_ref()
                            .map(|cb| cb(post_with_state, row_weak));
                    });
                    let post_for_repost = post.clone();
                    let win = w.clone();
                    post_row.connect_repost_clicked(move |row, was_reposted, repost_uri| {
                        let mut post_with_state = post_for_repost.clone();
                        post_with_state.viewer_repost =
                            if was_reposted { repost_uri } else { None };
                        let row_weak = row.downgrade();
                        win.imp()
                            .repost_callback
                            .borrow()
                            .as_ref()
                            .map(|cb| cb(post_with_state, row_weak));
                    });
                    let post_for_quote = post.clone();
                    let win = w.clone();
                    post_row.connect_quote_clicked(move || {
                        win.imp()
                            .quote_callback
                            .borrow()
                            .as_ref()
                            .map(|cb| cb(post_for_quote.clone()));
                    });
                    let post_clone = post.clone();
                    let win = w.clone();
                    post_row.connect_reply_clicked(move || {
                        win.imp()
                            .reply_callback
                            .borrow()
                            .as_ref()
                            .map(|cb| cb(post_clone.clone()));
                    });
                }
            });

            for post in posts_to_show {
                model.append(&PostObject::new(post));
            }

            let selection = gtk4::NoSelection::new(Some(model));
            let list_view = gtk4::ListView::new(Some(selection), Some(factory));
            list_view.add_css_class("background");
            list_view
        };

        // Add parent posts (if any)
        if !parent_posts.is_empty() {
            let parent_list = create_post_list(self, parent_posts);
            scroll_content.append(&parent_list);
        }

        // Add the main post
        let main_list = create_post_list(self, vec![the_main_post.clone()]);
        scroll_content.append(&main_list);

        // Add "Posted {date}" separator
        let posted_separator = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        posted_separator.add_css_class("thread-separator");
        posted_separator.set_margin_top(8);
        posted_separator.set_margin_bottom(8);

        // Format the date: "Posted Sat, Jan 31, 2026 at 12:50 PM"
        let posted_text = Self::format_full_timestamp(&the_main_post.indexed_at);
        let posted_label = gtk4::Label::new(Some(&posted_text));
        posted_label.add_css_class("dim-label");
        posted_label.set_halign(gtk4::Align::Center);
        posted_separator.append(&posted_label);

        scroll_content.append(&posted_separator);

        // Add "Replies" section if there are replies
        if !reply_posts.is_empty() {
            let replies_label = gtk4::Label::new(Some("Replies"));
            replies_label.add_css_class("title-4");
            replies_label.set_halign(gtk4::Align::Start);
            replies_label.set_margin_start(16);
            replies_label.set_margin_top(8);
            replies_label.set_margin_bottom(8);
            scroll_content.append(&replies_label);

            let replies_list = create_post_list(self, reply_posts);
            scroll_content.append(&replies_list);
        }

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&scroll_content));
        content_box.append(&scrolled);

        let page = adw::NavigationPage::new(&content_box, "Thread");
        page.set_tag(Some("thread"));
        page
    }

    /// Format a timestamp as "Posted Sat, Jan 31, 2026 at 12:50 PM"
    fn format_full_timestamp(indexed_at: &str) -> String {
        if indexed_at.is_empty() {
            return "Posted".to_string();
        }

        let Ok(post_time) = chrono::DateTime::parse_from_rfc3339(indexed_at) else {
            return "Posted".to_string();
        };

        // Format: "Posted Sat, Jan 31, 2026 at 12:50 PM"
        format!(
            "Posted {}",
            post_time
                .format("%a, %b %d, %Y at %l:%M %p")
                .to_string()
                .trim()
        )
    }

    /// Build a profile view page
    fn build_profile_page(&self, profile: &Profile, posts: Vec<Post>) -> adw::NavigationPage {
        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);

        let display_name = profile.display_name.as_deref().unwrap_or(&profile.handle);
        let title = gtk4::Label::new(Some(display_name));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));

        content_box.append(&header);

        // Profile header section
        let profile_header = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        profile_header.set_margin_start(16);
        profile_header.set_margin_end(16);
        profile_header.set_margin_top(16);
        profile_header.set_margin_bottom(16);

        let avatar = adw::Avatar::new(80, Some(display_name), true);
        if let Some(avatar_url) = &profile.avatar {
            crate::ui::avatar_cache::load_avatar(avatar.clone(), avatar_url.clone());
        }
        avatar.set_halign(gtk4::Align::Center);
        profile_header.append(&avatar);

        let name_label = gtk4::Label::new(Some(display_name));
        name_label.add_css_class("title-1");
        name_label.set_halign(gtk4::Align::Center);
        profile_header.append(&name_label);

        let handle_label = gtk4::Label::new(Some(&format!("@{}", profile.handle)));
        handle_label.add_css_class("dim-label");
        handle_label.set_halign(gtk4::Align::Center);
        profile_header.append(&handle_label);

        content_box.append(&profile_header);

        // Separator
        let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        content_box.append(&separator);

        // Posts section label
        let posts_label = gtk4::Label::new(Some("Posts"));
        posts_label.add_css_class("title-4");
        posts_label.set_halign(gtk4::Align::Start);
        posts_label.set_margin_start(16);
        posts_label.set_margin_top(12);
        posts_label.set_margin_bottom(8);
        content_box.append(&posts_label);

        // Build posts list
        let model = gio::ListStore::new::<PostObject>();
        let factory = gtk4::SignalListItemFactory::new();

        factory.connect_setup(|_, item| {
            let post_row = PostRow::new();
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() {
                list_item.set_child(Some(&post_row));
            }
        });

        let win = self.clone();
        factory.connect_bind(move |_, item| {
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(post_object) = list_item.item().and_downcast::<PostObject>()
                && let Some(post) = post_object.post()
                && let Some(post_row) = list_item.child().and_downcast::<PostRow>()
            {
                post_row.bind(&post);
                // Wire up like/repost/reply/quote callbacks
                let post_for_like = post.clone();
                let w = win.clone();
                post_row.connect_like_clicked(move |row, was_liked, like_uri| {
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
                let w = win.clone();
                post_row.connect_repost_clicked(move |row, was_reposted, repost_uri| {
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
                let w = win.clone();
                post_row.connect_quote_clicked(move || {
                    w.imp()
                        .quote_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(post_for_quote.clone()));
                });
                let post_clone = post.clone();
                let w = win.clone();
                post_row.connect_reply_clicked(move || {
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

        let selection = gtk4::NoSelection::new(Some(model));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.add_css_class("background");

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&list_view));
        content_box.append(&scrolled);

        let page = adw::NavigationPage::new(&content_box, display_name);
        page.set_tag(Some("profile"));
        page
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

        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.set_tooltip_text(Some("Close"));
        close_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                window.close();
            }
        ));
        header.pack_end(&close_btn);

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

        let win = self.clone();
        factory.connect_bind(move |_, item| {
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(notif_object) = list_item.item().and_downcast::<NotificationObject>()
                && let Some(notif) = notif_object.notification()
                && let Some(row) = list_item.child().and_downcast::<MentionRow>()
            {
                row.bind(&notif);
                // Connect profile click
                let profile = notif.author.clone();
                let w = win.clone();
                row.connect_profile_clicked(move |_| {
                    w.imp()
                        .profile_clicked_callback
                        .borrow()
                        .as_ref()
                        .map(|cb| cb(profile.clone()));
                });
                // Connect post click (if there's an associated post)
                if let Some(post) = notif.post.clone() {
                    let w = win.clone();
                    row.connect_clicked(move |_| {
                        w.imp()
                            .post_clicked_callback
                            .borrow()
                            .as_ref()
                            .map(|cb| cb(post.clone()));
                    });
                }
            }
        });

        let selection = gtk4::NoSelection::new(Some(model.clone()));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.add_css_class("background");

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&list_view));
        overlay.set_child(Some(&scrolled));

        // Loading spinner
        let spinner = gtk4::Spinner::new();
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        mentions_box.append(&overlay);

        let imp = self.imp();
        imp.mentions_model.replace(Some(model));
        imp.mentions_scrolled_window.replace(Some(scrolled.clone()));
        imp.mentions_spinner.replace(Some(spinner));

        // Infinite scroll
        let adj = scrolled.vadjustment();
        adj.connect_value_changed(glib::clone!(
            @weak self as win => move |adj| {
                let value = adj.value();
                let upper = adj.upper();
                let page_size = adj.page_size();
                if value >= upper - page_size - 200.0 {
                    if let Some(cb) = win.imp().mentions_load_more_callback.borrow().as_ref() {
                        cb();
                    }
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
        if let Some(model) = self.imp().mentions_model.borrow().as_ref() {
            model.remove_all();
            for notif in notifications {
                model.append(&NotificationObject::new(notif));
            }
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
        if let Some(spinner) = self.imp().mentions_spinner.borrow().as_ref() {
            spinner.set_visible(loading);
            spinner.set_spinning(loading);
        }
    }

    /// Set callback for loading more mentions
    pub fn set_mentions_load_more_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .mentions_load_more_callback
            .replace(Some(Box::new(callback)));
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
            let win = self.clone();
            sidebar.connect_nav_changed(move |item| {
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
            let avatar_click = gtk4::GestureClick::new();
            let row_weak = self.downgrade();
            avatar_click.connect_released(move |_, _, _, _| {
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
    }

    impl Default for MentionRow {
        fn default() -> Self {
            Self::new()
        }
    }
}

use mention_row::MentionRow;
