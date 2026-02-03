// SPDX-License-Identifier: MPL-2.0

//! Efficient image loading with parallel downloads and persistent caching.
//!
//! Architecture:
//! - Parallel downloads via Tokio (configurable concurrency)
//! - Three-tier cache: decoded memory LRU -> SQLite disk -> network
//! - Request deduplication (multiple widgets requesting same URL share one fetch)
//! - Off-thread image decoding with automatic downscaling for avatars
//! - Instant display from memory cache (no decoding needed)

use crate::cache::{CacheDb, ImageCache};
use crate::runtime;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::{Cast, WidgetExt};
use image::GenericImageView;
use image::imageops::FilterType;
use libadwaita as adw;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Duration for fade-in animation in milliseconds
const FADE_IN_DURATION_MS: u64 = 150;

/// Maximum concurrent image downloads (increased for modern connections)
const MAX_CONCURRENT_DOWNLOADS: usize = 16;

/// Maximum decoded images to keep in memory
const DECODED_CACHE_CAPACITY: usize = 500;

/// Maximum avatar dimension to decode (saves memory, avatars are typically small)
const MAX_AVATAR_SIZE: u32 = 128;

/// Shared HTTP client configured for image fetching with HTTP/2
static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .timeout(std::time::Duration::from_secs(15))
        .connect_timeout(std::time::Duration::from_secs(5))
        .tcp_nodelay(true)
        .build()
        .expect("failed to create HTTP client")
});

/// Semaphore to limit concurrent downloads
static DOWNLOAD_SEMAPHORE: Lazy<Arc<Semaphore>> =
    Lazy::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_DOWNLOADS)));

/// Global raw image cache instance (SQLite backed)
static IMAGE_CACHE: Lazy<ImageCache> = Lazy::new(ImageCache::new);

/// Pre-decoded image data ready for instant texture creation
#[derive(Clone)]
struct DecodedImage {
    rgba: Arc<Vec<u8>>,
    width: u32,
    height: u32,
}

/// LRU cache for decoded images - avoids re-decoding on every display
struct DecodedLruCache {
    map: HashMap<String, DecodedImage>,
    order: Vec<String>,
    capacity: usize,
}

impl DecodedLruCache {
    fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::with_capacity(capacity),
            order: Vec::with_capacity(capacity),
            capacity,
        }
    }

    fn get(&mut self, key: &str) -> Option<DecodedImage> {
        if let Some(img) = self.map.get(key).cloned() {
            // Move to end (most recently used)
            self.order.retain(|k| k != key);
            self.order.push(key.to_string());
            Some(img)
        } else {
            None
        }
    }

    fn insert(&mut self, key: String, value: DecodedImage) {
        // Evict oldest if at capacity
        while self.map.len() >= self.capacity && !self.order.is_empty() {
            let oldest = self.order.remove(0);
            self.map.remove(&oldest);
        }

        self.map.insert(key.clone(), value);
        self.order.retain(|k| k != &key);
        self.order.push(key);
    }
}

/// Global decoded image cache - for instant display without decoding
static DECODED_CACHE: Lazy<Mutex<DecodedLruCache>> =
    Lazy::new(|| Mutex::new(DecodedLruCache::new(DECODED_CACHE_CAPACITY)));

/// Track in-flight requests to deduplicate (URL -> list of reply channels)
static PENDING_REQUESTS: Lazy<Mutex<HashMap<String, Vec<std::sync::mpsc::Sender<DecodedImage>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Global reference to the cache database (set during app initialization)
static CACHE_DB: RwLock<Option<Arc<CacheDb>>> = RwLock::new(None);

/// Initialize the image loader with a cache database reference.
/// Must be called after CacheDb is opened.
pub fn init(cache_db: Arc<CacheDb>) {
    let mut db = CACHE_DB.write().unwrap();
    *db = Some(cache_db);
}

/// Load an avatar image from URL, using cache when available.
/// The avatar widget will be updated asynchronously when the image is ready.
pub fn load_avatar(avatar: adw::Avatar, url: String) {
    // FAST PATH: Check decoded memory cache first (instant, no decoding)
    {
        let mut decoded_cache = DECODED_CACHE.lock().unwrap();
        if let Some(decoded) = decoded_cache.get(&url) {
            // Instant display - already decoded!
            apply_decoded_to_avatar(&avatar, &decoded);
            return;
        }
    }

    // Create a channel for receiving the decoded image
    let (tx, rx) = std::sync::mpsc::channel();

    // Check if request is already in flight
    {
        let mut pending = PENDING_REQUESTS.lock().unwrap();
        if let Some(senders) = pending.get_mut(&url) {
            senders.push(tx);
            setup_avatar_polling(avatar, rx);
            return;
        }
        pending.insert(url.clone(), vec![tx]);
    }

    // Set up polling for this widget
    setup_avatar_polling(avatar, rx);

    // Start async fetch (will check disk cache in background)
    let url_clone = url.clone();
    runtime::spawn(async move {
        fetch_and_decode_image(url_clone).await;
    });
}

