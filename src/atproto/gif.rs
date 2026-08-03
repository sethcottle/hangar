// SPDX-License-Identifier: MPL-2.0

//! GIFs, which Bluesky sends as link cards.
//!
//! There is no GIF embed in the lexicon. A GIF from the composer's picker
//! arrives as an `app.bsky.embed.external` view whose `uri` points at a GIF
//! host, with `title`, `description` and a still `thumb`.
//!
//! Two hosts: Klipy (`static.klipy.com`) and Tenor, still in older posts. All
//! 2,390 GIFs sampled from `whats-hot` and `hot-classic` were Klipy.
//!
//! The `uri` is the multi-megabyte `.gif` itself. Both hosts put enough in the
//! URL to name Bluesky's MP4/WebM re-encode on `*.gifs.bsky.app`, which
//! [`crate::media::build_pipeline`] can decode and is about six times smaller.
//! Rewrite rules transcribed from Bluesky's `parseKlipyGif` / `parseTenorGif`.
//!
//! Giphy needs explicit user consent, so Giphy links stay link cards.

use url::Url;

/// Host serving Bluesky's re-encodes of Klipy GIFs.
const KLIPY_PLAYER_HOST: &str = "k.gifs.bsky.app";
/// Host serving Bluesky's re-encodes of Tenor GIFs.
const TENOR_PLAYER_HOST: &str = "t.gifs.bsky.app";

/// Query parameters that carry metadata rather than addressing the asset.
const METADATA_PARAMS: [&str; 4] = ["hh", "ww", "mp4", "webm"];

/// An animated GIF recovered from an external link card.
///
/// Derived at render time. Storing it on [`crate::atproto::ExternalEmbed`]
/// would add a non-defaulted field, which fails every cached post's
/// deserialize, and `cache::posts` drops those silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GifEmbed {
    /// An MP4 or WebM of the same animation, which `playbin3` can decode.
    pub video_url: String,
    /// `(width, height)` in pixels, from the link's own query string. The GIF
    /// hosts always state it, so a GIF never has to guess 16:9.
    pub aspect_ratio: (u32, u32),
}

/// Recognise `uri` as a GIF embed. `None` leaves it as a link card.
pub fn detect(uri: &str) -> Option<GifEmbed> {
    let url = Url::parse(uri).ok()?;
    parse_klipy(&url).or_else(|| parse_tenor(&url))
}

/// Klipy: `https://static.klipy.com/ii/<hash>/<a>/<b>/<slug>.gif?hh=&ww=&mp4=&webm=`
///
/// The playable asset is the same path with the host swapped, the metadata
/// parameters dropped, and the final segment replaced by the per-format slug
/// from `mp4=` / `webm=`. No slug means no playable asset; return `None`.
fn parse_klipy(url: &Url) -> Option<GifEmbed> {
    if url.host_str()? != "static.klipy.com" || !url.path().starts_with("/ii/") {
        return None;
    }

    let aspect_ratio = dimensions(url)?;

    // MP4 first: it is the wider-support format and the one measured smallest.
    let (slug, extension) = match (param(url, "mp4"), param(url, "webm")) {
        (Some(mp4), _) => (mp4, "mp4"),
        (None, Some(webm)) => (webm, "webm"),
        (None, None) => return None,
    };
    // Wire data spliced into a URL handed to GStreamer.
    if !is_safe_segment(&slug) {
        return None;
    }

    let mut player = url.clone();
    // Overwrite scheme and host so the player URL never inherits them from the post.
    player.set_scheme("https").ok()?;
    player.set_host(Some(KLIPY_PLAYER_HOST)).ok()?;
    strip_metadata_params(&mut player);
    {
        let mut segments = player.path_segments_mut().ok()?;
        segments.pop();
        segments.push(&format!("{slug}.{extension}"));
    }

    Some(GifEmbed {
        video_url: player.into(),
        aspect_ratio,
    })
}

/// Tenor: `https://media.tenor.com/<id>/<filename>.gif?hh=&ww=`
///
/// Tenor encodes the format in the id rather than in a slug: `AAAAC` is the GIF,
/// `AAAP1` the MP4, `AAAP3` the WebM.
///
/// Unverified: no Tenor GIF appeared in any sampled feed. Transcribed from
/// Bluesky's parser.
fn parse_tenor(url: &Url) -> Option<GifEmbed> {
    if url.host_str()? != "media.tenor.com" {
        return None;
    }

    let mut segments = url.path_segments()?;
    let id = segments.next()?;
    let filename = segments.next()?;
    // `AAAAC` is the marker for the size variant Bluesky knows how to rewrite.
    if !id.contains("AAAAC") || !is_safe_segment(id) || !is_safe_segment(filename) {
        return None;
    }

    let aspect_ratio = dimensions(url)?;

    Some(GifEmbed {
        video_url: format!(
            "https://{TENOR_PLAYER_HOST}/{}/{}",
            id.replace("AAAAC", "AAAP1"),
            filename.replace(".gif", ".mp4")
        ),
        aspect_ratio,
    })
}

