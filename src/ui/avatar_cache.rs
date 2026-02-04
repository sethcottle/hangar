// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::type_complexity)]

//! Concurrent image loading with a dedicated runtime.
//! Uses parallel fetching for fast avatar loading.

use crate::cache::CacheDb;
use gtk4::gdk;
use gtk4::glib;
use libadwaita as adw;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, RwLock};

/// Maximum concurrent downloads
const MAX_CONCURRENT: usize = 8;

/// In-memory cache for loaded images
static AVATAR_CACHE: Lazy<Arc<RwLock<HashMap<String, Vec<u8>>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

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

/// Initialize the avatar cache with a database reference
pub fn init(cache_db: Arc<CacheDb>) {
    let mut db = CACHE_DB.write().unwrap();
    *db = Some(cache_db);
}

/// Perform cache cleanup (no-op for now)
pub fn cleanup_cache() {
    // Currently no-op since we're using simple in-memory cache
}

/// Load an avatar image from URL
pub fn load_avatar(avatar: adw::Avatar, url: String) {
    // Check memory cache first
    if let Some(cached) = AVATAR_CACHE.read().unwrap().get(&url).cloned() {
        apply_avatar_bytes(&avatar, &cached);
        return;
    }

    // Create channel for this request
    let (reply_tx, reply_rx) = mpsc::channel();
    let url_clone = url.clone();
    let cache = Arc::clone(&AVATAR_CACHE);
    let semaphore = Arc::clone(&DOWNLOAD_SEMAPHORE);

    // Spawn async fetch on dedicated runtime
    IMAGE_RUNTIME.spawn(async move {
        // Acquire permit to limit concurrency
        let _permit = match semaphore.acquire().await {
            Ok(p) => p,
            Err(_) => return,
        };

        // Fetch the image
        let result: Option<Vec<u8>> = async {
            let response = reqwest::get(&url_clone).await.ok()?;
            let bytes = response.bytes().await.ok()?;
            Some(bytes.to_vec())
        }
        .await;

        if let Some(bytes) = result {
            // Store in cache
            let _ = cache.write().unwrap().insert(url_clone, bytes.clone());
            // Send to waiting widget
            let _ = reply_tx.send(bytes);
        }
    });

    // Poll for result on GTK main thread
    glib::timeout_add_local(
        std::time::Duration::from_millis(16),
        move || match reply_rx.try_recv() {
            Ok(bytes) => {
                apply_avatar_bytes(&avatar, &bytes);
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        },
    );
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

/// Load an image into a Picture widget
pub fn load_image_into_picture(picture: gtk4::Picture, url: String) {
    // Check memory cache first
    if let Some(cached) = AVATAR_CACHE.read().unwrap().get(&url).cloned() {
        apply_bytes_to_picture(&picture, &cached);
        return;
    }

    // Create channel for this request
    let (reply_tx, reply_rx) = mpsc::channel();
    let url_clone = url.clone();
    let cache = Arc::clone(&AVATAR_CACHE);
    let semaphore = Arc::clone(&DOWNLOAD_SEMAPHORE);

    // Spawn async fetch on dedicated runtime
    IMAGE_RUNTIME.spawn(async move {
        // Acquire permit to limit concurrency
        let _permit = match semaphore.acquire().await {
            Ok(p) => p,
            Err(_) => return,
        };

        // Fetch the image
        let result: Option<Vec<u8>> = async {
            let response = reqwest::get(&url_clone).await.ok()?;
            let bytes = response.bytes().await.ok()?;
            Some(bytes.to_vec())
        }
        .await;

        if let Some(bytes) = result {
            // Store in cache
            let _ = cache.write().unwrap().insert(url_clone, bytes.clone());
            // Send to waiting widget
            let _ = reply_tx.send(bytes);
        }
    });

    // Poll for result on GTK main thread
    glib::timeout_add_local(
        std::time::Duration::from_millis(16),
        move || match reply_rx.try_recv() {
            Ok(bytes) => {
                apply_bytes_to_picture(&picture, &bytes);
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        },
    );
}
