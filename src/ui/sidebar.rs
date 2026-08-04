// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::collapsible_if)]

use crate::ui::avatar_cache;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita as adw;

/// Navigation item definition
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavItem {
    Home,
    Mentions,
    Activity,
    Chat,
    Profile,
    Likes,
    Bookmarks,
    Search,
}

impl NavItem {
    pub fn icon_name(&self) -> &'static str {
        match self {
            Self::Home => "go-home-symbolic",
            Self::Mentions => "system-users-symbolic",
            Self::Activity => "preferences-system-notifications-symbolic",
            Self::Chat => "chat-message-new-symbolic",
            Self::Profile => "avatar-default-symbolic",
            Self::Likes => "emote-love-symbolic",
            Self::Bookmarks => "user-bookmarks-symbolic",
            Self::Search => "system-search-symbolic",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Mentions => "Mentions",
            Self::Activity => "Activity",
            Self::Chat => "Chat",
            Self::Profile => "Profile",
            Self::Likes => "Likes",
            Self::Bookmarks => "Saved",
            Self::Search => "Search",
        }
    }

    pub fn all() -> &'static [NavItem] {
        &[
            Self::Home,
            Self::Mentions,
            Self::Activity,
            Self::Chat,
            Self::Profile,
            Self::Likes,
            Self::Bookmarks,
            Self::Search,
        ]
    }
}

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct Sidebar {
        pub avatar: RefCell<Option<adw::Avatar>>,
        pub avatar_menu_btn: RefCell<Option<gtk4::MenuButton>>,
        pub nav_list: RefCell<Option<gtk4::ListBox>>,
        pub selected_item: Cell<Option<NavItem>>,
        pub compose_btn: RefCell<Option<gtk4::Button>>,
        pub my_profile_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub settings_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub about_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        pub sign_out_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
        /// Unread badge per nav row, in `NavItem::all()` order.
        pub badge_labels: RefCell<Vec<gtk4::Label>>,
        /// The uncapped counts behind the badges.
        pub badge_counts: RefCell<Vec<u32>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Sidebar {
        const NAME: &'static str = "HangarSidebar";
        type Type = super::Sidebar;
        type ParentType = gtk4::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk4::AccessibleRole::Navigation);
        }
    }

    impl ObjectImpl for Sidebar {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_ui();
        }
    }

    impl WidgetImpl for Sidebar {}
    impl BoxImpl for Sidebar {}
}

glib::wrapper! {
    pub struct Sidebar(ObjectSubclass<imp::Sidebar>)
        @extends gtk4::Box, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Orientable;
}