/// Load an image into a Picture widget from URL, using cache when available.
/// For post images, we keep full resolution; for thumbnails use load_thumbnail_into_picture.
pub fn load_image_into_picture(picture: gtk4::Picture, url: String) {
    load_image_into_picture_with_size(picture, url, None);
}

/// Load a thumbnail-sized image into a Picture widget
#[allow(dead_code)]
pub fn load_thumbnail_into_picture(picture: gtk4::Picture, url: String) {
    load_image_into_picture_with_size(picture, url, Some(256));
}

/// Internal: Load image with optional size limit
fn load_image_into_picture_with_size(picture: gtk4::Picture, url: String, max_size: Option<u32>) {
    // FAST PATH: Check decoded memory cache first (instant, no decoding)
    {
        let mut decoded_cache = DECODED_CACHE.lock().unwrap();
        if let Some(decoded) = decoded_cache.get(&url) {
            apply_decoded_to_picture(&picture, &decoded);
            return;
        }
    }

    // Create a channel for receiving the decoded image
    let (tx, rx) = std::sync::mpsc::channel();

    // Check if request is already in flight
    {
        let mut pending = PENDING_REQUESTS.lock().unwrap();
        if let Some(senders) = pending.get_mut(&url) {
            senders.push(tx);
            setup_picture_polling(picture, rx);
            return;
        }
        pending.insert(url.clone(), vec![tx]);
    }

    setup_picture_polling(picture, rx);

    // Start async fetch (will check disk cache in background)
    let url_clone = url.clone();
    runtime::spawn(async move {
        fetch_and_decode_image_with_size(url_clone, max_size).await;
    });
}