/// `(width, height)` from `ww` / `hh`, if both are present and positive.
fn dimensions(url: &Url) -> Option<(u32, u32)> {
    let width: u32 = param(url, "ww")?.parse().ok()?;
    let height: u32 = param(url, "hh")?.parse().ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

fn param(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// Drop the parameters that describe the asset instead of addressing it.
///
/// Rebuilt rather than edited in place so that stripping every parameter leaves
/// no dangling `?`.
fn strip_metadata_params(url: &mut Url) {
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| !METADATA_PARAMS.contains(&key.as_ref()))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    if kept.is_empty() {
        url.set_query(None);
    } else {
        url.query_pairs_mut().clear().extend_pairs(kept);
    }
}

/// Whether a wire-supplied string is safe to splice into a URL path.
///
/// Real slugs and ids are base62 with the occasional separator. Anything else
/// is refused and the post falls back to its link card.
fn is_safe_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim from a live `whats-hot` post, and the rewrites below were
    /// confirmed against the CDN: the `.mp4` answers 206 `video/mp4` and the
    /// `.webm` 206 `video/webm`.
    const KLIPY: &str = "https://static.klipy.com/ii/4493325008d34b7bf8cd6813cd5c1619/76/03/cgMM2dB2oEoY.gif?hh=278&ww=498&mp4=UxPaHvFAwuHpj3&webm=MCEXFCTK5uN0J47r";

    #[test]
    fn klipy_gif_becomes_a_playable_mp4_at_its_stated_size() {
        let gif = detect(KLIPY).expect("a Klipy GIF is a GIF");
        assert_eq!(
            gif.video_url,
            "https://k.gifs.bsky.app/ii/4493325008d34b7bf8cd6813cd5c1619/76/03/UxPaHvFAwuHpj3.mp4"
        );
        // hh/ww, so no 16:9 fallback.
        assert_eq!(gif.aspect_ratio, (498, 278));
        // Every metadata parameter is gone, and nothing is left dangling.
        assert!(!gif.video_url.contains('?'));
    }

    #[test]
    fn klipy_falls_back_to_webm_then_to_the_link_card() {
        let webm_only = KLIPY.replace("&mp4=UxPaHvFAwuHpj3", "");
        assert_eq!(
            detect(&webm_only)
                .expect("webm alone is still playable")
                .video_url,
            "https://k.gifs.bsky.app/ii/4493325008d34b7bf8cd6813cd5c1619/76/03/MCEXFCTK5uN0J47r.webm"
        );

        // Neither slug: there is no video to point at, so this stays a link
        // card rather than becoming a player that 404s.
        let no_slugs = webm_only.replace("&webm=MCEXFCTK5uN0J47r", "");
        assert_eq!(detect(&no_slugs), None);
    }

    #[test]
    fn klipy_needs_real_dimensions_and_the_gif_path() {
        assert_eq!(detect(&KLIPY.replace("hh=278", "hh=0")), None);
        assert_eq!(detect(&KLIPY.replace("hh=278&", "")), None);
        assert_eq!(detect(&KLIPY.replace("ww=498", "ww=wide")), None);
        assert_eq!(detect(&KLIPY.replace("/ii/", "/xx/")), None);
    }

    #[test]
    fn klipy_keeps_unrelated_query_parameters() {
        let extra = KLIPY.replace("?hh=", "?tracking=1&hh=");
        let gif = detect(&extra).expect("still a GIF");
        assert!(
            gif.video_url.ends_with("UxPaHvFAwuHpj3.mp4?tracking=1"),
            "{}",
            gif.video_url
        );
    }

    #[test]
    fn a_slug_off_the_wire_cannot_escape_the_path() {
        // The slug is spliced into a URL handed straight to GStreamer, so it
        // is checked rather than trusted. `%2F` is in the list because
        // `query_pairs` percent-decodes before this ever sees the value.
        for hostile in ["..", "a/../../etc", "a%2Fb", "", "%2E%2E%2Fx", "a?b"] {
            let uri = KLIPY.replace("mp4=UxPaHvFAwuHpj3", &format!("mp4={hostile}"));
            assert_eq!(
                detect(&uri),
                None,
                "a slug of {hostile:?} should leave this as a link card"
            );
        }
    }

    #[test]
    fn the_player_host_is_never_taken_from_the_post() {
        // detect() overwrites scheme and host, so an http link still comes
        // back as https on Bluesky's CDN.
        let gif = detect(&KLIPY.replace("https://", "http://")).expect("still a GIF");
        assert!(
            gif.video_url.starts_with("https://k.gifs.bsky.app/"),
            "{gif:?}"
        );
        // And a host that merely contains the real one is not the real one.
        assert_eq!(
            detect(&KLIPY.replace("static.klipy.com", "static.klipy.com.evil.example")),
            None
        );
        assert_eq!(
            detect("https://media.tenor.com.evil.example/abcAAAACxyz/f.gif?hh=2&ww=2"),
            None
        );
    }

    #[test]
    fn tenor_ids_are_rewritten_to_the_mp4_variant() {
        let gif = detect("https://media.tenor.com/abcAAAACxyz/funny.gif?hh=200&ww=400")
            .expect("a Tenor GIF is a GIF");
        assert_eq!(
            gif.video_url,
            "https://t.gifs.bsky.app/abcAAAP1xyz/funny.mp4"
        );
        assert_eq!(gif.aspect_ratio, (400, 200));

        // Without the AAAAC marker there is no rewrite Bluesky knows about.
        assert_eq!(
            detect("https://media.tenor.com/abcxyz/funny.gif?hh=200&ww=400"),
            None
        );
        // And without dimensions there is nothing to size the embed with.
        assert_eq!(
            detect("https://media.tenor.com/abcAAAACxyz/funny.gif"),
            None
        );
    }

    #[test]
    fn ordinary_link_cards_are_left_alone() {
        for uri in [
            "https://www.theguardian.com/world/2026/aug/01/a-story",
            "https://www.twitch.tv/somebody",
            // Giphy is gated behind consent on bsky, so it stays a link card.
            "https://media.giphy.com/media/abc123/giphy.gif",
            "https://example.com/cat.gif?hh=100&ww=100",
            "not a url at all",
            "",
        ] {
            assert_eq!(detect(uri), None, "{uri} should not be treated as a GIF");
        }
    }
}
