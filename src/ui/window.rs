// SPDX-License-Identifier: MPL-2.0

use super::post_row::PostRow;
use super::sidebar::Sidebar;
use crate::atproto::{Post, SavedFeed};
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

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct HangarWindow {
        pub sidebar: RefCell<Option<Sidebar>>,
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

        let content = self.build_content();
        main_box.append(&content);

        self.set_content(Some(&main_box));

        let imp = self.imp();
        imp.sidebar.replace(Some(sidebar));

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

    fn build_content(&self) -> gtk4::Box {
        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);

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

        content_box
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

        timeline_box.append(&overlay);

        // Loading spinner at bottom
        let spinner_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spinner_box.set_halign(gtk4::Align::Center);
        spinner_box.set_margin_top(12);
        spinner_box.set_margin_bottom(12);
        let spinner = gtk4::Spinner::new();
        spinner.set_visible(false);
        spinner_box.append(&spinner);
        timeline_box.append(&spinner_box);

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
        self.imp()
            .like_callback
            .replace(Some(Box::new(callback)));
    }

    pub fn set_repost_callback<F: Fn(Post, glib::WeakRef<PostRow>) + 'static>(&self, callback: F) {
        self.imp()
            .repost_callback
            .replace(Some(Box::new(callback)));
    }

    pub fn set_quote_callback<F: Fn(Post) + 'static>(&self, callback: F) {
        self.imp()
            .quote_callback
            .replace(Some(Box::new(callback)));
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
                win.imp()
                    .compose_callback
                    .borrow()
                    .as_ref()
                    .map(|cb| cb());
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
}
