// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::type_complexity)]
#![allow(clippy::option_map_unit_fn)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_else_if)]

use super::post_row::PostRow;
use super::sidebar::Sidebar;
use crate::atproto::{Conversation, Notification, Post, SavedFeed};
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
        pub new_posts_banner: RefCell<Option<gtk4::Button>>,
        pub new_posts_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub scrolled_window: RefCell<Option<gtk4::ScrolledWindow>>,
        pub saved_scroll_position: RefCell<f64>,
        pub feed_btn_label: RefCell<Option<gtk4::Label>>,
        pub feed_popover: RefCell<Option<gtk4::Popover>>,
        pub feed_list_box: RefCell<Option<gtk4::ListBox>>,
        pub feed_changed_callback: RefCell<Option<Box<dyn Fn(SavedFeed) + 'static>>>,
        pub saved_feeds: RefCell<Vec<SavedFeed>>,
        pub current_feed_uri: RefCell<String>,
        // Navigation callbacks
        pub post_clicked_callback: RefCell<Option<Box<dyn Fn(Post) + 'static>>>,
        pub profile_clicked_callback: RefCell<Option<Box<dyn Fn(Profile) + 'static>>>,
        pub mention_clicked_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
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
        pub conversation_clicked_callback: RefCell<Option<Box<dyn Fn(Conversation) + 'static>>>,
        // Current user info
        pub current_user_did: RefCell<Option<String>>,
        // Profile page state (for own profile in sidebar)
        pub profile_nav_view: RefCell<Option<adw::NavigationView>>,
        pub profile_page_model: RefCell<Option<gio::ListStore>>,
        pub profile_page_spinner: RefCell<Option<gtk4::Spinner>>,
        pub profile_page_scrolled: RefCell<Option<gtk4::ScrolledWindow>>,
        pub profile_load_more_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        // Store current profile for updating UI
        pub current_profile: RefCell<Option<Profile>>,
        // Profile page widgets (for updating header)
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
        // Search page state
        pub search_nav_view: RefCell<Option<adw::NavigationView>>,
        pub search_model: RefCell<Option<gio::ListStore>>,
        pub search_load_more_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub search_scrolled_window: RefCell<Option<gtk4::ScrolledWindow>>,
        pub search_spinner: RefCell<Option<gtk4::Spinner>>,
        pub search_entry: RefCell<Option<gtk4::SearchEntry>>,
        pub search_callback: RefCell<Option<Box<dyn Fn(String) + 'static>>>,
        // Toast overlay for notifications
        pub toast_overlay: RefCell<Option<adw::ToastOverlay>>,
        // Settings page
        pub font_size_css_provider: RefCell<Option<gtk4::CssProvider>>,
        pub font_size_changed_callback:
            RefCell<Option<Box<dyn Fn(crate::state::FontSize) + 'static>>>,
        pub reduce_motion_css_provider: RefCell<Option<gtk4::CssProvider>>,
        // Track which page was visible before settings (for back button)
        pub previous_page: RefCell<Option<String>>,
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
            .property("default-width", 620)
            .property("default-height", 780)
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
        let mentions_page = self.build_mentions_page();
        mentions_nav_view.add(&mentions_page);
        main_stack.add_named(&mentions_nav_view, Some("mentions"));

        // Activity section: NavigationView for thread/profile drill-down
        let activity_nav_view = adw::NavigationView::new();
        let activity_page = self.build_activity_page();
        activity_nav_view.add(&activity_page);
        main_stack.add_named(&activity_nav_view, Some("activity"));

        // Chat section: NavigationView for conversation drill-down
        let chat_nav_view = adw::NavigationView::new();
        let chat_page = self.build_chat_page();
        chat_nav_view.add(&chat_page);
        main_stack.add_named(&chat_nav_view, Some("chat"));

        // Profile section: NavigationView for own profile
        let profile_nav_view = adw::NavigationView::new();
        let profile_page = self.build_own_profile_page();
        profile_nav_view.add(&profile_page);
        main_stack.add_named(&profile_nav_view, Some("profile"));

        // Likes section: NavigationView for liked posts
        let likes_nav_view = adw::NavigationView::new();
        let likes_page = self.build_likes_page();
        likes_nav_view.add(&likes_page);
        main_stack.add_named(&likes_nav_view, Some("likes"));

        // Search section: NavigationView for search results
        let search_nav_view = adw::NavigationView::new();
        let search_page = self.build_search_page();
        search_nav_view.add(&search_page);
        main_stack.add_named(&search_nav_view, Some("search"));

        // Settings section
        let settings_page = self.build_settings_page();
        main_stack.add_named(&settings_page, Some("settings"));

        main_box.append(&main_stack);

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
        imp.search_nav_view.replace(Some(search_nav_view));

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

        factory.connect_bind(glib::clone!(
            #[strong(rename_to = win)]
            self,
            move |_, item| {
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
                        if let Some(cb) = w.imp().like_callback.borrow().as_ref() {
                            cb(post_with_state, row_weak);
                        }
                    });
                    // Repost callback receives (post_row, was_reposted, repost_uri) captured before toggle
                    let post_for_repost = post.clone();
                    let w = win.clone();
                    post_row.connect_repost_clicked(move |row, was_reposted, repost_uri| {
                        let mut post_with_state = post_for_repost.clone();
                        post_with_state.viewer_repost =
                            if was_reposted { repost_uri } else { None };
                        let row_weak = row.downgrade();
                        if let Some(cb) = w.imp().repost_callback.borrow().as_ref() {
                            cb(post_with_state, row_weak);
                        }
                    });
                    let post_for_quote = post.clone();
                    let w = win.clone();
                    post_row.connect_quote_clicked(move || {
                        if let Some(cb) = w.imp().quote_callback.borrow().as_ref() {
                            cb(post_for_quote.clone());
                        }
                    });
                    let post_clone = post.clone();
                    let w = win.clone();
                    post_row.connect_reply_clicked(move || {
                        if let Some(cb) = w.imp().reply_callback.borrow().as_ref() {
                            cb(post_clone.clone());
                        }
                    });
                    // Navigation callbacks
                    let w = win.clone();
                    post_row.set_post_clicked_callback(move |p| {
                        if let Some(cb) = w.imp().post_clicked_callback.borrow().as_ref() {
                            cb(p);
                        }
                    });
                    let w = win.clone();
                    post_row.set_profile_clicked_callback(move |profile| {
                        if let Some(cb) = w.imp().profile_clicked_callback.borrow().as_ref() {
                            cb(profile);
                        }
                    });
                    let w = win.clone();
                    post_row.set_mention_clicked_callback(move |handle| {
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

        // Wrap in AdwClamp for proper content width
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
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
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        timeline_box.append(&overlay);

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
                if let Some(cb) = win.imp().compose_callback.borrow().as_ref() {
                    cb();
                }
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
        if let Some(banner) = self.imp().new_posts_banner.borrow().as_ref() {
            let label_text = if count == 1 {
                "1 new post".to_string()
            } else if count > 99 {
                "99+ new posts".to_string()
            } else {
                format!("{} new posts", count)
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
        }
    }

    pub fn hide_new_posts_banner(&self) {
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

            // Clear and rebuild - this resets the ListView state properly
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
            let new_upper = adj.upper();
            let height_added = new_upper - old_upper;

            if height_added > 0.0 {
                // Add the height of new posts to maintain position
                let new_scroll = current_scroll + height_added;
                adj.set_value(new_scroll);
            }
        });
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
                    format!("{}...", &desc[..super::floor_char_boundary(desc, 57)])
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

    /// Set callback for when an @mention in post text is clicked (handle without @)
    pub fn set_mention_clicked_callback<F: Fn(String) + 'static>(&self, callback: F) {
        self.imp()
            .mention_clicked_callback
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

        let page = self.build_thread_page(post, thread_posts);
        nav_view.push(&page);
    }

    /// Push a profile view page onto the current section's navigation stack
    pub fn push_profile_page(&self, profile: &Profile, posts: Vec<Post>) {
        let nav_view = self.current_nav_view();
        let Some(nav_view) = nav_view else {
            return;
        };

        // Save current scroll position before navigating
        self.save_scroll_position();

        let page = self.build_profile_page(profile, posts);
        nav_view.push(&page);
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
            "search" => "search",
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
            "activity" => self.imp().activity_nav_view.borrow().clone(),
            "chat" => self.imp().chat_nav_view.borrow().clone(),
            "profile" => self.imp().profile_nav_view.borrow().clone(),
            "likes" => self.imp().likes_nav_view.borrow().clone(),
            "search" => self.imp().search_nav_view.borrow().clone(),
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
        // Hide default title buttons - we'll add explicit window controls
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

        // Create scrollable content - use a simple Box instead of multiple ListViews
        // to avoid height calculation issues with nested scrolling
        let scroll_content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        scroll_content.add_css_class("background");

        // Helper to create a PostRow with all callbacks wired up
        // `clickable` controls whether clicking the post opens its thread
        let create_post_row = |win: &Self, post: Post, clickable: bool| -> PostRow {
            let post_row = PostRow::new();
            post_row.bind(&post);

            // Like callback
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

            // Repost callback
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

            // Quote callback
            let post_for_quote = post.clone();
            let w = win.clone();
            post_row.connect_quote_clicked(move || {
                w.imp()
                    .quote_callback
                    .borrow()
                    .as_ref()
                    .map(|cb| cb(post_for_quote.clone()));
            });

            // Reply callback
            let post_clone = post.clone();
            let w = win.clone();
            post_row.connect_reply_clicked(move || {
                w.imp()
                    .reply_callback
                    .borrow()
                    .as_ref()
                    .map(|cb| cb(post_clone.clone()));
            });

            // Post click callback (only if clickable - main post in thread shouldn't re-open)
            if clickable {
                let w = win.clone();
                post_row.set_post_clicked_callback(move |p| {
                    if let Some(cb) = w.imp().post_clicked_callback.borrow().as_ref() {
                        cb(p);
                    }
                });
            }

            let w = win.clone();
            post_row.set_profile_clicked_callback(move |profile| {
                if let Some(cb) = w.imp().profile_clicked_callback.borrow().as_ref() {
                    cb(profile);
                }
            });

            let w = win.clone();
            post_row.set_mention_clicked_callback(move |handle| {
                if let Some(cb) = w.imp().mention_clicked_callback.borrow().as_ref() {
                    cb(handle);
                }
            });

            post_row
        };

        // Add parent posts (if any) - these are clickable to navigate to them
        for post in parent_posts {
            let post_row = create_post_row(self, post, true);
            scroll_content.append(&post_row);
        }

        // Add the main post - NOT clickable since we're already viewing it
        let main_row = create_post_row(self, the_main_post.clone(), false);
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
                let post_row = create_post_row(self, post, true);
                scroll_content.append(&post_row);
            }
        }

        // Add bottom padding to ensure last post isn't clipped
        let bottom_spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        bottom_spacer.set_height_request(24);
        scroll_content.append(&bottom_spacer);

        // Wrap in AdwClamp for proper content width (matching timeline)
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&scroll_content));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        // Disable horizontal scrolling - content should wrap/clip within the clamp
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&clamp));
        content_box.append(&scrolled);

        // Don't set a tag - each thread is unique and we want to allow multiple in the stack
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

        // Add window controls (close button) to the end
        let window_controls = gtk4::WindowControls::new(gtk4::PackType::End);
        header.pack_end(&window_controls);

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

        // Wrap in AdwClamp for proper content width
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&clamp));
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

        // Wrap in AdwClamp for proper content width
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&clamp));
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

        let win = self.clone();
        factory.connect_bind(move |_, item| {
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(notif_object) = list_item.item().and_downcast::<NotificationObject>()
                && let Some(notif) = notif_object.notification()
                && let Some(row) = list_item.child().and_downcast::<ActivityRow>()
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

        // Wrap in AdwClamp for proper content width
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&clamp));
        overlay.set_child(Some(&scrolled));

        // Loading spinner
        let spinner = gtk4::Spinner::new();
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        activity_box.append(&overlay);

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
        if let Some(model) = self.imp().activity_model.borrow().as_ref() {
            model.remove_all();
            for notif in notifications {
                model.append(&NotificationObject::new(notif));
            }
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
        if let Some(spinner) = self.imp().activity_spinner.borrow().as_ref() {
            spinner.set_visible(loading);
            spinner.set_spinning(loading);
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

        let win = self.clone();
        factory.connect_bind(move |_, item| {
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(convo_object) = list_item.item().and_downcast::<ConversationObject>()
                && let Some(convo) = convo_object.conversation()
                && let Some(row) = list_item.child().and_downcast::<ConversationRow>()
            {
                let my_did = win.imp().current_user_did.borrow();
                row.bind(&convo, my_did.as_deref());
                // Connect click
                let conversation = convo.clone();
                let w = win.clone();
                row.connect_clicked(move |_| {
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

        // Wrap in AdwClamp for proper content width
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&clamp));
        overlay.set_child(Some(&scrolled));

        // Loading spinner
        let spinner = gtk4::Spinner::new();
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        chat_box.append(&overlay);

        let imp = self.imp();
        imp.chat_model.replace(Some(model));
        imp.chat_scrolled_window.replace(Some(scrolled.clone()));
        imp.chat_spinner.replace(Some(spinner));

        // Infinite scroll
        let adj = scrolled.vadjustment();
        let win = self.clone();
        adj.connect_value_changed(move |adj| {
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
        if let Some(model) = self.imp().chat_model.borrow().as_ref() {
            model.remove_all();
            for convo in conversations {
                model.append(&ConversationObject::new(convo));
            }
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
        if let Some(spinner) = self.imp().chat_spinner.borrow().as_ref() {
            spinner.set_visible(loading);
            spinner.set_spinning(loading);
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

    /// Set callback for when Settings is clicked in the avatar popover
    pub fn set_settings_clicked_callback<F: Fn() + 'static>(&self, f: F) {
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            sidebar.connect_settings_clicked(f);
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
    fn build_own_profile_content(&self) -> gtk4::Box {
        let profile_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        profile_box.set_vexpand(true);

        // Scrollable content for entire profile
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);

        let scroll_content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Profile header section (banner, avatar, info, stats)
        let header_section = self.build_profile_header_section();
        scroll_content.append(&header_section);

        // Separator
        let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        scroll_content.append(&sep);

        // Posts label
        let posts_label = gtk4::Label::new(Some("Posts"));
        posts_label.add_css_class("title-4");
        posts_label.set_halign(gtk4::Align::Start);
        posts_label.set_margin_start(16);
        posts_label.set_margin_top(12);
        posts_label.set_margin_bottom(8);
        scroll_content.append(&posts_label);

        // Posts list
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

        let win = self.clone();
        factory.connect_bind(move |_, item| {
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(post_object) = list_item.item().and_downcast::<PostObject>()
                && let Some(post) = post_object.post()
                && let Some(post_row) = list_item.child().and_downcast::<PostRow>()
            {
                post_row.bind(&post);
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
                // Navigation callbacks for posts in profile
                let w = win.clone();
                post_row.set_post_clicked_callback(move |p| {
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
        overlay.set_child(Some(&list_view));

        // Loading spinner
        let spinner = gtk4::Spinner::new();
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        scroll_content.append(&overlay);
        scrolled.set_child(Some(&scroll_content));
        profile_box.append(&scrolled);

        // Store references
        let imp = self.imp();
        imp.profile_page_model.replace(Some(model));
        imp.profile_page_spinner.replace(Some(spinner));
        imp.profile_page_scrolled.replace(Some(scrolled.clone()));

        // Infinite scroll
        let adj = scrolled.vadjustment();
        let win = self.clone();
        adj.connect_value_changed(move |adj| {
            let value = adj.value();
            let upper = adj.upper();
            let page_size = adj.page_size();
            if value >= upper - page_size - 200.0 {
                if let Some(cb) = win.imp().profile_load_more_callback.borrow().as_ref() {
                    cb();
                }
            }
        });

        profile_box
    }

    /// Build the profile header section with banner, avatar, bio, and stats
    fn build_profile_header_section(&self) -> gtk4::Box {
        let header_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Banner placeholder (will be updated when profile loads)
        let banner_overlay = gtk4::Overlay::new();
        banner_overlay.set_height_request(150);

        let banner_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        banner_box.add_css_class("profile-banner-placeholder");
        banner_overlay.set_child(Some(&banner_box));

        // Avatar overlaid on banner (positioned at bottom)
        let avatar = adw::Avatar::new(80, None, true);
        avatar.set_halign(gtk4::Align::Start);
        avatar.set_valign(gtk4::Align::End);
        avatar.set_margin_start(16);
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
        name_label.set_halign(gtk4::Align::Start);
        info_box.append(&name_label);

        // Handle
        let handle_label = gtk4::Label::new(None);
        handle_label.add_css_class("dim-label");
        handle_label.set_halign(gtk4::Align::Start);
        info_box.append(&handle_label);

        // Bio/description
        let bio_label = gtk4::Label::new(None);
        bio_label.set_wrap(true);
        bio_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        bio_label.set_halign(gtk4::Align::Start);
        bio_label.set_xalign(0.0);
        bio_label.set_visible(false); // Hidden until profile loads
        info_box.append(&bio_label);

        // Stats row
        let stats_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
        stats_box.set_margin_top(8);

        // Followers
        let followers_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let followers_count = gtk4::Label::new(Some("0"));
        followers_count.add_css_class("heading");
        followers_box.append(&followers_count);
        let followers_static = gtk4::Label::new(Some("followers"));
        followers_static.add_css_class("dim-label");
        followers_box.append(&followers_static);
        stats_box.append(&followers_box);

        // Following
        let following_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let following_count = gtk4::Label::new(Some("0"));
        following_count.add_css_class("heading");
        following_box.append(&following_count);
        let following_static = gtk4::Label::new(Some("following"));
        following_static.add_css_class("dim-label");
        following_box.append(&following_static);
        stats_box.append(&following_box);

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
        header_box.append(&info_box);

        // Store widget references
        let imp = self.imp();
        imp.profile_name_label.replace(Some(name_label));
        imp.profile_handle_label.replace(Some(handle_label));
        imp.profile_bio_label.replace(Some(bio_label));
        imp.profile_avatar.replace(Some(avatar));
        imp.profile_followers_label.replace(Some(followers_count));
        imp.profile_following_label.replace(Some(following_count));
        imp.profile_posts_label.replace(Some(posts_count));

        header_box
    }

    /// Update the profile page header with profile data
    pub fn update_profile_header(&self, profile: &Profile) {
        let imp = self.imp();
        imp.current_profile.replace(Some(profile.clone()));

        let display_name = profile.display_name.as_deref().unwrap_or(&profile.handle);

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
            if let Some(bio) = &profile.description {
                label.set_text(bio);
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

    /// Set loading state for profile page
    pub fn set_profile_loading(&self, loading: bool) {
        if let Some(spinner) = self.imp().profile_page_spinner.borrow().as_ref() {
            spinner.set_visible(loading);
            spinner.set_spinning(loading);
        }
    }

    /// Set callback for loading more profile posts
    pub fn set_profile_load_more_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .profile_load_more_callback
            .replace(Some(Box::new(callback)));
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

        let win = self.clone();
        factory.connect_bind(move |_, item| {
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(post_object) = list_item.item().and_downcast::<PostObject>()
                && let Some(post) = post_object.post()
                && let Some(post_row) = list_item.child().and_downcast::<PostRow>()
            {
                post_row.bind(&post);
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
                // Navigation callbacks for posts in likes
                let w = win.clone();
                post_row.set_post_clicked_callback(move |p| {
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

        // Wrap in AdwClamp for proper content width
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&clamp));
        overlay.set_child(Some(&scrolled));

        // Loading spinner
        let spinner = gtk4::Spinner::new();
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        likes_box.append(&overlay);

        // Store references
        let imp = self.imp();
        imp.likes_model.replace(Some(model));
        imp.likes_scrolled_window.replace(Some(scrolled.clone()));
        imp.likes_spinner.replace(Some(spinner));

        // Infinite scroll
        let adj = scrolled.vadjustment();
        let win = self.clone();
        adj.connect_value_changed(move |adj| {
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
        if let Some(model) = self.imp().likes_model.borrow().as_ref() {
            model.remove_all();
            for post in posts {
                model.append(&PostObject::new(post));
            }
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
        if let Some(spinner) = self.imp().likes_spinner.borrow().as_ref() {
            spinner.set_visible(loading);
            spinner.set_spinning(loading);
        }
    }

    /// Set callback for loading more liked posts
    pub fn set_likes_load_more_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .likes_load_more_callback
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
        search_entry.set_placeholder_text(Some("Search posts…"));
        search_entry.set_margin_start(12);
        search_entry.set_margin_end(12);
        search_entry.set_margin_top(12);
        search_entry.set_margin_bottom(12);

        // Connect search activation (Enter key)
        let win = self.clone();
        search_entry.connect_activate(move |entry| {
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

        // Results list in overlay (for spinner)
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

        let win = self.clone();
        factory.connect_bind(move |_, item| {
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>()
                && let Some(post_object) = list_item.item().and_downcast::<PostObject>()
                && let Some(post) = post_object.post()
                && let Some(post_row) = list_item.child().and_downcast::<PostRow>()
            {
                post_row.bind(&post);
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
                // Navigation callbacks for posts in search results
                let w = win.clone();
                post_row.set_post_clicked_callback(move |p| {
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

        // Wrap in AdwClamp for proper content width
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(800);
        clamp.set_tightening_threshold(600);
        clamp.set_child(Some(&list_view));

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&clamp));
        overlay.set_child(Some(&scrolled));

        // Loading spinner
        let spinner = gtk4::Spinner::new();
        spinner.set_visible(false);
        spinner.set_halign(gtk4::Align::Center);
        spinner.set_valign(gtk4::Align::End);
        spinner.set_margin_bottom(16);
        overlay.add_overlay(&spinner);

        search_box.append(&overlay);

        // Store references
        let imp = self.imp();
        imp.search_model.replace(Some(model));
        imp.search_scrolled_window.replace(Some(scrolled.clone()));
        imp.search_spinner.replace(Some(spinner));

        // Infinite scroll
        let adj = scrolled.vadjustment();
        let win = self.clone();
        adj.connect_value_changed(move |adj| {
            let value = adj.value();
            let upper = adj.upper();
            let page_size = adj.page_size();
            if value >= upper - page_size - 200.0 {
                if let Some(cb) = win.imp().search_load_more_callback.borrow().as_ref() {
                    cb();
                }
            }
        });

        search_box
    }

    /// Show the search page (top-level navigation, instant switch)
    pub fn show_search_page(&self) {
        self.switch_to_page("search");
    }

    /// Set search results in the search list
    pub fn set_search_results(&self, posts: Vec<Post>) {
        if let Some(model) = self.imp().search_model.borrow().as_ref() {
            model.remove_all();
            for post in posts {
                model.append(&PostObject::new(post));
            }
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

    /// Clear search results
    pub fn clear_search_results(&self) {
        if let Some(model) = self.imp().search_model.borrow().as_ref() {
            model.remove_all();
        }
    }

    /// Focus the search entry
    pub fn focus_search_entry(&self) {
        if let Some(entry) = self.imp().search_entry.borrow().as_ref() {
            entry.grab_focus();
        }
    }

    /// Show a toast notification
    pub fn show_toast(&self, message: &str) {
        if let Some(overlay) = self.imp().toast_overlay.borrow().as_ref() {
            let toast = adw::Toast::new(message);
            toast.set_timeout(3); // 3 seconds
            overlay.add_toast(toast);
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

    // ======== Settings Page ========

    fn build_settings_page(&self) -> gtk4::Box {
        use crate::state::AppSettings;

        let outer_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        outer_box.set_hexpand(true);
        outer_box.set_vexpand(true);

        // Header bar with back button and "Settings" title
        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);
        let title = gtk4::Label::new(Some("Settings"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));

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
        outer_box.append(&header);

        // Split layout: sidebar list on the left, content stack on the right
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

        // Category entries: (id, label, icon)
        let categories: &[(&str, &str, &str)] = &[
            (
                "display",
                "Display",
                "preferences-desktop-appearance-symbolic",
            ),
            (
                "accessibility",
                "Accessibility",
                "preferences-desktop-accessibility-symbolic",
            ),
            ("account", "Account", "avatar-default-symbolic"),
        ];

        for &(id, label, icon_name) in categories {
            let row = gtk4::ListBoxRow::new();
            row.set_widget_name(id);
            let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
            row_box.set_margin_top(8);
            row_box.set_margin_bottom(8);
            row_box.set_margin_start(12);
            row_box.set_margin_end(12);
            let icon = gtk4::Image::from_icon_name(icon_name);
            icon.set_pixel_size(18);
            row_box.append(&icon);
            let lbl = gtk4::Label::new(Some(label));
            lbl.set_halign(gtk4::Align::Start);
            row_box.append(&lbl);
            row.set_child(Some(&row_box));
            row.update_property(&[gtk4::accessible::Property::Label(label)]);
            sidebar_list.append(&row);
        }

        sidebar_box.append(&sidebar_list);
        sidebar_scrolled.set_child(Some(&sidebar_box));

        // Separator between sidebar and content
        let separator = gtk4::Separator::new(gtk4::Orientation::Vertical);

        // ---- Content stack ----
        let content_stack = gtk4::Stack::new();
        content_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
        content_stack.set_transition_duration(150);
        content_stack.set_hexpand(true);
        content_stack.set_vexpand(true);

        // Load current settings once
        let current_settings = AppSettings::load();

        // Display page (in a ScrolledWindow)
        let display_scrolled = gtk4::ScrolledWindow::new();
        display_scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        let display_page = self.build_settings_display_tab(&current_settings);
        display_scrolled.set_child(Some(&display_page));
        content_stack.add_named(&display_scrolled, Some("display"));

        // Accessibility page (in a ScrolledWindow)
        let a11y_scrolled = gtk4::ScrolledWindow::new();
        a11y_scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        let a11y_page = self.build_settings_accessibility_tab(&current_settings);
        a11y_scrolled.set_child(Some(&a11y_page));
        content_stack.add_named(&a11y_scrolled, Some("accessibility"));

        // Account page (in a ScrolledWindow)
        let account_scrolled = gtk4::ScrolledWindow::new();
        account_scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        let account_page = self.build_settings_account_tab();
        account_scrolled.set_child(Some(&account_page));
        content_stack.add_named(&account_scrolled, Some("account"));

        // Wire sidebar selection to content stack
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
        outer_box.append(&split_box);

        outer_box
    }

    /// Build the Display tab contents for the settings page
    fn build_settings_display_tab(
        &self,
        current_settings: &crate::state::AppSettings,
    ) -> gtk4::Box {
        use crate::state::FontSize;

        let page_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        page_box.set_vexpand(true);

        // Settings content with clamp for proper width
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(600);
        clamp.set_margin_top(16);
        clamp.set_margin_bottom(16);
        clamp.set_margin_start(16);
        clamp.set_margin_end(16);

        let settings_box = gtk4::Box::new(gtk4::Orientation::Vertical, 24);

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
        settings_box.append(&appearance_group);

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
        settings_box.append(&display_group);

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

        clamp.set_child(Some(&settings_box));
        page_box.append(&clamp);
        page_box
    }

    /// Build the Accessibility tab contents for the settings page
    fn build_settings_accessibility_tab(
        &self,
        current_settings: &crate::state::AppSettings,
    ) -> gtk4::Box {
        let page_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        page_box.set_vexpand(true);

        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(600);
        clamp.set_margin_top(16);
        clamp.set_margin_bottom(16);
        clamp.set_margin_start(16);
        clamp.set_margin_end(16);

        let settings_box = gtk4::Box::new(gtk4::Orientation::Vertical, 24);

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
        settings_box.append(&motion_group);

        clamp.set_child(Some(&settings_box));
        page_box.append(&clamp);
        page_box
    }

    /// Build the Account tab contents for the settings page
    fn build_settings_account_tab(&self) -> gtk4::Box {
        let page_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        page_box.set_vexpand(true);

        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(600);
        clamp.set_margin_top(16);
        clamp.set_margin_bottom(16);
        clamp.set_margin_start(16);
        clamp.set_margin_end(16);

        let settings_box = gtk4::Box::new(gtk4::Orientation::Vertical, 24);

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
        safety_row.connect_activated(|_| {
            let _ = open::that("https://bsky.app/settings/content-and-media");
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
        moderation_row.connect_activated(|_| {
            let _ = open::that("https://bsky.app/moderation");
        });
        safety_group.add(&moderation_row);

        settings_box.append(&safety_group);

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
        settings_box.append(&data_group);

        clamp.set_child(Some(&settings_box));
        page_box.append(&clamp);
        page_box
    }

    /// Show the settings page (top-level navigation, instant switch)
    pub fn show_settings_page(&self) {
        // Remember current page so back button can return to it
        if let Some(stack) = self.imp().main_stack.borrow().as_ref() {
            if let Some(name) = stack.visible_child_name() {
                self.imp().previous_page.replace(Some(name.to_string()));
            }
        }

        // Deselect sidebar nav since settings isn't a nav item
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            if let Some(nav_list) = sidebar.imp().nav_list.borrow().as_ref() {
                nav_list.unselect_all();
            }
        }

        self.switch_to_page("settings");
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
                "search" => Some(crate::ui::sidebar::NavItem::Search),
                _ => Some(crate::ui::sidebar::NavItem::Home),
            };
            if let Some(item) = nav_item {
                sidebar.select_nav_item(item);
            }
        }

        self.switch_to_page(&previous);
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

        // Scale text labels only — NOT the container itself, which would cause
        // horizontal overflow from fixed-width child widgets (action buttons, etc.)
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

        provider.load_from_data(&css);
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
            provider.load_from_data(
                "* { animation-duration: 0s !important; transition-duration: 0s !important; }",
            );
        } else {
            // Clear the CSS — system prefers-reduced-motion still works via style.css
            provider.load_from_data("");
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
            pub post_time_label: RefCell<Option<gtk4::Label>>,
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

            // Avatar with badge overlay
            let avatar_overlay = gtk4::Overlay::new();
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
            let avatar_click = gtk4::GestureClick::new();
            let row_weak = self.downgrade();
            avatar_click.connect_released(move |_, _, _, _| {
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

            let post_spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            post_spacer.set_hexpand(true);
            post_header.append(&post_spacer);

            let post_time_label = gtk4::Label::new(None);
            post_time_label.add_css_class("dim-label");
            post_time_label.add_css_class("caption");
            post_header.append(&post_time_label);

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
            imp.post_time_label.replace(Some(post_time_label));
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

            // Badge icon based on reason
            let icon_name = match notification.reason.as_str() {
                "like" => "emblem-favorite-symbolic",
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

            // Action label - use Pango markup for bold name
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

                // Post time
                if let Some(label) = imp.post_time_label.borrow().as_ref() {
                    label.set_text(&Self::format_relative_time(&post.indexed_at));
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

            // Get the other participant(s) - filter out ourselves
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
