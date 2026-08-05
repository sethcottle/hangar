// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::type_complexity)]

//! A search result row for a user, plus the object its list model holds.

use crate::atproto::Profile;
use crate::ui::avatar_cache;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita as adw;

thread_local! {
    /// The signed-in user's DID; their own row hides the follow button.
    static VIEWER_DID: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
    /// What a Follow click does. One handler serves every row, same as the
    /// post row's delete and save handlers.
    static FOLLOW_HANDLER: std::cell::RefCell<
        Option<Box<dyn Fn(Profile, glib::WeakRef<ActorRow>)>>,
    > = const { std::cell::RefCell::new(None) };
}

/// Record whose rows hide the follow button. `None` on sign-out.
pub fn set_viewer_did(did: Option<&str>) {
    VIEWER_DID.with(|cell| {
        cell.replace(did.map(str::to_string));
    });
}

/// Install the app-level follow flow. The row comes along weakly so the
/// app can settle the button once the server answers.
pub fn set_follow_handler<F: Fn(Profile, glib::WeakRef<ActorRow>) + 'static>(handler: F) {
    FOLLOW_HANDLER.with(|cell| {
        cell.replace(Some(Box::new(handler)));
    });
}

mod actor_object {
    use super::*;
    use std::cell::RefCell;

    mod imp {
        use super::*;

        #[derive(Default)]
        pub struct ActorObject {
            pub profile: RefCell<Option<Profile>>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for ActorObject {
            const NAME: &'static str = "HangarActorObject";
            type Type = super::ActorObject;
            type ParentType = glib::Object;
        }

        impl ObjectImpl for ActorObject {}
    }

    glib::wrapper! {
        pub struct ActorObject(ObjectSubclass<imp::ActorObject>);
    }

    impl ActorObject {
        pub fn new(profile: Profile) -> Self {
            let obj: Self = glib::Object::builder().build();
            obj.imp().profile.replace(Some(profile));
            obj
        }

        pub fn profile(&self) -> Option<Profile> {
            self.imp().profile.borrow().clone()
        }

        /// Follow state settled after this object was built; a later
        /// rebind reads the current truth.
        pub fn set_viewer_following(&self, follow_uri: Option<String>) {
            if let Some(profile) = self.imp().profile.borrow_mut().as_mut() {
                profile.viewer_following = follow_uri;
            }
        }
    }
}

pub use actor_object::ActorObject;

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct ActorRow {
        pub profile: RefCell<Option<Profile>>,
        pub avatar: RefCell<Option<adw::Avatar>>,
        pub name_label: RefCell<Option<gtk4::Label>>,
        pub handle_label: RefCell<Option<gtk4::Label>>,
        pub follow_label: RefCell<Option<gtk4::Label>>,
        pub bio_label: RefCell<Option<gtk4::Label>>,
        pub follow_btn: RefCell<Option<gtk4::Button>>,
        /// The model object behind the current bind, so a follow that
        /// settles after a scroll still lands in the list's state.
        pub bound_object: RefCell<Option<glib::WeakRef<super::ActorObject>>>,
        pub activated_callback: RefCell<Option<Box<dyn Fn(Profile) + 'static>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ActorRow {
        const NAME: &'static str = "HangarActorRow";
        type Type = super::ActorRow;
        type ParentType = gtk4::Box;

        fn class_init(klass: &mut Self::Class) {
            // A plain box announces nothing; Group carries the row's label.
            klass.set_accessible_role(gtk4::AccessibleRole::Group);

            // Open the person, live while the row itself has focus. Focus on
            // the follow button inside keeps that button's own keys.
            klass.install_action("actor.open", None, |row, _, _| row.activate_row());
            let empty = gtk4::gdk::ModifierType::empty();
            klass.add_binding_action(gtk4::gdk::Key::Return, empty, "actor.open");
            klass.add_binding_action(gtk4::gdk::Key::KP_Enter, empty, "actor.open");
            klass.add_binding_action(gtk4::gdk::Key::space, empty, "actor.open");
        }
    }

    impl ObjectImpl for ActorRow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_ui();
        }
    }

    impl WidgetImpl for ActorRow {}
    impl BoxImpl for ActorRow {}
}

