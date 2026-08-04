// SPDX-License-Identifier: MPL-2.0

//! A small pie-chart progress icon, the kind Builder shows beside a
//! running clone. A dim disc fills clockwise from twelve as the fraction
//! climbs, all in the theme's foreground color.

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

mod imp {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    pub struct ProgressIcon {
        pub fraction: Cell<f64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ProgressIcon {
        const NAME: &'static str = "HangarProgressIcon";
        type Type = super::ProgressIcon;
        type ParentType = gtk4::DrawingArea;
    }

    impl ObjectImpl for ProgressIcon {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_content_width(16);
            obj.set_content_height(16);
            obj.set_valign(gtk4::Align::Center);
            obj.set_halign(gtk4::Align::Center);
            obj.set_draw_func(|area, cr, width, height| {
                let Some(icon) = area.downcast_ref::<super::ProgressIcon>() else {
                    return;
                };
                let fraction = icon.fraction();
                let color = area.color();
                let (r, g, b, a) = (
                    color.red() as f64,
                    color.green() as f64,
                    color.blue() as f64,
                    color.alpha() as f64,
                );
                let cx = width as f64 / 2.0;
                let cy = height as f64 / 2.0;
                let radius = cx.min(cy);
                let tau = std::f64::consts::TAU;
                let top = -std::f64::consts::FRAC_PI_2;

                // The full disc, faint, so the goal shape is always there.
                cr.set_source_rgba(r, g, b, a * 0.15);
                cr.arc(cx, cy, radius, 0.0, tau);
                let _ = cr.fill();

                // The earned slice, solid, from twelve o'clock.
                if fraction > 0.0 {
                    cr.set_source_rgba(r, g, b, a);
                    cr.move_to(cx, cy);
                    cr.arc(cx, cy, radius, top, top + fraction * tau);
                    cr.close_path();
                    let _ = cr.fill();
                }
            });
        }
    }

    impl WidgetImpl for ProgressIcon {}
    impl DrawingAreaImpl for ProgressIcon {}
}

glib::wrapper! {
    pub struct ProgressIcon(ObjectSubclass<imp::ProgressIcon>)
        @extends gtk4::DrawingArea, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl ProgressIcon {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    /// How much of the pie is filled, clamped to 0..=1.
    pub fn set_fraction(&self, fraction: f64) {
        let fraction = fraction.clamp(0.0, 1.0);
        if self.imp().fraction.get() != fraction {
            self.imp().fraction.set(fraction);
            self.queue_draw();
        }
    }

    pub fn fraction(&self) -> f64 {
        self.imp().fraction.get()
    }
}

impl Default for ProgressIcon {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fraction clamps to its unit range and starts empty.
    #[test]
    fn the_fraction_stays_in_its_unit_range() {
        crate::ui::with_gtk(the_fraction_stays_in_its_unit_range_body);
    }

    fn the_fraction_stays_in_its_unit_range_body() {
        let icon = ProgressIcon::new();
        assert_eq!(icon.fraction(), 0.0, "starts empty");

        icon.set_fraction(0.4);
        assert_eq!(icon.fraction(), 0.4);

        icon.set_fraction(1.7);
        assert_eq!(icon.fraction(), 1.0, "never past full");

        icon.set_fraction(-0.3);
        assert_eq!(icon.fraction(), 0.0, "never negative");
    }
}
