// SPDX-License-Identifier: MPL-2.0

use gtk4::gdk;
use gtk4::glib;
use libadwaita as adw;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, RwLock};

static AVATAR_CACHE: Lazy<Arc<RwLock<HashMap<String, Vec<u8>>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

/// Single worker thread + runtime for all image/avatar fetches to avoid exhausting file descriptors.
static REQUEST_TX: Lazy<mpsc::Sender<(String, mpsc::Sender<Vec<u8>>)>> = Lazy::new(|| {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || worker(rx));
    tx
});

fn worker(rx: mpsc::Receiver<(String, mpsc::Sender<Vec<u8>>)>) {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(_) => return,
    };
    let cache = Arc::clone(&AVATAR_CACHE);
    while let Ok((url, reply_tx)) = rx.recv() {
        let cache = Arc::clone(&cache);
        let url_clone = url.clone();
        let result: Option<Vec<u8>> = rt.block_on(async {
            let response = reqwest::get(&url).await.ok()?;
            let bytes = response.bytes().await.ok()?;
            Some(bytes.to_vec())
        });
        if let Some(bytes) = result {
            let _ = cache.write().unwrap().insert(url_clone, bytes.clone());
            let _ = reply_tx.send(bytes);
        }
    }
}

fn apply_avatar_bytes(avatar: &adw::Avatar, bytes: &[u8]) {
    let glib_bytes = glib::Bytes::from(bytes);
    let stream = gtk4::gio::MemoryInputStream::from_bytes(&glib_bytes);

    if let Ok(pixbuf) =
        gdk::gdk_pixbuf::Pixbuf::from_stream(&stream, gtk4::gio::Cancellable::NONE)
    {
        let texture = gdk::Texture::for_pixbuf(&pixbuf);
        avatar.set_custom_image(Some(&texture));
    }
}

pub fn load_avatar(avatar: adw::Avatar, url: String) {
    if let Some(cached) = AVATAR_CACHE.read().unwrap().get(&url).cloned() {
        apply_avatar_bytes(&avatar, &cached);
        return;
    }

    let (reply_tx, reply_rx) = mpsc::channel();
    if REQUEST_TX.send((url, reply_tx)).is_err() {
        return;
    }

    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        match reply_rx.try_recv() {
            Ok(bytes) => {
                apply_avatar_bytes(&avatar, &bytes);
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

fn apply_bytes_to_picture(picture: &gtk4::Picture, bytes: &[u8]) {
    let glib_bytes = glib::Bytes::from(bytes);
    let stream = gtk4::gio::MemoryInputStream::from_bytes(&glib_bytes);

    if let Ok(pixbuf) =
        gdk::gdk_pixbuf::Pixbuf::from_stream(&stream, gtk4::gio::Cancellable::NONE)
    {
        let texture = gdk::Texture::for_pixbuf(&pixbuf);
        picture.set_paintable(Some(&texture));
    }
}

pub fn load_image_into_picture(picture: gtk4::Picture, url: String) {
    if let Some(cached) = AVATAR_CACHE.read().unwrap().get(&url).cloned() {
        apply_bytes_to_picture(&picture, &cached);
        return;
    }

    let (reply_tx, reply_rx) = mpsc::channel();
    if REQUEST_TX.send((url, reply_tx)).is_err() {
        return;
    }

    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        match reply_rx.try_recv() {
            Ok(bytes) => {
                apply_bytes_to_picture(&picture, &bytes);
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}
