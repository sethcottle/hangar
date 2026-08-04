// SPDX-License-Identifier: MPL-2.0

//! The report dialog: pick a reason, add optional context, submit.
//!
//! One dialog serves posts and accounts; the subject line says which.
//! Submit stays off until a reason is chosen, so an empty report cannot
//! leave the machine.

use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

/// The lexicon's report reasons, in the order the official clients ask.
const REASONS: &[(&str, &str, &str)] = &[
    (
        "Spam",
        "Excessive mentions, replies, or unwanted promotion",
        "com.atproto.moderation.defs#reasonSpam",
    ),
    (
        "Harassment",
        "Abusive, rude, or targeted at someone",
        "com.atproto.moderation.defs#reasonRude",
    ),
    (
        "Misleading",
        "Impersonation or false information",
        "com.atproto.moderation.defs#reasonMisleading",
    ),
    (
        "Unwanted Sexual Content",
        "Nudity or adult content that is unlabeled or unwelcome",
        "com.atproto.moderation.defs#reasonSexual",
    ),
    (
        "Illegal or Urgent",
        "Breaks the law or needs urgent attention",
        "com.atproto.moderation.defs#reasonViolation",
    ),
    (
        "Other",
        "Something else; say what below",
        "com.atproto.moderation.defs#reasonOther",
    ),
];

/// The pieces a test needs to drive the dialog without a pointer.
pub(crate) struct ReportDialogParts {
    pub dialog: adw::Dialog,
    pub reasons: Vec<(gtk4::CheckButton, &'static str)>,
    pub details: gtk4::TextView,
    pub submit: gtk4::Button,
}

impl ReportDialogParts {
    /// The reason the user picked, if any.
    pub fn selected_reason(&self) -> Option<&'static str> {
        self.reasons
            .iter()
            .find(|(check, _)| check.is_active())
            .map(|(_, reason)| *reason)
    }
}

pub(crate) fn build(subject_line: &str) -> ReportDialogParts {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let header = adw::HeaderBar::new();
    let title = gtk4::Label::new(Some("Report"));
    title.add_css_class("title");
    header.set_title_widget(Some(&title));
    let submit = gtk4::Button::with_label("Report");
    submit.add_css_class("destructive-action");
    submit.set_sensitive(false);
    submit.set_tooltip_text(Some("Pick a reason first"));
    header.pack_end(&submit);
    content.append(&header);

    let subject = gtk4::Label::new(Some(subject_line));
    subject.add_css_class("dim-label");
    subject.set_halign(gtk4::Align::Start);
    subject.set_margin_start(16);
    subject.set_margin_top(8);
    subject.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    content.append(&subject);

    let list = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    list.set_margin_start(16);
    list.set_margin_end(16);
    list.set_margin_top(8);

    let mut reasons = Vec::new();
    let mut first: Option<gtk4::CheckButton> = None;
    for (label, description, reason) in REASONS {
        let check = gtk4::CheckButton::new();
        let text = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let name = gtk4::Label::new(Some(label));
        name.set_halign(gtk4::Align::Start);
        text.append(&name);
        let desc = gtk4::Label::new(Some(description));
        desc.add_css_class("dim-label");
        desc.add_css_class("caption");
        desc.set_halign(gtk4::Align::Start);
        desc.set_wrap(true);
        desc.set_xalign(0.0);
        text.append(&desc);
        check.set_child(Some(&text));
        match &first {
            Some(f) => check.set_group(Some(f)),
            None => first = Some(check.clone()),
        }
        let submit_ref = submit.clone();
        check.connect_toggled(move |check| {
            if check.is_active() {
                submit_ref.set_sensitive(true);
                submit_ref.set_tooltip_text(None);
            }
        });
        list.append(&check);
        reasons.push((check, *reason));
    }
    content.append(&list);

    let details_label = gtk4::Label::new(Some("Anything else? (optional)"));
    details_label.add_css_class("heading");
    details_label.set_halign(gtk4::Align::Start);
    details_label.set_margin_start(16);
    details_label.set_margin_top(8);
    content.append(&details_label);

    let details_frame = gtk4::Frame::new(None);
    details_frame.set_margin_start(16);
    details_frame.set_margin_end(16);
    details_frame.set_margin_top(6);
    details_frame.set_margin_bottom(16);
    let details = gtk4::TextView::new();
    details.set_wrap_mode(gtk4::WrapMode::WordChar);
    details.set_top_margin(8);
    details.set_bottom_margin(8);
    details.set_left_margin(8);
    details.set_right_margin(8);
    details.set_size_request(-1, 72);
    details_frame.set_child(Some(&details));
    content.append(&details_frame);

    let dialog = adw::Dialog::builder()
        .title("Report")
        .content_width(400)
        .child(&content)
        .build();

    ReportDialogParts {
        dialog,
        reasons,
        details,
        submit,
    }
}

/// Show the dialog over `parent`; Submit hands over the reason and any
/// details, then closes.
pub fn present(
    parent: &impl IsA<gtk4::Widget>,
    subject_line: &str,
    on_submit: impl Fn(&'static str, String) + 'static,
) {
    let parts = std::rc::Rc::new(build(subject_line));
    let dialog = parts.dialog.clone();
    let submit = parts.submit.clone();
    let parts_for_click = parts.clone();
    submit.connect_clicked(move |_| {
        let Some(reason) = parts_for_click.selected_reason() else {
            return;
        };
        let buffer = parts_for_click.details.buffer();
        let text = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        on_submit(reason, text);
        dialog.close();
    });

    parts.dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Submit waits for a reason; every lexicon reason is offered; the
    /// pick reads back as the wire value.
    #[test]
    fn submit_waits_for_a_reason() {
        crate::ui::with_gtk(submit_waits_for_a_reason_body);
    }

    fn submit_waits_for_a_reason_body() {
        let parts = build("Reporting a post by @someone.bsky.social");
        assert_eq!(parts.reasons.len(), 6, "all lexicon reasons offered");
        assert!(!parts.submit.is_sensitive(), "no reason, no report");
        assert!(parts.selected_reason().is_none());

        parts.reasons[0].0.set_active(true);
        assert!(parts.submit.is_sensitive());
        assert_eq!(
            parts.selected_reason(),
            Some("com.atproto.moderation.defs#reasonSpam")
        );

        // Radio semantics: picking another replaces, never stacks.
        parts.reasons[4].0.set_active(true);
        assert_eq!(
            parts.selected_reason(),
            Some("com.atproto.moderation.defs#reasonViolation")
        );
    }
}
