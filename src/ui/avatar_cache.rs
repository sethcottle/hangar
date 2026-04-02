// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::type_complexity)]

//! Concurrent image loading with a dedicated runtime.
//! Uses parallel fetching for fast avatar loading with deduplication.

use crate::cache::CacheDb;
use gtk4::gdk;
use gtk4::glib;
use libadwaita as adw;
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

/// Maximum concurrent downloads
const MAX_CONCURRENT: usize = 8;

/// In-memory cache for loaded images (URL → bytes)
static AVATAR_CACHE: Lazy<Arc<RwLock<HashMap<String, Vec<u8>>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

/// URLs currently being fetched (deduplication set)
static IN_FLIGHT: Lazy<Arc<RwLock<HashSet<String>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashSet::new())));

/// Dedicated runtime for image fetching - isolated from main app runtime
static IMAGE_RUNTIME: Lazy<tokio::runtime::Runtime> = Lazy::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("hangar-images")
        .build()
        .expect("failed to create image runtime")
});

/// Semaphore to limit concurrent downloads
static DOWNLOAD_SEMAPHORE: Lazy<Arc<tokio::sync::Semaphore>> =
    Lazy::new(|| Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT)));

/// Reference to cache database (for API compatibility)
static CACHE_DB: Lazy<RwLock<Option<Arc<CacheDb>>>> = Lazy::new(|| RwLock::new(None));

fn apply_avatar_bytes(avatar: &adw::Avatar, bytes: &[u8]) {
    let glib_bytes = glib::Bytes::from(bytes);
    let stream = gtk4::gio::MemoryInputStream::from_bytes(&glib_bytes);

    if let Ok(pixbuf) = gdk::gdk_pixbuf::Pixbuf::from_stream(&stream, gtk4::gio::Cancellable::NONE)
    {
        let texture = gdk::Texture::for_pixbuf(&pixbuf);
        avatar.set_custom_image(Some(&texture));
    }
}

fn apply_bytes_to_picture(picture: &gtk4::Picture, bytes: &[u8]) {
    let glib_bytes = glib::Bytes::from(bytes);
    let stream = gtk4::gio::MemoryInputStream::from_bytes(&glib_bytes);

    if let Ok(pixbuf) = gdk::gdk_pixbuf::Pixbuf::from_stream(&stream, gtk4::gio::Cancellable::NONE)
    {
        let texture = gdk::Texture::for_pixbuf(&pixbuf);
        picture.set_paintable(Some(&texture));
    }
}

/// Initialize the avatar cache with a database reference
pub fn init(cache_db: Arc<CacheDb>) {
    let mut db = CACHE_DB.write().unwrap();
    *db = Some(cache_db);
}

/// Perform cache cleanup (no-op for now)
pub fn cleanup_cache() {
    // Currently no-op since we're using simple in-memory cache
}

/// Ensure CDN URLs request JPEG format (GdkPixbuf may lack WebP/AVIF loaders)
fn ensure_jpeg_format(url: &str) -> String {
    if url.contains("cdn.bsky.app/img/") && !url.contains('@') {
        format!("{}@jpeg", url)
    } else {
        url.to_string()
    }
}

/// Spawn a fetch for the given URL if one isn't already in progress.
/// Returns true if a new fetch was spawned, false if deduplicated.
fn spawn_fetch_if_needed(url: &str) -> bool {
    // Check if already in flight
    {
        let in_flight = IN_FLIGHT.read().unwrap();
        if in_flight.contains(url) {
            return false;
        }
    }

    // Mark as in flight
    IN_FLIGHT.write().unwrap().insert(url.to_string());

    let url_clone = url.to_string();
    let cache = Arc::clone(&AVATAR_CACHE);
    let semaphore = Arc::clone(&DOWNLOAD_SEMAPHORE);
    let in_flight = Arc::clone(&IN_FLIGHT);

    IMAGE_RUNTIME.spawn(async move {
        let _permit = match semaphore.acquire().await {
            Ok(p) => p,
            Err(_) => {
                in_flight.write().unwrap().remove(&url_clone);
                return;
            }
        };

        let result: Option<Vec<u8>> = async {
            let response = reqwest::get(&url_clone).await.ok()?;
            if !response.status().is_success() {
                return None;
            }
            let bytes = response.bytes().await.ok()?;
            Some(bytes.to_vec())
        }
        .await;

        if let Some(bytes) = result {
            cache.write().unwrap().insert(url_clone.clone(), bytes);
        }

        // Remove from in-flight set
        in_flight.write().unwrap().remove(&url_clone);
    });

    true
}

/// Load an avatar image from URL
pub fn load_avatar(avatar: adw::Avatar, url: String) {
    let url = ensure_jpeg_format(&url);

    // Check memory cache first — instant hit
    if let Some(cached) = AVATAR_CACHE.read().unwrap().get(&url).cloned() {
        apply_avatar_bytes(&avatar, &cached);
        return;
    }

    // Kick off a fetch if not already in progress (deduplicates)
    spawn_fetch_if_needed(&url);

    // Poll the cache on the GTK main thread until the image arrives
    let poll_url = url.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        if let Some(bytes) = AVATAR_CACHE.read().unwrap().get(&poll_url).cloned() {
            apply_avatar_bytes(&avatar, &bytes);
            glib::ControlFlow::Break
        } else {
            // Check if fetch is still in progress
            let still_in_flight = IN_FLIGHT.read().unwrap().contains(&poll_url);
            if still_in_flight {
                glib::ControlFlow::Continue
            } else {
                // Fetch completed but no data in cache = fetch failed
                glib::ControlFlow::Break
            }
        }
    });
}

/// Load an image into a Picture widget
pub fn load_image_into_picture(picture: gtk4::Picture, url: String) {
    let url = ensure_jpeg_format(&url);

    // Check memory cache first — instant hit
    if let Some(cached) = AVATAR_CACHE.read().unwrap().get(&url).cloned() {
        apply_bytes_to_picture(&picture, &cached);
        return;
    }

    // Kick off a fetch if not already in progress (deduplicates)
    spawn_fetch_if_needed(&url);

    // Poll the cache on the GTK main thread until the image arrives
    let poll_url = url.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        if let Some(bytes) = AVATAR_CACHE.read().unwrap().get(&poll_url).cloned() {
            apply_bytes_to_picture(&picture, &bytes);
            glib::ControlFlow::Break
        } else {
            let still_in_flight = IN_FLIGHT.read().unwrap().contains(&poll_url);
            if still_in_flight {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        }
    });
}
