# Hangar

![Hangar](https://cdn.cottle.cloud/hangar/hangar-icon.svg)

A native Bluesky client for Linux, built with Rust, GTK4, and Libadwaita. Learn more at [hangar.blue](https://hangar.blue).

> **Technical Preview**: Hangar is in early development. The foundations are being built feature by feature. Expect rough edges, missing functionality, and breaking changes.

![License](https://img.shields.io/badge/license-MPL--2.0-blue)
![Status](https://img.shields.io/badge/status-technical_preview-orange)
![Rust](https://img.shields.io/badge/rust-2024_edition-orange?logo=rust)
![GTK4](https://img.shields.io/badge/GTK4-Libadwaita-4a86cf?logo=gnome&logoColor=white)
![AT Protocol](https://img.shields.io/badge/AT_Protocol-Bluesky-0085ff?logo=bluesky&logoColor=white)
![Platform](https://img.shields.io/badge/platform-Linux-yellow?logo=linux&logoColor=white)
![CI](https://img.shields.io/github/actions/workflow/status/sethcottle/hangar/ci.yml?label=CI&logo=github)
![GitHub Release](https://img.shields.io/github/v/release/sethcottle/hangar?include_prereleases&logo=github)
![GitHub Downloads](https://img.shields.io/github/downloads/sethcottle/hangar/total?logo=github)
![GitHub last commit](https://img.shields.io/github/last-commit/sethcottle/hangar)
![GitHub issues](https://img.shields.io/github/issues/sethcottle/hangar)
![Repo size](https://img.shields.io/github/repo-size/sethcottle/hangar)

## What is Hangar?

Hangar is a desktop Bluesky client designed specifically for Linux and the GNOME desktop environment. No Electron, no web views, no hybrid UI. Just native GTK4 with Libadwaita for a fast, integrated experience.

### Goals

- **Purpose-built**: Built for Linux, feels at home in GNOME, with each feature being meticulously crafted
- **Performance**: Instant startup, smooth scrolling, efficient memory usage, optimized networking
- **Full Bluesky support**: Timeline, feeds, posts, interactions, notifications, and DMs

## Current Status

This is a **technical preview**. The app is functional but incomplete. Development follows a slice-by-slice methodology. One feature at a time, end-to-end.

### What Works

- **Authentication**: OAuth (PKCE + DPoP) with persistent sessions, app password fallback
- **Timeline**: Home feed with infinite scroll, cursor-based pagination, pull-to-refresh
- **Custom Feeds**: Feed selector with Following, Discover, and pinned feeds
- **Live Updates**: Background polling with seamless new post insertion
- **Rich Embeds**: Image grids (1–4+), external link cards, quote posts
- **Video**: In-app HLS playback (GStreamer `playbin3`) with seek, mute, and a persistent volume level
- **Interactions**: Like/unlike, repost/unrepost, quote, reply
- **Compose**: Rich text highlighting (mentions, hashtags, URLs), mention autocomplete, image attachments (up to 4 with alt text), link card preview (Open Graph), language selection, per-post content warnings, interaction settings, thread composer
- **Navigation**: Home, Mentions, Activity, Chat, Profile, Likes, Search tabs with drill-down views
- **Thread View**: Full thread with parent posts and replies
- **Profile View**: Own profile with banner, bio, follower/following/post counts; drill-down profiles from clicking avatars
- **Notifications**: Mentions tab with filtered notifications; Activity tab with badge overlays and embedded post cards
- **Chat**: Conversation list with unread badges and last message preview
- **Search**: Search tab with results list and interactions
- **Settings**: Display (post text size, color scheme), Accessibility (reduce motion), Account (content safety link, clear cache)
- **Caching**: SQLite cache for posts, feeds, profiles, images, notifications with per-user isolation and automatic eviction
- **Accessibility**: Accessible roles and labels, keyboard shortcuts (F5/Ctrl+R), reduced motion support, focus ring visibility, theme variable usage, non-color state indicators

### What's Next

- Follow/unfollow
- Chat message thread view and sending
- Keyboard shortcuts for navigation and post actions
- Enhanced screen reader support (composite labels, live regions, focus management)
- Image lightbox, loading skeletons
- Inline video playback in the timeline, with an autoplay preference
- Full profile view on drill-down, profile editing
- Internationalization (i18n) and Flatpak distribution
- Bookmarks, moderation tools, desktop notifications, multi-account support
- Lots of polish

## Authentication

### OAuth (Recommended)

Hangar uses **OAuth (PKCE + DPoP)** for authentication. When you sign in, a browser window opens to complete the authorization flow via your Bluesky PDS. Sessions are persisted locally and automatically refreshed.

### App Password (Fallback)

App password authentication is also supported as a fallback:
1. Go to [bsky.app/settings/app-passwords](https://bsky.app/settings/app-passwords)
2. Create a new app password
3. Use that password to log in to Hangar

App passwords can be revoked at any time without affecting your main account password.

### Session Storage

OAuth sessions are stored in a local file-based session store. App password sessions are stored securely using [libsecret](https://wiki.gnome.org/Projects/Libsecret) (the GNOME keyring). If libsecret/D-Bus is unavailable, app password session persistence will fail gracefully and you'll need to log in each time.

## Installation

### AppImage

Download the latest AppImage from the [Releases](https://github.com/sethcottle/hangar/releases) page, make it executable, and run:

```bash
chmod +x Hangar-x86_64.AppImage
./Hangar-x86_64.AppImage
```

The AppImage bundles no libraries. Besides GTK 4.12+ and libadwaita 1.5+, it
now needs the host's GStreamer 1.20+: `libgstreamer1.0-0`,
`libgstreamer-plugins-base1.0-0` and `libgstreamer-gl1.0-0` on Debian/Ubuntu,
`gstreamer1` and `gstreamer1-plugins-base` on Fedora, plus the playback
plugins listed under [Dependencies](#dependencies) for video. These are linked
in rather than loaded on demand, so the AppImage refuses to start without them
instead of merely losing video.

The AppImage supports delta updates via zsync and is also available through the [AM](https://github.com/ivan-hc/AM) AppImage package manager:

```bash
am -i hangar
```

### Flatpak (sideload)

Download the `.flatpak` bundle from the [Releases](https://github.com/sethcottle/hangar/releases) page and install:

```bash
flatpak install hangar.flatpak
```

Video decoding relies on `org.freedesktop.Platform.codecs-extra`, which flatpak
installs alongside `org.gnome.Platform` by default. The base runtime's
libavcodec has no H.264 decoder, so a runtime installed with `--no-related`
plays audio and shows nothing.

Flathub submission is planned but not yet available.

### Build from Source

#### Dependencies

Building requires **GTK 4.14+** and **libadwaita 1.5+** (GNOME 46+, Fedora 40+,
Ubuntu 24.04+). The resulting binary runs against **GTK 4.12+** and
**libadwaita 1.5+** (GNOME 45+), so release builds target an older floor than
the build host requires.

**Fedora/RHEL:**
```bash
sudo dnf install gtk4-devel libadwaita-devel gstreamer1-devel \
  gstreamer1-plugins-base-devel gcc pkg-config
```

**Ubuntu/Debian:**
```bash
sudo apt install libgtk-4-dev libadwaita-1-dev libgstreamer1.0-dev \
  libgstreamer-plugins-base1.0-dev build-essential pkg-config
```

**Arch:**
```bash
sudo pacman -S gtk4 libadwaita gstreamer gst-plugins-base base-devel
```

#### Runtime plugins for video

Playing Bluesky video needs GStreamer 1.20+ with these plugin sets installed at
runtime. `-good` carries `souphttpsrc` and `hlsdemux2`, `-bad` carries
`tsdemux`, and `gst-libav` carries `avdec_h264`.

- **Ubuntu/Debian:** `gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-libav`
- **Fedora:** `gstreamer1-plugins-good gstreamer1-plugins-bad-free gstreamer1-libav`
- **Arch:** `gst-plugins-good gst-plugins-bad gst-libav`

The GTK video sink is compiled into Hangar, so `gstreamer1.0-gtk4` is not
needed. When a stream cannot be played the viewer says so and offers to open
the post in a browser; it never does that on its own.

#### Build & Run

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

### Threading Model

GTK runs on the main thread. Network I/O runs on background threads with a shared Tokio runtime. Results are sent back to the main thread via `glib::timeout_add_local` polling or `glib::spawn_future_local`. A semaphore limits concurrent API requests to 4.

## Technology Stack

| Component | Technology |
|-----------|------------|
| Language | [Rust](https://www.rust-lang.org/) (2024 edition) |
| UI Toolkit | [GTK4](https://gtk.org/) + [Libadwaita](https://gnome.pages.gitlab.gnome.org/libadwaita/) |
| AT Protocol | [atrium-api](https://github.com/sugyan/atrium) + [atrium-oauth](https://github.com/sugyan/atrium) |
| HTTP Client | [reqwest](https://github.com/seanmonstar/reqwest) (rustls-tls) |
| Async Runtime | [Tokio](https://tokio.rs/) |
| Cache | [rusqlite](https://github.com/rusqlite/rusqlite) (SQLite, bundled) |
| Media | [gstreamer-rs](https://gitlab.freedesktop.org/gstreamer/gstreamer-rs) + [gst-plugin-gtk4](https://gitlab.freedesktop.org/gstreamer/gst-plugins-rs) (`playbin3`, `gtk4paintablesink` registered statically) |
| Secrets | [secret-service](https://crates.io/crates/secret-service) (libsecret bindings) |
| Serialization | [serde](https://serde.rs/) |

## Project Structure

```
src/
├── main.rs              # Entry point
├── app.rs               # Application lifecycle, data fetching, navigation
├── config.rs            # Constants (APP_ID, PDS URL)
├── runtime.rs           # Shared Tokio runtime
├── media/
│   └── mod.rs           # GStreamer init + playbin3 pipeline construction
├── atproto/
│   ├── client.rs        # HangarClient, the AT Protocol wrapper
│   ├── facets.rs        # Rich text facet parsing (mentions, links, hashtags)
│   └── types.rs         # Post, Profile, Session, Notification types
├── cache/
│   ├── db.rs            # SQLite database setup
│   ├── schema.rs        # Table definitions
│   ├── posts.rs         # Post cache operations
│   ├── feeds.rs         # Feed cache operations
│   └── profiles.rs      # Profile cache operations
├── state/
│   ├── oauth.rs         # OAuth flow (PKCE + DPoP, localhost callback)
│   ├── session.rs       # App password session persistence via libsecret
│   ├── session_store.rs # File-based OAuth session store
│   └── settings.rs      # AppSettings + FontSize (persistent JSON)
└── ui/
    ├── window.rs        # Main window, stack navigation, settings page
    ├── sidebar.rs       # Navigation rail with avatar menu
    ├── post_row.rs      # Post widget with embeds and actions
    ├── compose_dialog.rs # Rich compose (posts, replies, quotes, threads)
    ├── login_dialog.rs  # Sign-in dialog
    ├── media_viewer.rs  # Full-size image and video dialogs
    ├── video_player.rs  # Video widget: picture, transport controls, error page
    ├── avatar_cache.rs  # Image loading with LRU + SQLite caching
    └── style.css        # Custom CSS styles
```

## Support Hangar

If you find Hangar useful, consider supporting its development:

[![GitHub Sponsors](https://img.shields.io/badge/GitHub_Sponsors-♥-ea4aaa?logo=githubsponsors&logoColor=white)](https://github.com/sponsors/sethcottle)
[![Liberapay](https://img.shields.io/badge/Liberapay-donate-f6c915?logo=liberapay&logoColor=black)](https://en.liberapay.com/seth/)
[![Buy Me a Coffee](https://img.shields.io/badge/Buy_Me_a_Coffee-donate-ffdd00?logo=buymeacoffee&logoColor=black)](https://buymeacoffee.com/seth)
[![Ko-fi](https://img.shields.io/badge/Ko--fi-donate-ff5e5b?logo=kofi&logoColor=white)](https://ko-fi.com/sethcottle)
[![PayPal](https://img.shields.io/badge/PayPal-donate-003087?logo=paypal&logoColor=white)](https://www.paypal.com/paypalme/sethcottle)
[![Stripe](https://img.shields.io/badge/Stripe-donate-635bff?logo=stripe&logoColor=white)](https://donate.stripe.com/aFa8wweI4dBr5sm6Qd8g001)

## Contributing

Hangar is open source under the MPL-2.0 license. Contributions are welcome.

Before contributing:
- Run `cargo fmt` (required)
- Address `clippy` warnings
- Follow existing code patterns
- Keep changes focused, one feature per PR

**Resources:**
- [Brand assets and logos](https://hangar.blue/brand/)
- [Transparency report](https://hangar.blue/transparency/)
- [AI usage policy](https://hangar.blue/ai/)

## License

[Mozilla Public License 2.0](LICENSE)
