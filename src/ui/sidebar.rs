// SPDX-License-Identifier: MPL-2.0

use gtk4::gdk;
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
            Self::Mentions => "mail-unread-symbolic",
            Self::Activity => "bell-outline-symbolic",
            Self::Chat => "user-available-symbolic",
            Self::Profile => "avatar-default-symbolic",
            Self::Likes => "emblem-favorite-symbolic",
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
            Self::Bookmarks => "Bookmarks",
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
        pub nav_list: RefCell<Option<gtk4::ListBox>>,
        pub selected_item: Cell<Option<NavItem>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Sidebar {
        const NAME: &'static str = "HangarSidebar";
        type Type = super::Sidebar;
        type ParentType = gtk4::Box;
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

        // Avatar at top
        let avatar_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        avatar_box.set_margin_top(12);
        avatar_box.set_margin_bottom(8);
        avatar_box.set_halign(gtk4::Align::Center);

        let avatar = adw::Avatar::new(40, None, true);
        avatar.set_tooltip_text(Some("Account"));
        avatar_box.append(&avatar);

        self.append(&avatar_box);

        self.imp().avatar.replace(Some(avatar));

        // Navigation list
        let nav_list = gtk4::ListBox::new();
        nav_list.set_selection_mode(gtk4::SelectionMode::Single);
        nav_list.add_css_class("navigation-sidebar");

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
        compose_btn.set_halign(gtk4::Align::Center);
        compose_btn.set_width_request(48);
        compose_btn.set_height_request(48);

        compose_box.append(&compose_btn);
        self.append(&compose_box);
    }

    fn create_nav_row(&self, item: NavItem) -> gtk4::ListBoxRow {
        let row = gtk4::ListBoxRow::new();
        row.set_selectable(true);

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
        content.append(&icon);

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

    pub fn set_user_avatar(&self, display_name: &str, avatar_url: Option<&str>) {
        if let Some(avatar) = self.imp().avatar.borrow().as_ref() {
            avatar.set_text(Some(display_name));

            if let Some(url) = avatar_url {
                Self::load_avatar(avatar.clone(), url.to_string());
            }
        }
    }

    fn load_avatar(avatar: adw::Avatar, url: String) {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                if let Ok(response) = reqwest::get(&url).await {
                    if let Ok(bytes) = response.bytes().await {
                        let _ = tx.send(bytes.to_vec());
                    }
                }
            });
        });

        glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
            match rx.try_recv() {
                Ok(bytes) => {
                    let glib_bytes = glib::Bytes::from(&bytes);
                    let stream = gtk4::gio::MemoryInputStream::from_bytes(&glib_bytes);

                    if let Ok(pixbuf) =
                        gdk::gdk_pixbuf::Pixbuf::from_stream(&stream, gtk4::gio::Cancellable::NONE)
                    {
                        let texture = gdk::Texture::for_pixbuf(&pixbuf);
                        avatar.set_custom_image(Some(&texture));
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}
