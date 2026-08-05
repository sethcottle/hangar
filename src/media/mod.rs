// SPDX-License-Identifier: MPL-2.0

//! GStreamer plumbing for video playback.
//!
//! `GtkMediaFile` cannot play Bluesky's video: it marks the stream prepared and
//! then fails inside `GstHLSDemux2`, giving a black frame and no usable error.
//! The same playlist plays under `playbin3` with stock plugins, so Hangar drives
//! its own pipeline and renders through `gtk4paintablesink`'s `GdkPaintable`.
//!
//! The sink is registered statically rather than looked up in the system
//! registry, so nobody has to install `gstreamer1.0-gtk4`.

use gst::prelude::*;
use gstreamer as gst;
use gtk4::gdk;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use url::Url;

/// How many pipelines this process owns.
///
/// A rebound `GtkListView` row leaks a decoder, an audio sink and a bus watch;
/// the symptom is the app getting slow a hundred rows later. Assertable in
/// `ui::inline_video::tests`, and watchable with `HANGAR_DEBUG_PIPELINES=1`.
static LIVE_PIPELINES: AtomicUsize = AtomicUsize::new(0);

/// How many pipelines are alive right now.
pub fn live_pipelines() -> usize {
    LIVE_PIPELINES.load(Ordering::Relaxed)
}

/// RAII token proving one pipeline exists.
///
/// `gst::Element` is a refcounted GObject cloned all over a player, so the count
/// cannot hang off the element itself.
pub struct PipelineToken {
    _private: (),
}

impl PipelineToken {
    fn new(uri: &str) -> Self {
        let live = LIVE_PIPELINES.fetch_add(1, Ordering::Relaxed) + 1;
        trace_pipelines("built", live, uri);
        Self { _private: () }
    }
}

impl Drop for PipelineToken {
    fn drop(&mut self) {
        let live = LIVE_PIPELINES.fetch_sub(1, Ordering::Relaxed) - 1;
        trace_pipelines("dropped", live, "");
    }
}

fn trace_pipelines(what: &str, live: usize, uri: &str) {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if *ENABLED.get_or_init(|| std::env::var_os("HANGAR_DEBUG_PIPELINES").is_some()) {
        eprintln!("hangar: pipeline {what}, {live} live {uri}");
    }
}

/// Everything that can keep a video from playing, in a shape the UI can show.
#[derive(Debug, Clone)]
pub enum MediaError {
    /// GStreamer itself refused to start.
    Init(String),
    /// An element the pipeline needs is not installed on this system.
    MissingElement(&'static str),
    /// The uri is not http or https, so it was never handed to GStreamer.
    UnsupportedScheme(String),
    /// The pipeline posted an error on its bus.
    Pipeline {
        message: String,
        debug: Option<String>,
    },
}

impl MediaError {
    /// One sentence, safe to put in front of a person.
    pub fn user_message(&self) -> String {
        match self {
            Self::Init(_) => "GStreamer could not be started on this system.".to_string(),
            Self::MissingElement(_) => {
                "Your system is missing the GStreamer plugins needed to play this video."
                    .to_string()
            }
            Self::UnsupportedScheme(_) => {
                "This video is not at a web address, so Hangar did not play it.".to_string()
            }
            Self::Pipeline { .. } => "This video could not be played.".to_string(),
        }
    }

