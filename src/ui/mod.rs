// SPDX-License-Identifier: MPL-2.0

pub mod avatar_cache;
mod compose_dialog;
mod login_dialog;
pub mod post_row;
pub mod sidebar;
mod window;

pub use compose_dialog::{ComposeDialog, QuoteContext, ReplyContext};
pub use login_dialog::LoginDialog;
pub use sidebar::NavItem;
pub use window::HangarWindow;

/// Stable replacement for the nightly-only `str::floor_char_boundary`.
pub fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        s.len()
    } else {
        let mut i = index;
        while i > 0 && !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}