impl Sidebar {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk4::Orientation::Vertical)
            .property("spacing", 0)
            .build()
    }

    fn setup_ui(&self) {
        // Narrower rail width
        self.set_width_request(88);
        self.add_css_class("sidebar-rail");

        // Accessible label for the navigation landmark
        self.update_property(&[gtk4::accessible::Property::Label("Main navigation")]);

        // Avatar at top: a MenuButton whose popover holds Settings and Sign Out
        let avatar_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        avatar_box.set_margin_top(12);
        avatar_box.set_margin_bottom(8);
        avatar_box.set_halign(gtk4::Align::Center);

        let avatar = adw::Avatar::new(48, None, true);

        // Build popover menu
        let popover_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        popover_box.set_margin_top(8);
        popover_box.set_margin_bottom(8);
        popover_box.set_margin_start(8);
        popover_box.set_margin_end(8);

        // My Profile item. The avatar reads as "me"; give it a way there.
        let my_profile_item = gtk4::Button::new();
        let my_profile_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        my_profile_content.append(&gtk4::Image::from_icon_name("avatar-default-symbolic"));
        my_profile_content.append(&gtk4::Label::new(Some("My Profile")));
        my_profile_item.set_child(Some(&my_profile_content));
        my_profile_item.add_css_class("flat");
        popover_box.append(&my_profile_item);

        // Settings item
        let settings_item = gtk4::Button::new();
        let settings_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        settings_content.append(&gtk4::Image::from_icon_name("emblem-system-symbolic"));
        settings_content.append(&gtk4::Label::new(Some("Settings")));
        settings_item.set_child(Some(&settings_content));
        settings_item.add_css_class("flat");
        popover_box.append(&settings_item);

        // About item
        let about_item = gtk4::Button::new();
        let about_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        about_content.append(&gtk4::Image::from_icon_name("help-about-symbolic"));
        about_content.append(&gtk4::Label::new(Some("About Hangar")));
        about_item.set_child(Some(&about_content));
        about_item.add_css_class("flat");
        popover_box.append(&about_item);

        // Separator
        let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        sep.set_margin_top(4);
        sep.set_margin_bottom(4);
        popover_box.append(&sep);

        // Sign Out item
        let sign_out_item = gtk4::Button::new();
        let sign_out_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        sign_out_content.append(&gtk4::Image::from_icon_name("system-log-out-symbolic"));
        sign_out_content.append(&gtk4::Label::new(Some("Sign Out")));
        sign_out_item.set_child(Some(&sign_out_content));
        sign_out_item.add_css_class("flat");
        popover_box.append(&sign_out_item);

        let popover = gtk4::Popover::new();
        popover.set_child(Some(&popover_box));
        popover.add_css_class("menu");
        popover.set_has_arrow(false);

        // MenuButton wrapping the avatar
        let avatar_menu_btn = gtk4::MenuButton::new();
        avatar_menu_btn.set_child(Some(&avatar));
        avatar_menu_btn.set_popover(Some(&popover));
        avatar_menu_btn.add_css_class("flat");
        avatar_menu_btn.add_css_class("circular");
        avatar_menu_btn.set_tooltip_text(Some("Account"));
        avatar_menu_btn.update_property(&[gtk4::accessible::Property::Label("Account menu")]);

        // Wire up my profile click
        let sidebar_weak = self.downgrade();
        let popover_ref = popover.clone();
        my_profile_item.connect_clicked(move |_| {
            popover_ref.popdown();
            if let Some(sidebar) = sidebar_weak.upgrade() {
                if let Some(cb) = sidebar.imp().my_profile_callback.borrow().as_ref() {
                    cb();
                }
            }
        });

        // Wire up settings click
        let sidebar_weak = self.downgrade();
        let popover_ref = popover.clone();
        settings_item.connect_clicked(move |_| {
            popover_ref.popdown();
            if let Some(sidebar) = sidebar_weak.upgrade() {
                if let Some(cb) = sidebar.imp().settings_callback.borrow().as_ref() {
                    cb();
                }
            }
        });

        // Wire up about click
        let sidebar_weak = self.downgrade();
        let popover_ref = popover.clone();
        about_item.connect_clicked(move |_| {
            popover_ref.popdown();
            if let Some(sidebar) = sidebar_weak.upgrade() {
                if let Some(cb) = sidebar.imp().about_callback.borrow().as_ref() {
                    cb();
                }
            }
        });

        // Wire up sign out click
        let sidebar_weak = self.downgrade();
        let popover_ref = popover.clone();
        sign_out_item.connect_clicked(move |_| {
            popover_ref.popdown();
            if let Some(sidebar) = sidebar_weak.upgrade() {
                if let Some(cb) = sidebar.imp().sign_out_callback.borrow().as_ref() {
                    cb();
                }
            }
        });

        avatar_box.append(&avatar_menu_btn);
        self.append(&avatar_box);

        let imp_ref = self.imp();
        imp_ref.avatar.replace(Some(avatar));
        imp_ref.avatar_menu_btn.replace(Some(avatar_menu_btn));

        // Navigation list
        let nav_list = gtk4::ListBox::new();
        nav_list.set_selection_mode(gtk4::SelectionMode::Single);
        nav_list.add_css_class("navigation-sidebar");
        nav_list.update_property(&[gtk4::accessible::Property::Label("Navigation menu")]);

        for item in NavItem::all() {
            let row = self.create_nav_row(*item);
            nav_list.append(&row);
        }

        // Select Home by default
        if let Some(first_row) = nav_list.row_at_index(0) {
            nav_list.select_row(Some(&first_row));
        }

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
        scrolled.set_child(Some(&nav_list));

        self.append(&scrolled);

        let imp = self.imp();
        imp.nav_list.replace(Some(nav_list));
        imp.selected_item.set(Some(NavItem::Home));

        // Compose button at bottom
        let compose_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        compose_box.set_margin_start(8);
        compose_box.set_margin_end(8);
        compose_box.set_margin_top(8);
        compose_box.set_margin_bottom(12);

        let compose_btn = gtk4::Button::new();
        compose_btn.set_child(Some(&self.create_compose_content()));
        compose_btn.add_css_class("suggested-action");
        compose_btn.add_css_class("circular");
        compose_btn.set_tooltip_text(Some("New Post"));
        compose_btn.update_property(&[gtk4::accessible::Property::Label("Compose new post")]);
        compose_btn.set_halign(gtk4::Align::Center);
        compose_btn.set_width_request(48);
        compose_btn.set_height_request(48);

        self.imp().compose_btn.replace(Some(compose_btn.clone()));
        compose_box.append(&compose_btn);
        self.append(&compose_box);
    }

    fn create_nav_row(&self, item: NavItem) -> gtk4::ListBoxRow {
        let row = gtk4::ListBoxRow::new();
        row.set_selectable(true);
        row.update_property(&[gtk4::accessible::Property::Label(item.label())]);

        // Vertical stack: icon on top, label below
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        content.set_halign(gtk4::Align::Center);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(4);
        content.set_margin_end(4);

        // Larger icon
        let icon = gtk4::Image::from_icon_name(item.icon_name());
        icon.set_icon_size(gtk4::IconSize::Large);
        icon.set_pixel_size(24);

        // Unread count pinned to the icon's corner. The row's accessible
        // label carries the count, so the badge itself stays out of the
        // a11y tree.
        let badge = gtk4::Label::builder()
            .visible(false)
            .can_target(false)
            .halign(gtk4::Align::End)
            .valign(gtk4::Align::Start)
            .accessible_role(gtk4::AccessibleRole::Presentation)
            .build();
        badge.add_css_class("unread-badge");

        let icon_overlay = gtk4::Overlay::new();
        icon_overlay.set_child(Some(&icon));
        icon_overlay.add_overlay(&badge);
        content.append(&icon_overlay);

        self.imp().badge_labels.borrow_mut().push(badge);
        self.imp().badge_counts.borrow_mut().push(0);

        // Small label underneath
        let label = gtk4::Label::new(Some(item.label()));
        label.add_css_class("caption");
        label.set_halign(gtk4::Align::Center);
        content.append(&label);

        row.set_child(Some(&content));
        row
    }

    fn create_compose_content(&self) -> gtk4::Image {
        let icon = gtk4::Image::from_icon_name("document-edit-symbolic");
        icon.set_pixel_size(24);
        icon
    }

    pub fn selected_item(&self) -> Option<NavItem> {
        self.imp().selected_item.get()
    }

    pub fn connect_compose_clicked<F: Fn() + 'static>(&self, callback: F) {
        if let Some(btn) = self.imp().compose_btn.borrow().as_ref() {
            btn.connect_clicked(move |_| callback());
        }
    }

    pub fn set_user_avatar(&self, display_name: &str, avatar_url: Option<&str>) {
        if let Some(avatar) = self.imp().avatar.borrow().as_ref() {
            avatar.set_text(Some(display_name));

            if let Some(url) = avatar_url {
                avatar_cache::load_avatar(avatar.clone(), url.to_string());
            }
        }
    }

    pub fn connect_nav_changed<F: Fn(NavItem) + 'static>(&self, callback: F) {
        if let Some(nav_list) = self.imp().nav_list.borrow().as_ref() {
            let sidebar_weak = self.downgrade();
            nav_list.connect_row_activated(move |_, row| {
                let index = row.index() as usize;
                if let Some(item) = NavItem::all().get(index) {
                    if let Some(sidebar) = sidebar_weak.upgrade() {
                        sidebar.imp().selected_item.set(Some(*item));
                    }
                    callback(*item);
                }
            });
        }
    }

    pub fn connect_my_profile_clicked<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .my_profile_callback
            .replace(Some(Box::new(callback)));
    }

    pub fn connect_settings_clicked<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .settings_callback
            .replace(Some(Box::new(callback)));
    }

    pub fn connect_about_clicked<F: Fn() + 'static>(&self, callback: F) {
        self.imp().about_callback.replace(Some(Box::new(callback)));
    }

    pub fn connect_sign_out_clicked<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .sign_out_callback
            .replace(Some(Box::new(callback)));
    }

    /// Show `count` unread on `item`'s row. Zero hides the badge.
    pub fn set_badge(&self, item: NavItem, count: u32) {
        let Some(index) = NavItem::all().iter().position(|i| *i == item) else {
            return;
        };
        let imp = self.imp();
        if let Some(stored) = imp.badge_counts.borrow_mut().get_mut(index) {
            if *stored == count {
                return;
            }
            *stored = count;
        }
        if let Some(badge) = imp.badge_labels.borrow().get(index) {
            badge.set_label(&badge_text(count));
            badge.set_visible(count > 0);
        }
        if let Some(nav_list) = imp.nav_list.borrow().as_ref() {
            if let Some(row) = nav_list.row_at_index(index as i32) {
                row.update_property(&[gtk4::accessible::Property::Label(&badge_a11y_label(
                    item, count,
                ))]);
            }
        }
    }

    /// The count behind `item`'s badge, uncapped.
    pub fn badge_count(&self, item: NavItem) -> u32 {
        NavItem::all()
            .iter()
            .position(|i| *i == item)
            .and_then(|index| self.imp().badge_counts.borrow().get(index).copied())
            .unwrap_or(0)
    }

    pub fn select_nav_item(&self, item: NavItem) {
        if let Some(nav_list) = self.imp().nav_list.borrow().as_ref() {
            if let Some(index) = NavItem::all().iter().position(|i| *i == item) {
                if let Some(row) = nav_list.row_at_index(index as i32) {
                    nav_list.select_row(Some(&row));
                    self.imp().selected_item.set(Some(item));
                }
            }
        }
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}