    /// The unabridged GStreamer text, for the details expander and stderr.
    ///
    /// A missing element is a packaging problem, so name the packages.
    pub fn detail(&self) -> String {
        match self {
            Self::Init(e) => e.clone(),
            Self::MissingElement(name) => format!(
                "The GStreamer element \"{name}\" is not installed.\n\n\
                 Playing Bluesky video needs the base, good and bad plugin sets \
                 plus an H.264 decoder:\n  \
                 Debian/Ubuntu: gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
                 gstreamer1.0-plugins-bad gstreamer1.0-libav\n  \
                 Fedora: gstreamer1-plugins-base gstreamer1-plugins-good \
                 gstreamer1-plugins-bad-free gstreamer1-libav\n  \
                 Arch: gst-plugins-base gst-plugins-good gst-plugins-bad gst-libav"
            ),
            Self::UnsupportedScheme(uri) => format!(
                "Refused to play \"{uri}\".\n\n\
                 Only http and https are allowed. playbin3 picks its source \
                 element from the scheme, so any other one reaches a GStreamer \
                 uri handler with an address chosen by whoever posted the video."
            ),
            Self::Pipeline { message, debug } => match debug {
                Some(d) => format!("{message}\n\n{d}"),
                None => message.clone(),
            },
        }
    }
}

/// A built pipeline and the paintable its video lands in.
pub struct VideoPipeline {
    pub playbin: gst::Element,
    pub paintable: gdk::Paintable,
    /// Held as long as the pipeline is; dropping it decrements
    /// [`live_pipelines`]. Cloning the playbin does not stand in for it.
    pub token: PipelineToken,
}

/// Initialize GStreamer and register `gtk4paintablesink`, once per process.
///
/// Called on first playback rather than from `main`: `gst::init` scans the
/// plugin registry, and most sessions never open a video.
///
/// Must run on the GTK main thread, because the caller goes on to read the
/// sink's `paintable` property, which is main-thread only.
pub fn ensure_initialized() -> Result<(), MediaError> {
    static INIT: OnceLock<Result<(), MediaError>> = OnceLock::new();

    INIT.get_or_init(|| {
        gst::init().map_err(|e| MediaError::Init(e.to_string()))?;
        gstgtk4::plugin_register_static().map_err(|e| MediaError::Init(e.to_string()))?;
        Ok(())
    })
    .clone()
}

/// Whether a uri is one this process will let GStreamer resolve.
fn is_web_uri(uri: &str) -> bool {
    Url::parse(uri).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

fn make(name: &'static str) -> Result<gst::Element, MediaError> {
    gst::ElementFactory::make(name)
        .build()
        .map_err(|_| MediaError::MissingElement(name))
}

/// Build a `playbin3` for `uri` rendering into a fresh `gtk4paintablesink`.
///
/// `volume` is linear (see [`linear_volume`]) and `muted` is applied before the
/// pipeline reaches PLAYING, so a muted start cannot leak a burst of audio.
///
/// Must be called on the GTK main thread.
pub fn build_pipeline(uri: &str, volume: f64, muted: bool) -> Result<VideoPipeline, MediaError> {
    // The playlist url is copied verbatim off the wire, and playbin3 chooses
    // its source element by scheme: rtsp, udp and mms are blind requests to
    // whatever host it names, gvfs schemes mount, and a local path just opens.
    // Same rule crate::atproto::gif applies to the uris it builds.
    if !is_web_uri(uri) {
        return Err(MediaError::UnsupportedScheme(uri.to_string()));
    }

    ensure_initialized()?;

    // No reconfigure-on-window-resize: "overlay-only" is already the default,
    // and it only matters for the sink's own RenderWidget. Hangar draws the
    // paintable in a plain GtkPicture.
    let sink = gst::ElementFactory::make("gtk4paintablesink")
        .build()
        .map_err(|_| MediaError::MissingElement("gtk4paintablesink"))?;

    let paintable = sink.property::<gdk::Paintable>("paintable");

    // The sink only accepts GL buffers directly when it managed to create a GL
    // context; otherwise its caps are system memory and playbin3 needs a
    // converter in front of it.
    let video_sink: gst::Element = if paintable
        .property::<Option<gdk::GLContext>>("gl-context")
        .is_some()
    {
        gst::ElementFactory::make("glsinkbin")
            .property("sink", &sink)
            .build()
            .map_err(|_| MediaError::MissingElement("glsinkbin"))?
    } else {
        let bin = gst::Bin::default();
        let convert = make("videoconvert")?;
        bin.add_many([&convert, &sink])
            .map_err(|e| MediaError::Init(e.to_string()))?;
        convert
            .link(&sink)
            .map_err(|e| MediaError::Init(e.to_string()))?;
        let target = convert
            .static_pad("sink")
            .expect("videoconvert always has a sink pad");
        let ghost =
            gst::GhostPad::with_target(&target).map_err(|e| MediaError::Init(e.to_string()))?;
        bin.add_pad(&ghost)
            .map_err(|e| MediaError::Init(e.to_string()))?;
        bin.upcast()
    };

    let playbin = gst::ElementFactory::make("playbin3")
        .property("uri", uri)
        .property("video-sink", &video_sink)
        .property("volume", volume)
        .property("mute", muted)
        .build()
        .map_err(|_| MediaError::MissingElement("playbin3"))?;

    Ok(VideoPipeline {
        playbin,
        paintable,
        // Last, so nothing counted here can be undone by a `?` above it.
        token: PipelineToken::new(uri),
    })
}

/// Convert a perceptual 0.0..=1.0 slider position to playbin3's linear scale.
///
/// `playbin3` implements `GstStreamVolume`, whose `volume` property is linear;
/// a volume slider needs a cubic scale. GStreamer's own cubic-to-linear
/// conversion is x^3, so this needs no audio bindings.
pub fn linear_volume(slider: f64) -> f64 {
    let s = slider.clamp(0.0, 1.0);
    s * s * s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_playlist_url_off_the_wire_can_only_be_http() {
        for hostile in [
            "rtsp://attacker.example/stream",
            "rtmp://attacker.example/live",
            "udp://239.0.0.1:1234",
            "tcp://192.168.1.1:80",
            "mms://attacker.example/x",
            "smb://attacker.example/share",
            "file:///home/you/.ssh/id_rsa",
            "/home/you/.ssh/id_rsa",
            "",
        ] {
            // Refused before ensure_initialized, so GStreamer never sees it.
            let refusal = build_pipeline(hostile, 1.0, true)
                .err()
                .unwrap_or_else(|| panic!("{hostile} should not have built a pipeline"));
            match refusal {
                MediaError::UnsupportedScheme(uri) => assert_eq!(uri, hostile),
                other => panic!("{hostile} should be refused for its scheme, got {other:?}"),
            }
        }
    }

    #[test]
    fn the_video_hosts_bluesky_uses_are_allowed() {
        for ok in [
            "https://video.bsky.app/watch/did/cid/playlist.m3u8",
            "https://k.gifs.bsky.app/ii/hash/76/03/slug.mp4",
            "http://example.com/v.m3u8",
        ] {
            assert!(is_web_uri(ok), "{ok} should be playable");
        }
    }
}
