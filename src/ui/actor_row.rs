// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::type_complexity)]

//! A search result row for a user, plus the object its list model holds.

use crate::atproto::Profile;
use crate::ui::avatar_cache;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita as adw;

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
        pub activated_callback: RefCell<Option<Box<dyn Fn(Profile) + 'static>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ActorRow {
        const NAME: &'static str = "HangarActorRow";
        type Type = super::ActorRow;
        type ParentType = gtk4::Box;
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
                let profile = row.imp().profile.borrow().clone();
                if let Some(profile) = profile
                    && let Some(cb) = row.imp().activated_callback.borrow().as_ref()
                {
                    cb(profile);
                }
            }
        });
        self.add_controller(click);

        let imp = self.imp();
        imp.avatar.replace(Some(avatar));
        imp.name_label.replace(Some(name_label));
        imp.handle_label.replace(Some(handle_label));
        imp.follow_label.replace(Some(follow_label));
        imp.bio_label.replace(Some(bio_label));
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

        let a11y_label = format!("{}, @{}", display_name, profile.handle);
        self.update_property(&[gtk4::accessible::Property::Label(&a11y_label)]);
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
}
