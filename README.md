# Hangar

![Hangar](https://cdn.cottle.cloud/hangar/hangar.png)

A native Bluesky client for Linux, built with Rust, GTK4, and Libadwaita.

> **Technical Preview**: Hangar is in early development. The foundations are being built feature by feature. Expect rough edges, missing functionality, and breaking changes.

![License](https://img.shields.io/badge/license-MPL--2.0-blue)

## What is Hangar?

Hangar is a desktop Bluesky client designed specifically for Linux and the GNOME desktop environment. No Electron, no web views, no hybrid UI—just native GTK4 with Libadwaita for a fast, integrated experience.

### Goals

- **Purpose-built**: Built for Linux, feels at home in GNOME, with each feature being meticulously crafted
- **Performance**: Instant startup, smooth scrolling, efficient memory usage, optimized networking
- **Full Bluesky support**: Timeline, feeds, posts, interactions, notifications, and DMs

## Current Status

This is a **technical preview**. The app is functional but incomplete. Development follows a slice-by-slice methodology. One feature at a time, end-to-end.

### What Works

- **Authentication**: Login with handle + app password, session persistence via libsecret
- **Timeline**: Home feed with infinite scroll, cursor-based pagination, pull-to-refresh
- **Custom Feeds**: Feed selector with Following, Discover, and pinned feeds
- **Live Updates**: Background polling with seamless new post insertion, "N new posts" banner
- **Rich Embeds**: Images (smart grid layouts), external links, quote posts, video thumbnails
- **Interactions**: Like, repost, quote, reply with visual state feedback
- **Compose**: Text posts, replies, quote posts
- **Navigation**: Home, Mentions, Activity, Chat, Profile, Likes tabs
- **Thread View**: Full thread with parents and replies
- **Profile View**: User profiles with posts

### What's Missing

- Image/media attachments in composer
- Rich text (facets) in composer
- Native video playback
- Link opening
- Being able to view DM converstaion details
- Rich search
- Follow/unfollow
- Viewing followers/following
- Profile editing
- OAuth authentication
- Bookmarks (requires OAuth)
- Lots of polish

## Security Notice

### Use an App Password

Hangar currently uses **handle + app password authentication**, not OAuth.

**Create a dedicated app password for Hangar:**
1. Go to [bsky.app/settings/app-passwords](https://bsky.app/settings/app-passwords)
2. Create a new app password
3. Use that password to log in to Hangar

App passwords can be revoked at any time without affecting your main account password.

### OAuth Migration Planned

OAuth support is on the roadmap. This will enable:
- More secure authentication flow
- Access to features like Bookmarks (stored off-protocol)
- Better session management

Until then, app passwords are the recommended authentication method.

### Session Storage

Login sessions are stored securely using [libsecret](https://wiki.gnome.org/Projects/Libsecret) (the GNOME keyring). If libsecret/D-Bus is unavailable, session persistence will fail gracefully and you'll need to log in each time.

## Building

### Dependencies

**Fedora/RHEL:**
```bash
sudo dnf install gtk4-devel libadwaita-devel gcc pkg-config
```

**Ubuntu/Debian:**
```bash
sudo apt install libgtk-4-dev libadwaita-1-dev build-essential pkg-config
```

**Arch:**
```bash
sudo pacman -S gtk4 libadwaita base-devel
```

### Build & Run

```bash
cargo build --release
cargo run --release
```

## Architecture Overview

```
┌─────────────────────────────────────────────┐
│                  UI Layer                   │
│          GTK4 + Libadwaita widgets          │
│    (window, sidebar, post_row, dialogs)     │
└─────────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────┐
│             Application Layer               │
│     Orchestrates login, data fetching,      │
│     navigation, state management            │
└─────────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────┐
│            AT Protocol Layer                │
│         HangarClient wraps atrium           │
│     Converts atrium types → app types       │
└─────────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────┐
│            External Crates                  │
│        atrium-api, reqwest, tokio           │
└─────────────────────────────────────────────┘
```

### Async Pattern

GTK runs on the main thread; network I/O runs on a dedicated Tokio worker thread. Communication happens via channels, the UI is never blocked by network requests.

## Technology Stack

| Component | Technology |
|-----------|------------|
| Language | [Rust](https://www.rust-lang.org/) (2024 edition) |
| UI Toolkit | [GTK4](https://gtk.org/) + [Libadwaita](https://gnome.pages.gitlab.gnome.org/libadwaita/) |
| AT Protocol | [atrium-api](https://github.com/sugyan/atrium) |
| HTTP Client | [reqwest](https://github.com/seanmonstar/reqwest) (rustls-tls) |
| Async Runtime | [Tokio](https://tokio.rs/) |
| Secrets | [secret-service](https://crates.io/crates/secret-service) (libsecret bindings) |
| Serialization | [serde](https://serde.rs/) |

## Project Structure

```
src/
├── main.rs              # Entry point
├── app.rs               # Application lifecycle, data fetching
├── config.rs            # Constants (APP_ID, PDS URL)
├── atproto/
│   ├── client.rs        # HangarClient - AT Protocol wrapper
│   └── types.rs         # Post, Profile, Session types
├── state/
│   └── session.rs       # Session persistence via libsecret
└── ui/
    ├── window.rs        # Main window, navigation
    ├── sidebar.rs       # Navigation rail
    ├── post_row.rs      # Post widget
    ├── compose_dialog.rs
    ├── login_dialog.rs
    └── avatar_cache.rs  # Image loading with LRU + disk caching
```

## Contributing

Hangar is open source under the MPL-2.0 license. Contributions are welcome.

Before contributing:
- Run `cargo fmt` (required)
- Address `clippy` warnings
- Follow existing code patterns
- Keep changes focused—one feature per PR

## License

[Mozilla Public License 2.0](LICENSE)
