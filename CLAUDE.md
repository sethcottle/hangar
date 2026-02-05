# CLAUDE.md — Project Context for AI Assistants

## What is Hangar?

Hangar is a native Bluesky (AT Protocol) desktop client for Linux, built with Rust + GTK4 + Libadwaita. It targets GNOME desktops and aims to be hyper-accessible.

## Tech Stack

- **Language:** Rust (2024 edition)
- **UI:** GTK4 0.9 (features: `v4_6`) + Libadwaita 0.7 (features: `v1_5`)
- **Async:** Tokio (network I/O only — GTK main loop handles UI)
- **AT Protocol:** `atrium-api` + `atrium-xrpc-client`
- **Settings:** JSON file at `~/.config/io.github.sethcottle.Hangar/settings.json`
- **Secrets:** `secret-service` crate (Linux keyring via libsecret; does NOT work on macOS)
- **Cache:** SQLite via `rusqlite` (bundled)

## Project Layout

```
src/
├── main.rs              # Entry point
├── app.rs               # HangarApplication — lifecycle, login, data fetching, navigation
├── config.rs            # Constants (APP_ID, DEFAULT_PDS)
├── atproto/             # AT Protocol client + types
│   ├── client.rs        # HangarClient wrapper over atrium
│   └── types.rs         # Session, Post, Profile, Notification, etc.
├── cache/               # SQLite caching layer
├── state/
│   ├── session.rs       # SessionManager (libsecret)
│   └── settings.rs      # AppSettings + FontSize (persistent JSON)
├── runtime.rs           # Shared Tokio runtime
└── ui/
    ├── window.rs        # HangarWindow — Stack + NavigationViews, settings page
    ├── sidebar.rs       # Sidebar rail — NavItem enum, avatar menu, compose btn
    ├── post_row.rs      # PostRow — post display with embeds, actions, a11y labels
    ├── compose_dialog.rs # New post / reply / quote
    ├── login_dialog.rs  # Sign-in UI
    ├── avatar_cache.rs  # Image loading + caching worker thread
    └── style.css        # All custom CSS
DOCS/
├── architecture.md      # Detailed architecture reference
├── accessibility-plan.md # Comprehensive WCAG 2.1 AA accessibility plan
├── settings-roadmap.md  # Planned settings features + implementation status
├── progress.md          # Feature progress tracking
└── build.md             # Build instructions
```

## Architecture Patterns

### Threading Model
- **Main thread:** GTK main loop. All UI operations happen here.
- **Background threads:** `std::thread::spawn` for network calls. Results sent back via `glib::timeout_add_local` polling or `glib::spawn_future_local`.
- **Tokio runtime:** Shared via `runtime::block_on()` — only used inside background threads.
- **API concurrency:** `Semaphore::new(4)` limits concurrent API requests.

### Widget Pattern (GObject Subclassing)
All custom widgets follow the gtk4-rs GObject pattern:
```rust
mod imp {
    pub struct MyWidget { /* RefCell fields */ }

    #[glib::object_subclass]
    impl ObjectSubclass for MyWidget {
        const NAME: &'static str = "HangarMyWidget";
        type Type = super::MyWidget;
        type ParentType = gtk4::Box; // or adw::Dialog, etc.

        fn class_init(klass: &mut Self::Class) {
            // Set accessible role here if needed
            klass.set_accessible_role(gtk4::AccessibleRole::Group);
        }
    }
    impl ObjectImpl for MyWidget { fn constructed(&self) { ... } }
    impl WidgetImpl for MyWidget {}
    impl BoxImpl for MyWidget {}
}
```

### Settings Persistence
`AppSettings` serializes to JSON. Load with `AppSettings::load()`, save with `settings.save()`. The `FontSize` struct wraps an `f64` scale factor (0.8–1.2, step 0.05).

### CSS Architecture
- Base styles in `style.css`, loaded at `STYLE_PROVIDER_PRIORITY_APPLICATION`.
- Dynamic font-size CSS applied at `STYLE_PROVIDER_PRIORITY_APPLICATION + 1` (overrides base).
- Uses Adwaita CSS variables (`@accent_color`, `@window_bg_color`, etc.) — **never hardcode hex colors**.

## Accessibility Requirements (CRITICAL)

Hangar aims to be **hyper-accessible**. Every change must follow these rules:

### When Adding New Widgets
1. **Set accessible role** in `class_init()` if the widget has a semantic role (Group, Navigation, etc.)
2. **Set accessible label** via `widget.update_property(&[gtk4::accessible::Property::Label("...")])` on all interactive elements
3. **Add tooltips** on icon-only buttons: `btn.set_tooltip_text(Some("Label"))`
4. **Use theme variables** in CSS — never raw hex colors
5. **Ensure focus visibility** — don't suppress focus rings

### When Adding Action Buttons
```rust
// Always set both tooltip and accessible label
btn.set_tooltip_text(Some("Reply"));
btn.update_property(&[gtk4::accessible::Property::Label("Reply")]);
```

### When Binding Post Data
Update accessible labels dynamically in `bind()`:
```rust
// Composite label for screen readers
let article_label = format!("Post by {} (@{}), {}: {}",
    display_name, handle, timestamp, text_preview);
self.update_property(&[
    gtk4::accessible::Property::Label(&article_label),
    gtk4::accessible::Property::RoleDescription("post"),
]);

// Action buttons with state
let like_label = if is_liked {
    format!("Unlike. {} likes", count)
} else {
    format!("Like. {} likes", count)
};
btn.update_property(&[gtk4::accessible::Property::Label(&like_label)]);
```

### When Toggling State (like/repost)
Update the accessible label to reflect the new state:
```rust
btn.update_property(&[gtk4::accessible::Property::Label(
    &format!("Unlike. {} likes", new_count),
)]);
```

### CSS Accessibility Rules
- `@media (prefers-reduced-motion: reduce)` — already in style.css
- Links must have `text-decoration: underline` (color alone is not sufficient per WCAG 1.4.1)
- Liked/reposted states must have non-color indicators (we use `-gtk-icon-style: filled`)
- Minimum target size: 36×36px for action buttons, 44×44px ideal (WCAG 2.5.5)
- Focus rings: `outline: 2px solid @accent_color` on interactive elements

### Available Accessible Roles (v4_6 compatible)
Use these roles without needing feature flag changes:
- `Group` — for semantic containers (PostRow uses this + RoleDescription "post")
- `Navigation` — for nav landmarks (Sidebar)
- `Alert` — for error messages (requires subclassing)
- `Img`, `Link`, `Button`, `List`, `ListItem`, `Heading`, `Search`, `Status`

Roles requiring `v4_14` feature flag (NOT currently enabled):
- `Article`, `Comment`, `Application`, `ToggleButton`, `Paragraph`, `BlockQuote`

### Reference Docs
- Full accessibility plan: `DOCS/accessibility-plan.md`
- Settings with accessibility features: `DOCS/settings-roadmap.md`
- GTK4 accessible API: `gtk4::accessible::Property`, `gtk4::accessible::State`, `gtk4::AccessibleRole`

## Key Implementation Notes

### Settings Page
- Accessed via avatar popover menu (not a nav item — too prominent)
- Back button returns to previous page; sidebar deselects when in settings
- Post Text Size slider with live preview card
- Settings page has its own window controls (must include `gtk4::WindowControls::new(gtk4::PackType::End)`)

### Sign Out Flow
Closes window entirely, drops all UI state, and re-activates the app to create a fresh window. Session cleared from keyring via background thread.

### Navigation
- Sidebar `NavItem` enum: Home, Mentions, Activity, Chat, Profile, Likes, Search
- Main content uses `gtk4::Stack` for top-level pages
- Each section has its own `adw::NavigationView` for drill-down (thread view, profile view)
- Settings is a separate stack page (not a NavigationView)

### Image Handling
- `avatar_cache.rs` manages a worker thread for fetching/caching images
- Images cached in SQLite blob storage
- `load_avatar()` and `load_image_into_picture()` are the main entry points

### Post Embeds
- Handled in `post_row.rs` via `render_embed()` → dispatches to `render_images()`, `render_external_card()`, `render_video()`, `render_quote()`
- Image alt text comes from `ImageEmbed.alt` field — must be surfaced to screen readers (partially done, needs ALT badge overlay)

## Performance Notes
- `update_property()` calls are synchronous metadata sets — zero performance impact
- Avoid blocking the main thread — always use background threads for network/disk I/O
- `API_SEMAPHORE` (4 permits) prevents request flooding during rapid scrolling
- Dev builds (`cargo run`) are significantly slower than release builds (`cargo run --release`)