glib::wrapper! {
    pub struct ActorRow(ObjectSubclass<imp::ActorRow>)
        @extends gtk4::Box, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Orientable;
}

impl ActorRow {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk4::Orientation::Vertical)
            .property("spacing", 0)
            .build()
    }

    fn setup_ui(&self) {
        self.set_margin_start(12);
        self.set_margin_end(12);
        self.set_margin_top(8);
        self.set_margin_bottom(8);
        self.set_cursor_from_name(Some("pointer"));
        // A keyboard stop of its own, in a list view or a plain box alike.
        self.set_focusable(true);

        let main_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);

        let avatar = adw::Avatar::new(48, None, true);
        avatar.set_valign(gtk4::Align::Start);
        main_box.append(&avatar);

        let info_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        info_box.set_hexpand(true);
        info_box.set_valign(gtk4::Align::Center);

        let name_label = gtk4::Label::new(None);
        name_label.add_css_class("heading");
        name_label.set_halign(gtk4::Align::Start);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        info_box.append(&name_label);

        let handle_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);

        let handle_label = gtk4::Label::new(None);
        handle_label.add_css_class("dim-label");
        handle_label.add_css_class("caption");
        handle_label.set_halign(gtk4::Align::Start);
        handle_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        handle_row.append(&handle_label);

        // Follow relationship, read only. The client has no follow call yet.
        let follow_label = gtk4::Label::new(None);
        follow_label.add_css_class("dim-label");
        follow_label.add_css_class("caption");
        follow_label.set_visible(false);
        handle_row.append(&follow_label);

        info_box.append(&handle_row);

        // Wire text. It wraps and stops at two lines; the list has no
        // horizontal scrollbar to absorb an unbroken run.
        let bio_label = gtk4::Label::new(None);
        bio_label.set_wrap(true);
        bio_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        bio_label.set_natural_wrap_mode(gtk4::NaturalWrapMode::Word);
        bio_label.set_lines(2);
        bio_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        bio_label.set_xalign(0.0);
        bio_label.set_visible(false);
        info_box.append(&bio_label);

        main_box.append(&info_box);

        // Follow/unfollow without leaving the list. The button claims its
        // clicks, so the row's open-profile gesture stays out of it.
        let follow_btn = gtk4::Button::new();
        follow_btn.add_css_class("pill");
        follow_btn.set_valign(gtk4::Align::Center);
        follow_btn.set_visible(false);
        let row_weak = self.downgrade();
        follow_btn.connect_clicked(move |btn| {
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            let Some(profile) = row.imp().profile.borrow().clone() else {
                return;
            };
            // Show the hoped-for state, locked until the app reports back.
            super::window::HangarWindow::sync_follow_button(
                btn,
                profile.viewer_following.is_none(),
            );
            btn.set_sensitive(false);
            FOLLOW_HANDLER.with(|cell| {
                if let Some(handler) = cell.borrow().as_ref() {
                    handler(profile.clone(), row.downgrade());
                }
            });
        });
        main_box.append(&follow_btn);

        self.append(&main_box);

        let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        sep.set_margin_top(8);
        self.append(&sep);

        // Installed once. `bind` swaps the stored profile under it, so a
        // recycled row never stacks gestures.
        let click = gtk4::GestureClick::new();
        let row_weak = self.downgrade();
        click.connect_released(move |_, _, _, _| {
            if let Some(row) = row_weak.upgrade() {
                row.activate_row();
            }
        });
        self.add_controller(click);

        let imp = self.imp();
        imp.avatar.replace(Some(avatar));
        imp.name_label.replace(Some(name_label));
        imp.handle_label.replace(Some(handle_label));
        imp.follow_label.replace(Some(follow_label));
        imp.bio_label.replace(Some(bio_label));
        imp.follow_btn.replace(Some(follow_btn));
    }

    /// Show `profile`. Rebinding a recycled row overwrites everything the
    /// previous occupant set.
    pub fn bind(&self, profile: &Profile) {
        let imp = self.imp();
        imp.profile.replace(Some(profile.clone()));

        let display_name = profile
            .display_name
            .as_deref()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or(&profile.handle);

        if let Some(avatar) = imp.avatar.borrow().as_ref() {
            // A recycled row keeps the last photo otherwise.
            avatar.set_custom_image(gtk4::gdk::Paintable::NONE);
            avatar.set_text(Some(display_name));
            if let Some(url) = &profile.avatar {
                avatar_cache::load_avatar(avatar.clone(), url.clone());
            }
        }

        if let Some(label) = imp.name_label.borrow().as_ref() {
            label.set_text(display_name);
        }

        if let Some(label) = imp.handle_label.borrow().as_ref() {
            label.set_text(&format!("@{}", profile.handle));
        }

        if let Some(label) = imp.follow_label.borrow().as_ref() {
            let follow_text = match (&profile.viewer_following, &profile.viewer_followed_by) {
                (Some(_), Some(_)) => Some("· Mutuals"),
                (Some(_), None) => Some("· Following"),
                (None, Some(_)) => Some("· Follows you"),
                (None, None) => None,
            };
            match follow_text {
                Some(text) => {
                    label.set_text(text);
                    label.set_visible(true);
                }
                None => label.set_visible(false),
            }
        }

        if let Some(label) = imp.bio_label.borrow().as_ref() {
            match profile.description.as_deref().map(str::trim) {
                Some(bio) if !bio.is_empty() => {
                    label.set_text(bio);
                    label.set_visible(true);
                }
                _ => label.set_visible(false),
            }
        }

        if let Some(btn) = imp.follow_btn.borrow().as_ref() {
            let own_row = VIEWER_DID
                .with(|cell| cell.borrow().clone())
                .is_none_or(|did| did == profile.did);
            btn.set_visible(!own_row);
            // sync_follow_button re-enables a row recycled mid-flight.
            super::window::HangarWindow::sync_follow_button(
                btn,
                profile.viewer_following.is_some(),
            );
        }

        let a11y_label = format!("{}, @{}", display_name, profile.handle);
        self.update_property(&[gtk4::accessible::Property::Label(&a11y_label)]);
    }

    /// Remember the model object behind this bind; follow results write
    /// through so a scrolled-away row comes back current.
    pub fn set_bound_object(&self, object: &ActorObject) {
        self.imp().bound_object.replace(Some(object.downgrade()));
    }

    /// DID of the profile currently shown, for settling async results on
    /// the right occupant.
    pub fn profile_did(&self) -> Option<String> {
        self.imp().profile.borrow().as_ref().map(|p| p.did.clone())
    }

    /// Record the follow state the server confirmed: the row's profile,
    /// its button, and the model object all take it together.
    pub fn set_viewer_following(&self, follow_uri: Option<String>) {
        let imp = self.imp();
        if let Some(profile) = imp.profile.borrow_mut().as_mut() {
            profile.viewer_following = follow_uri.clone();
        }
        if let Some(btn) = imp.follow_btn.borrow().as_ref() {
            super::window::HangarWindow::sync_follow_button(btn, follow_uri.is_some());
        }
        if let Some(object) = imp
            .bound_object
            .borrow()
            .as_ref()
            .and_then(|weak| weak.upgrade())
        {
            let same = object
                .profile()
                .zip(imp.profile.borrow().clone())
                .is_some_and(|(a, b)| a.did == b.did);
            if same {
                object.set_viewer_following(follow_uri);
            }
        }
    }

    /// Open the person this row shows. The click gesture and the keyboard
    /// action both land here, so neither needs pointer coordinates.
    fn activate_row(&self) {
        let imp = self.imp();
        // Out of the borrow first; the callback can rebind this row.
        let profile = imp.profile.borrow().clone();
        let callback = imp.activated_callback.borrow();
        if let Some(profile) = profile
            && let Some(cb) = callback.as_ref()
        {
            cb(profile);
        }
    }

    /// Replace the activation callback, so a recycled row keeps exactly one.
    pub fn set_activated_callback<F: Fn(Profile) + 'static>(&self, callback: F) {
        self.imp()
            .activated_callback
            .replace(Some(Box::new(callback)));
    }
}