/// Badge text. Three digits would not fit on the rail.
fn badge_text(count: u32) -> String {
    if count > 99 {
        "99+".to_string()
    } else {
        count.to_string()
    }
}

/// The row's accessible name. Exact even when the badge shows 99+.
fn badge_a11y_label(item: NavItem, count: u32) -> String {
    if count == 0 {
        item.label().to_string()
    } else {
        format!("{}, {} unread", item.label(), count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Set, cap, and clear, without disturbing the other rows.
    #[test]
    fn a_badge_shows_caps_and_hides() {
        crate::ui::with_gtk(a_badge_shows_caps_and_hides_body);
    }

    fn a_badge_shows_caps_and_hides_body() {
        let sidebar = Sidebar::new();
        let index = NavItem::all()
            .iter()
            .position(|i| *i == NavItem::Mentions)
            .expect("Mentions is a nav row");

        assert!(
            !sidebar.imp().badge_labels.borrow()[index].is_visible(),
            "no unread, no badge"
        );

        sidebar.set_badge(NavItem::Mentions, 3);
        {
            let labels = sidebar.imp().badge_labels.borrow();
            assert!(labels[index].is_visible());
            assert_eq!(labels[index].label(), "3");
        }
        assert_eq!(sidebar.badge_count(NavItem::Mentions), 3);

        // The display caps; the stored count must not, or the a11y label
        // and the clearing logic would work from a lie.
        sidebar.set_badge(NavItem::Mentions, 150);
        assert_eq!(sidebar.imp().badge_labels.borrow()[index].label(), "99+");
        assert_eq!(sidebar.badge_count(NavItem::Mentions), 150);

        assert_eq!(
            sidebar.badge_count(NavItem::Chat),
            0,
            "other rows stay untouched"
        );

        sidebar.set_badge(NavItem::Mentions, 0);
        assert!(
            !sidebar.imp().badge_labels.borrow()[index].is_visible(),
            "zero hides the badge"
        );
        assert_eq!(sidebar.badge_count(NavItem::Mentions), 0);
    }

    /// GTK has no getter for accessible labels, so the string handed to the
    /// rows is checked directly.
    #[test]
    fn the_row_a11y_label_carries_the_true_count() {
        assert_eq!(badge_a11y_label(NavItem::Mentions, 0), "Mentions");
        assert_eq!(badge_a11y_label(NavItem::Mentions, 3), "Mentions, 3 unread");
        // Capped on screen, exact for the screen reader.
        assert_eq!(badge_a11y_label(NavItem::Chat, 150), "Chat, 150 unread");
    }
}