/// Set up GTK polling to receive decoded image for an Avatar
fn setup_avatar_polling(avatar: adw::Avatar, rx: std::sync::mpsc::Receiver<DecodedImage>) {
    glib::timeout_add_local(std::time::Duration::from_millis(8), move || {
        match rx.try_recv() {
            Ok(decoded) => {
                apply_decoded_to_avatar(&avatar, &decoded);
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

/// Set up GTK polling to receive decoded image for a Picture
fn setup_picture_polling(picture: gtk4::Picture, rx: std::sync::mpsc::Receiver<DecodedImage>) {
    glib::timeout_add_local(std::time::Duration::from_millis(8), move || {
        match rx.try_recv() {
            Ok(decoded) => {
                apply_decoded_to_picture(&picture, &decoded);
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

/// Fetch an image, decode it off-thread, and notify all waiting callbacks
async fn fetch_and_decode_image(url: String) {
    fetch_and_decode_image_with_size(url, Some(MAX_AVATAR_SIZE)).await;
}

/// Fetch a full-size image without downscaling
#[allow(dead_code)]
async fn fetch_and_decode_image_full(url: String) {
    fetch_and_decode_image_with_size(url, None).await;
}

/// Fetch an image with optional size limit
async fn fetch_and_decode_image_with_size(url: String, max_size: Option<u32>) {
    // Acquire semaphore permit to limit concurrency
    let _permit: OwnedSemaphorePermit = match DOWNLOAD_SEMAPHORE.clone().acquire_owned().await {
        Ok(p) => p,
        Err(_) => {
            cleanup_pending(&url);
            return;
        }
    };

    // Check disk cache first (in blocking task to not block async runtime)
    let url_for_disk = url.clone();
    let cached_bytes = tokio::task::spawn_blocking(move || {
        if let Some(db) = CACHE_DB.read().unwrap().clone() {
            IMAGE_CACHE.get(&db, &url_for_disk).map(|c| c.data)
        } else {
            None
        }
    })
    .await
    .ok()
    .flatten();

    // If not in disk cache, fetch from network
    let bytes = match cached_bytes {
        Some(b) => b,
        None => {
            let result = HTTP_CLIENT.get(&url).send().await;
            let fetched = match result {
                Ok(response) => match response.bytes().await {
                    Ok(b) => b.to_vec(),
                    Err(_) => {
                        cleanup_pending(&url);
                        return;
                    }
                },
                Err(_) => {
                    cleanup_pending(&url);
                    return;
                }
            };

            // Store in disk cache (non-blocking)
            let url_for_store = url.clone();
            let data_for_store = fetched.clone();
            tokio::task::spawn_blocking(move || {
                if let Some(db) = CACHE_DB.read().unwrap().clone() {
                    let _ = IMAGE_CACHE.store(
                        &db,
                        &url_for_store,
                        data_for_store,
                        Some("image/jpeg".to_string()),
                    );
                }
            });

            fetched
        }
    };

    // Decode image off the async runtime thread
    let url_for_cache = url.clone();
    let decoded = tokio::task::spawn_blocking(move || {
        decode_image_with_size(&bytes, max_size).map(|d| {
            let decoded = DecodedImage {
                rgba: Arc::new(d.rgba),
                width: d.width,
                height: d.height,
            };

            // Store in decoded memory cache for instant future access
            {
                let mut cache = DECODED_CACHE.lock().unwrap();
                cache.insert(url_for_cache, decoded.clone());
            }

            decoded
        })
    })
    .await;

    let decoded = match decoded {
        Ok(Some(d)) => d,
        _ => {
            cleanup_pending(&url);
            return;
        }
    };

    // Get and clear pending senders
    let senders = {
        let mut pending = PENDING_REQUESTS.lock().unwrap();
        pending.remove(&url).unwrap_or_default()
    };

    // Send decoded image to all waiting receivers
    for sender in senders {
        let _ = sender.send(decoded.clone());
    }
}

/// Raw decoded data from image crate
struct RawDecoded {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

/// Decode image bytes to RGBA using the image crate (runs off main thread)
/// Automatically downscales large images to save memory and improve performance
fn decode_image_with_size(bytes: &[u8], max_size: Option<u32>) -> Option<RawDecoded> {
    let img = image::load_from_memory(bytes).ok()?;
    let (width, height) = img.dimensions();

    // Downscale large images if max_size is specified
    let img = if let Some(max) = max_size {
        if width > max || height > max {
            let scale = max as f32 / width.max(height) as f32;
            let new_width = (width as f32 * scale) as u32;
            let new_height = (height as f32 * scale) as u32;
            // Use Triangle filter - fast and good enough for downscaling
            image::DynamicImage::ImageRgba8(image::imageops::resize(
                &img.to_rgba8(),
                new_width,
                new_height,
                FilterType::Triangle,
            ))
        } else {
            img
        }
    } else {
        img
    };

    let (width, height) = img.dimensions();
    let rgba = img.into_rgba8().into_raw();

    Some(RawDecoded {
        rgba,
        width,
        height,
    })
}

/// Remove pending senders for a failed request
fn cleanup_pending(url: &str) {
    let mut pending = PENDING_REQUESTS.lock().unwrap();
    pending.remove(url);
}

/// Apply pre-decoded image to an Avatar widget with smooth fade-in animation
fn apply_decoded_to_avatar(avatar: &adw::Avatar, decoded: &DecodedImage) {
    if let Some(texture) = create_texture_from_rgba(&decoded.rgba, decoded.width, decoded.height) {
        // Start invisible
        avatar.set_opacity(0.0);
        avatar.set_custom_image(Some(&texture));
        // Animate fade-in
        fade_in_widget(avatar.upcast_ref());
    }
}

/// Apply pre-decoded image to a Picture widget with smooth fade-in animation
fn apply_decoded_to_picture(picture: &gtk4::Picture, decoded: &DecodedImage) {
    if let Some(texture) = create_texture_from_rgba(&decoded.rgba, decoded.width, decoded.height) {
        // Start invisible
        picture.set_opacity(0.0);
        picture.set_paintable(Some(&texture));
        // Animate fade-in
        fade_in_widget(picture.upcast_ref());
    }
}

/// Smoothly fade in a widget over FADE_IN_DURATION_MS
fn fade_in_widget(widget: &gtk4::Widget) {
    let widget = widget.clone();
    let start_time = std::time::Instant::now();
    let duration = std::time::Duration::from_millis(FADE_IN_DURATION_MS);

    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        let elapsed = start_time.elapsed();
        if elapsed >= duration {
            widget.set_opacity(1.0);
            glib::ControlFlow::Break
        } else {
            // Ease-out cubic for smooth deceleration
            let progress = elapsed.as_secs_f64() / duration.as_secs_f64();
            let eased = 1.0 - (1.0 - progress).powi(3);
            widget.set_opacity(eased);
            glib::ControlFlow::Continue
        }
    });
}

/// Create a GDK texture from raw RGBA pixel data
fn create_texture_from_rgba(rgba: &[u8], width: u32, height: u32) -> Option<gdk::Texture> {
    let bytes = glib::Bytes::from(rgba);
    let stride = width as i32 * 4;

    let texture = gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        stride as usize,
    );

    Some(texture.upcast())
}

/// Perform cache cleanup (call periodically or on app startup)
pub fn cleanup_cache() {
    if let Some(db) = CACHE_DB.read().unwrap().clone()
        && let Ok(stats) = IMAGE_CACHE.cleanup(&db)
        && (stats.old_deleted > 0 || stats.size_deleted > 0)
    {
        eprintln!(
            "Image cache cleanup: {} old, {} for size, {}MB remaining",
            stats.old_deleted,
            stats.size_deleted,
            stats.total_size_after / 1024 / 1024
        );
    }
}

/// Get cache statistics for debugging
#[allow(dead_code)]
pub fn cache_stats() -> Option<crate::cache::CacheStats> {
    let db = CACHE_DB.read().unwrap().clone()?;
    IMAGE_CACHE.stats(&db).ok()
}