impl Default for ActorRow {
    fn default() -> Self {
        Self::new()
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

    /// A recycled row must not keep the previous profile's badge or bio.
    #[test]
    fn a_rebound_row_drops_the_previous_profiles_badge_and_bio() {
        crate::ui::with_gtk(a_rebound_row_drops_the_previous_profiles_badge_and_bio_body);
    }

    fn a_rebound_row_drops_the_previous_profiles_badge_and_bio_body() {
        let row = ActorRow::new();
        let imp = row.imp();

        let mut first = a_profile("first");
        first.display_name = Some("First".into());
        first.description = Some("Writes bios.".into());
        first.viewer_following = Some("at://did:plc:me/app.bsky.graph.follow/x".into());
        row.bind(&first);

        assert!(imp.bio_label.borrow().as_ref().unwrap().is_visible());
        assert!(imp.follow_label.borrow().as_ref().unwrap().is_visible());
        assert_eq!(imp.name_label.borrow().as_ref().unwrap().text(), "First");

        // No display name, no bio, no relationship. Everything optional
        // that the first bind turned on has to turn back off.
        let second = a_profile("second");
        row.bind(&second);

        assert!(!imp.bio_label.borrow().as_ref().unwrap().is_visible());
        assert!(!imp.follow_label.borrow().as_ref().unwrap().is_visible());
        assert_eq!(
            imp.name_label.borrow().as_ref().unwrap().text(),
            "second.bsky.social"
        );
        assert_eq!(
            imp.handle_label.borrow().as_ref().unwrap().text(),
            "@second.bsky.social"
        );
    }

    /// A keyboard-only user has to be able to land on a row and open it.
    #[test]
    fn the_row_opens_from_the_keyboard_action() {
        crate::ui::with_gtk(the_row_opens_from_the_keyboard_action_body);
    }

    fn the_row_opens_from_the_keyboard_action_body() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let row = ActorRow::new();
        let opened: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
        let sink = opened.clone();
        row.set_activated_callback(move |profile| sink.borrow_mut().push(profile.did));

        let person = a_profile("keyed");
        row.bind(&person);

        assert!(row.is_focusable(), "Tab has to be able to reach the row");
        assert_eq!(row.accessible_role(), gtk4::AccessibleRole::Group);

        // What Return, KP_Enter and space are bound to.
        WidgetExt::activate_action(&row, "actor.open", None).expect("the action is installed");
        assert_eq!(opened.borrow().as_slice(), ["did:plc:keyed"]);
    }

