// SPDX-License-Identifier: MPL-2.0

use super::post_row::PostRow;
use super::sidebar::Sidebar;
use crate::atproto::Post;
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
        pub like_callback: RefCell<Option<Box<dyn Fn(String, String) + 'static>>>,
        pub repost_callback: RefCell<Option<Box<dyn Fn(String, String) + 'static>>>,
        pub compose_callback: RefCell<Option<Box<dyn Fn() + 'static>>>,
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
    }

    fn build_content(&self) -> gtk4::Box {
        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.set_hexpand(true);

        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);

        let feed_btn = gtk4::Button::new();
        let feed_label = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let title_label = gtk4::Label::new(Some("Following"));
        title_label.add_css_class("title");
        feed_label.append(&title_label);
        let dropdown_icon = gtk4::Image::from_icon_name("pan-down-symbolic");
        feed_label.append(&dropdown_icon);
        feed_btn.set_child(Some(&feed_label));
        feed_btn.add_css_class("flat");
        header.set_title_widget(Some(&feed_btn));

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

    fn build_timeline(&self) -> gtk4::ScrolledWindow {
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
                    let uri = post.uri.clone();
                    let cid = post.cid.clone();
                    let w = win.clone();
                    post_row.connect_like_clicked(move || {
                        w.imp().like_callback.borrow().as_ref().map(|cb| cb(uri.clone(), cid.clone()));
                    });
                    let uri = post.uri.clone();
                    let cid = post.cid.clone();
                    let w = win.clone();
                    post_row.connect_repost_clicked(move || {
                        w.imp().repost_callback.borrow().as_ref().map(|cb| cb(uri.clone(), cid.clone()));
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

        let imp = self.imp();
        imp.timeline_model.replace(Some(model));

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

        scrolled
    }

    pub fn set_load_more_callback<F: Fn() + 'static>(&self, callback: F) {
        self.imp()
            .load_more_callback
            .replace(Some(Box::new(callback)));
    }

    pub fn set_like_callback<F: Fn(String, String) + 'static>(&self, callback: F) {
        self.imp()
            .like_callback
            .replace(Some(Box::new(callback)));
    }

    pub fn set_repost_callback<F: Fn(String, String) + 'static>(&self, callback: F) {
        self.imp()
            .repost_callback
            .replace(Some(Box::new(callback)));
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
}
