// SPDX-License-Identifier: MPL-2.0

//! GStreamer plumbing for video playback.
//!
//! GTK's own `GtkMediaFile` cannot play Bluesky's video: it marks the stream
//! prepared and only then fails inside `GstHLSDemux2`, which is what produced a
//! black frame with no usable error. The very same playlist plays under
//! `playbin3` with the host's stock plugins, so Hangar drives its own pipeline
//! and renders through a `GdkPaintable` handed out by `gtk4paintablesink`.
//!
//! The sink is registered statically from inside the process rather than being
//! looked up in the system registry, so nobody has to install
//! `gstreamer1.0-gtk4` to get a picture.

use gst::prelude::*;
use gstreamer as gst;
use gtk4::gdk;
use std::sync::OnceLock;

/// Everything that can keep a video from playing, in a shape the UI can show.
#[derive(Debug, Clone)]
pub enum MediaError {
    /// GStreamer itself refused to start.
    Init(String),
    /// An element the pipeline needs is not installed on this system.
    MissingElement(&'static str),
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
            Self::Pipeline { .. } => "This video could not be played.".to_string(),
        }
    }

    /// The unabridged GStreamer text, for the details expander and stderr.
    ///
    /// A missing element is nearly always a packaging problem rather than a bug
    /// in the stream, so name the packages that carry the HLS chain instead of
    /// making people search for them.
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
}

/// Initialize GStreamer and register `gtk4paintablesink`, once per process.
///
/// Deliberately called on first playback rather than from `main`: `gst::init`
/// scans the plugin registry, and most sessions never open a video at all.
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

fn make(name: &'static str) -> Result<gst::Element, MediaError> {
    gst::ElementFactory::make(name)
        .build()
        .map_err(|_| MediaError::MissingElement(name))
}

/// Build a `playbin3` for `uri` rendering into a fresh `gtk4paintablesink`.
///
/// `volume` is linear (see [`linear_volume`]) and `muted` is applied before the
/// pipeline ever reaches PLAYING, so a muted start never leaks a burst of audio.
///
/// Must be called on the GTK main thread.
pub fn build_pipeline(uri: &str, volume: f64, muted: bool) -> Result<VideoPipeline, MediaError> {
    ensure_initialized()?;

    // No `reconfigure-on-window-resize` here: "overlay-only" is already the
    // property's default, and it only does anything once something feeds the
    // sink its window-width/window-height, which only the sink's own
    // RenderWidget does. Hangar draws the paintable in a plain GtkPicture, so
    // setting it would buy nothing.
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

    Ok(VideoPipeline { playbin, paintable })
}

/// Convert a perceptual 0.0..=1.0 slider position to playbin3's linear scale.
///
/// `playbin3` implements `GstStreamVolume`, whose `volume` property is linear,
/// while a slider people can reason about is cubic. GStreamer's own cubic to
/// linear conversion is exactly x^3, so no audio bindings are needed for it.
pub fn linear_volume(slider: f64) -> f64 {
    let s = slider.clamp(0.0, 1.0);
    s * s * s
}