    /// The follow button reads the bound profile, reports one click with
    /// it, settles through the row into the model object, and stays off
    /// your own row.
    #[test]
    fn the_follow_button_serves_the_bound_profile_only() {
        crate::ui::with_gtk(the_follow_button_serves_the_bound_profile_only_body);
    }

    fn the_follow_button_serves_the_bound_profile_only_body() {
        use std::cell::RefCell;
        use std::rc::Rc;

        set_viewer_did(Some("did:plc:me"));
        let clicked: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
        let sink = clicked.clone();
        set_follow_handler(move |profile, _row| {
            sink.borrow_mut().push(profile.did);
        });

        let row = ActorRow::new();
        let btn = row.imp().follow_btn.borrow().clone().unwrap();

        let stranger = a_profile("stranger");
        let object = ActorObject::new(stranger.clone());
        row.bind(&stranger);
        row.set_bound_object(&object);

        assert!(btn.is_visible());
        assert_eq!(btn.label().as_deref(), Some("Follow"));

        btn.emit_clicked();
        assert_eq!(clicked.borrow().as_slice(), ["did:plc:stranger"]);
        assert_eq!(
            btn.label().as_deref(),
            Some("Following"),
            "the hoped-for state shows while the server is out"
        );
        assert!(!btn.is_sensitive());

        // The app settles it: profile, button, and model object agree.
        row.set_viewer_following(Some("at://did:plc:me/app.bsky.graph.follow/1".into()));
        assert!(btn.is_sensitive());
        assert_eq!(
            object.profile().unwrap().viewer_following.as_deref(),
            Some("at://did:plc:me/app.bsky.graph.follow/1"),
            "a later rebind must read the settled state"
        );

        // A recycled row leaves the lock behind with the old profile.
        let mut followed = a_profile("followed");
        followed.viewer_following = Some("at://did:plc:me/app.bsky.graph.follow/2".into());
        row.bind(&followed);
        assert!(btn.is_sensitive());
        assert_eq!(btn.label().as_deref(), Some("Following"));

        // Your own row offers no follow button.
        row.bind(&a_profile("me"));
        assert!(!btn.is_visible());

        set_follow_handler(|_, _| {});
        set_viewer_did(None);
    }
}
