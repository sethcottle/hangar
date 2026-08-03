// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::type_complexity)]

//! Concurrent image loading with a dedicated runtime.
//! Uses parallel fetching for fast avatar loading with deduplication.

use crate::cache::CacheDb;
use gtk4::gdk;
use gtk4::glib;
use libadwaita as adw;
use once_cell::sync::Lazy;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Maximum concurrent downloads
const MAX_CONCURRENT: usize = 8;

/// How often a waiting widget looks to see whether its bytes have landed.
///
/// The fetch runs on another runtime and cannot reach into GTK, so the main
/// thread polls. One frame at 60Hz.
const POLL_INTERVAL: Duration = Duration::from_millis(16);

/// In-memory cache for loaded images (URL → bytes)
static AVATAR_CACHE: Lazy<Arc<RwLock<HashMap<String, Vec<u8>>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

/// URLs currently being fetched (deduplication set)
static IN_FLIGHT: Lazy<Arc<RwLock<HashSet<String>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashSet::new())));

/// Dedicated runtime so image fetching stays off the main app runtime
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

/// Decode `bytes` onto `picture`, reporting whether anything was drawn.
///
/// Bytes in a format gdk-pixbuf has no loader for count as a failed load.
fn apply_bytes_to_picture(picture: &gtk4::Picture, bytes: &[u8]) -> bool {
    let glib_bytes = glib::Bytes::from(bytes);
    let stream = gtk4::gio::MemoryInputStream::from_bytes(&glib_bytes);

    match gdk::gdk_pixbuf::Pixbuf::from_stream(&stream, gtk4::gio::Cancellable::NONE) {
        Ok(pixbuf) => {
            let texture = gdk::Texture::for_pixbuf(&pixbuf);
            picture.set_paintable(Some(&texture));
            true
        }
        Err(_) => false,
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

/// The bytes fetched for `url`, if its fetch has already finished.
///
/// Pass the same URL given to `load_image_into_picture`: the JPEG normalisation
/// that decides the cache key happens in here.
pub fn cached_bytes(url: &str) -> Option<Vec<u8>> {
    AVATAR_CACHE
        .read()
        .unwrap()
        .get(&ensure_jpeg_format(url))
        .cloned()
}

/// Load an avatar image from URL
pub fn load_avatar(avatar: adw::Avatar, url: String) {
    let url = ensure_jpeg_format(&url);

    // Check the memory cache first; hits return immediately
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

/// How a load into a `Picture` ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageLoadResult {
    /// The bytes arrived, decoded, and are on the widget now.
    Shown,
    /// The fetch failed, or what came back was not an image this system can
    /// decode. The widget was not touched.
    Failed,
}

/// Shared between the handle and the poll it owns.
#[derive(Default)]
struct LoadState {
    cancelled: Cell<bool>,
    /// `Some` only while the poll is installed. The poll clears it before it
    /// finishes so [`ImageLoad::drop`] never removes a source that is already
    /// on its way out.
    source: RefCell<Option<glib::SourceId>>,
}

/// A load in flight.
///
/// Dropping it removes the poll and the callback never runs. Without it, paging
/// a gallery leaves one poll per image, each holding the same `Picture` and
/// calling `set_paintable` when its bytes land.
#[must_use = "the load is cancelled the moment this handle is dropped"]
pub struct ImageLoad {
    state: Rc<LoadState>,
    cancel_on_drop: bool,
}

impl ImageLoad {
    /// Let the load finish on its own.
    ///
    /// For a widget that shows one image and outlives the load, like a feed
    /// thumbnail.
    pub fn detach(mut self) {
        self.cancel_on_drop = false;
    }
}

impl Drop for ImageLoad {
    fn drop(&mut self) {
        if !self.cancel_on_drop {
            return;
        }
        self.state.cancelled.set(true);
        if let Some(id) = self.state.source.borrow_mut().take() {
            id.remove();
        }
    }
}

/// Load an image into a Picture widget.
///
/// Fire and forget: use [`load_image_into_picture_tracked`] where the same
/// widget can be asked for a different image before this one arrives.
pub fn load_image_into_picture(picture: gtk4::Picture, url: String) {
    start_picture_load(picture, url, None).detach();
}

/// Load an image into a Picture widget, keeping hold of the load.
///
/// `on_done` is called exactly once unless the returned handle is dropped
/// first. It runs inline when the bytes are already in the memory cache, so a
/// caller must have the widget in the state it wants before it calls this.
pub fn load_image_into_picture_tracked(
    picture: gtk4::Picture,
    url: String,
    on_done: impl Fn(ImageLoadResult) + 'static,
) -> ImageLoad {
    start_picture_load(picture, url, Some(Box::new(on_done)))
}

type DoneCallback = Box<dyn Fn(ImageLoadResult)>;

fn start_picture_load(
    picture: gtk4::Picture,
    url: String,
    on_done: Option<DoneCallback>,
) -> ImageLoad {
    let url = ensure_jpeg_format(&url);
    let state = Rc::new(LoadState::default());
    let handle = ImageLoad {
        state: Rc::clone(&state),
        cancel_on_drop: true,
    };

    // Check the memory cache first; hits return immediately
    if let Some(cached) = AVATAR_CACHE.read().unwrap().get(&url).cloned() {
        let result = decode_result(&picture, &cached);
        if let Some(done) = &on_done {
            done(result);
        }
        return handle;
    }

    // Kick off a fetch if not already in progress (deduplicates)
    spawn_fetch_if_needed(&url);

    // Poll the cache on the GTK main thread until the image arrives
    let poll_state = Rc::clone(&state);
    let id = glib::timeout_add_local(POLL_INTERVAL, move || {
        if poll_state.cancelled.get() {
            return glib::ControlFlow::Break;
        }

        let cached = AVATAR_CACHE.read().unwrap().get(&url).cloned();
        let result = match cached {
            Some(bytes) => decode_result(&picture, &bytes),
            // The fetch caches before clearing the in-flight mark, so "not
            // cached and not in flight" is a fetch that finished with nothing.
            None if IN_FLIGHT.read().unwrap().contains(&url) => {
                return glib::ControlFlow::Continue;
            }
            None => ImageLoadResult::Failed,
        };

        // Retire the source before the callback runs: `on_done` is allowed to
        // drop the handle, and `Drop` must not try to remove a source that is
        // finishing anyway.
        poll_state.source.borrow_mut().take();
        if let Some(done) = &on_done {
            done(result);
        }
        glib::ControlFlow::Break
    });
    state.source.replace(Some(id));

    handle
}

fn decode_result(picture: &gtk4::Picture, bytes: &[u8]) -> ImageLoadResult {
    if apply_bytes_to_picture(picture, bytes) {
        ImageLoadResult::Shown
    } else {
        ImageLoadResult::Failed
    }
}
