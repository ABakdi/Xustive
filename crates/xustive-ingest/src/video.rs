//! Video extraction (M9-T01).
//!
//! The parser has never produced a video: `<iframe>` sits on the invisible-element list, so every
//! embedded YouTube or Dailymotion player was discarded at parse time, and `og:video` was never
//! read. This module reads them — and reads **only metadata**.
//!
//! Two rules, both pinned by tests:
//!
//! - The stored `url` is the **watch page**, never a stream. A stream URL would invite something
//!   downstream to fetch it, and [[Milestone 2 - Ingestion at Scale|M2-T10.8]] requires that no
//!   code path downloads video bytes.
//! - A poster is stored only when it is **derivable without a fetch** — YouTube and Dailymotion
//!   publish thumbnails at URLs computable from the id. Everything else has no poster rather than
//!   a poster we paid a request for.

use xustive_core::model::{Media, MediaKind};

/// Where a click will take the reader. Named on the tile, because leaving our site is the
/// reader's choice and they should know where to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    YouTube,
    Dailymotion,
    Vimeo,
    /// A `<video>` element hosted by the page itself.
    SelfHosted,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::YouTube => "youtube",
            Provider::Dailymotion => "dailymotion",
            Provider::Vimeo => "vimeo",
            Provider::SelfHosted => "self",
        }
    }
}

/// A recognised video reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Video {
    pub provider: Provider,
    /// The page a person watches it on. Never a stream.
    pub watch_url: String,
    pub poster: Option<String>,
}

impl Video {
    pub fn into_media(self) -> Media {
        Media {
            kind: MediaKind::Video,
            url: self.watch_url,
            thumb_url: self.poster,
            width: 0,
            height: 0,
            ocr_text: None,
            ocr_lang: None,
            embedding_id: None,
            phash: None,
            provider: Some(self.provider.as_str().to_string()),
        }
    }
}

/// Recognise an embed or link URL from one of the known providers.
///
/// Accepts the shapes that actually appear in the wild — `youtube.com/embed/ID`,
/// `youtube.com/watch?v=ID`, `youtu.be/ID`, `youtube-nocookie.com/embed/ID`,
/// `dailymotion.com/embed/video/ID`, `dailymotion.com/video/ID`, `dai.ly/ID`,
/// `player.vimeo.com/video/ID`, `vimeo.com/ID` — and nothing else. An unrecognised host is
/// `None`, not a guess.
pub fn from_url(raw: &str) -> Option<Video> {
    let url = url::Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url
        .host_str()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let segments: Vec<&str> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();

    let (provider, id): (Provider, String) = match host.as_str() {
        "youtube.com" | "m.youtube.com" | "youtube-nocookie.com" => {
            let id = match segments.first().copied() {
                Some("embed") | Some("shorts") | Some("v") => {
                    segments.get(1).map(|s| s.to_string())
                }
                Some("watch") => url
                    .query_pairs()
                    .find(|(k, _)| k == "v")
                    .map(|(_, v)| v.into_owned()),
                _ => None,
            }?;
            (Provider::YouTube, id)
        }
        "youtu.be" => (Provider::YouTube, segments.first()?.to_string()),
        "dailymotion.com" | "geo.dailymotion.com" => {
            let id = match segments.first().copied() {
                Some("embed") => match segments.get(1).copied() {
                    Some("video") => segments.get(2).copied(),
                    _ => None,
                },
                Some("video") => segments.get(1).copied(),
                _ => None,
            }?;
            // Dailymotion ids may carry a `_title-slug` suffix on watch pages.
            let id = id.split('_').next().unwrap_or(id);
            (Provider::Dailymotion, id.to_string())
        }
        "dai.ly" => (Provider::Dailymotion, segments.first()?.to_string()),
        "vimeo.com" => (Provider::Vimeo, segments.last()?.to_string()),
        "player.vimeo.com" => match segments.first().copied() {
            Some("video") => (Provider::Vimeo, segments.get(1)?.to_string()),
            _ => return None,
        },
        _ => return None,
    };

    if !is_plausible_id(&id) {
        return None;
    }

    Some(match provider {
        Provider::YouTube => Video {
            provider,
            watch_url: format!("https://www.youtube.com/watch?v={id}"),
            // `hqdefault` exists for every video; `maxresdefault` does not.
            poster: Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")),
        },
        Provider::Dailymotion => Video {
            provider,
            watch_url: format!("https://www.dailymotion.com/video/{id}"),
            poster: Some(format!("https://www.dailymotion.com/thumbnail/video/{id}")),
        },
        Provider::Vimeo => Video {
            provider,
            watch_url: format!("https://vimeo.com/{id}"),
            // Vimeo's poster needs an API call, and we do not make one.
            poster: None,
        },
        Provider::SelfHosted => unreachable!("self-hosted video never comes from a URL match"),
    })
}

/// A self-hosted `<video>` on the page itself.
///
/// The watch page is the page it sits on, because that is where a person watches it. The `src`
/// is deliberately **not** stored: it is a stream URL, and storing one is how a byte gets fetched
/// later by something that did not read this comment.
pub fn self_hosted(page_url: &str, poster: Option<String>) -> Video {
    Video {
        provider: Provider::SelfHosted,
        watch_url: page_url.to_string(),
        poster,
    }
}

/// Provider ids are short and alphanumeric-ish. Anything else is a path we misread as an id.
fn is_plausible_id(id: &str) -> bool {
    let n = id.chars().count();
    (3..=64).contains(&n)
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stored_url_is_always_a_watch_page_never_a_stream() {
        // The whole reason this module stores what it stores. An embed URL becomes a watch URL;
        // a self-hosted video points at its page, and its `src` is never kept.
        let v = from_url("https://www.youtube.com/embed/dQw4w9WgXcQ").unwrap();
        assert_eq!(v.watch_url, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");

        let own = self_hosted("https://example.dz/article", None);
        assert_eq!(own.watch_url, "https://example.dz/article");
        let media = own.into_media();
        assert!(!media.url.ends_with(".mp4"));
    }

    #[test]
    fn every_youtube_shape_resolves_to_one_id() {
        for u in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?rel=0",
            "https://m.youtube.com/watch?v=dQw4w9WgXcQ&t=10",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
        ] {
            let v = from_url(u).unwrap_or_else(|| panic!("{u} should parse"));
            assert_eq!(v.provider, Provider::YouTube);
            assert_eq!(v.watch_url, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
            assert_eq!(
                v.poster.as_deref(),
                Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg")
            );
        }
    }

    #[test]
    fn dailymotion_ids_lose_their_title_slug() {
        let v = from_url("https://www.dailymotion.com/video/x8abcde_some-title-here").unwrap();
        assert_eq!(v.watch_url, "https://www.dailymotion.com/video/x8abcde");
        assert!(v.poster.is_some());
        let e = from_url("https://geo.dailymotion.com/embed/video/x8abcde").unwrap();
        assert_eq!(e.watch_url, v.watch_url);
    }

    #[test]
    fn vimeo_gets_no_poster_because_that_would_cost_a_request() {
        let v = from_url("https://player.vimeo.com/video/76979871").unwrap();
        assert_eq!(v.watch_url, "https://vimeo.com/76979871");
        assert!(v.poster.is_none());
    }

    #[test]
    fn an_unknown_host_is_not_a_guess() {
        assert!(from_url("https://example.com/embed/abc123").is_none());
        assert!(from_url("https://www.youtube.com/").is_none());
        assert!(from_url("javascript:alert(1)").is_none());
        // A path we might misread as an id.
        assert!(from_url("https://www.youtube.com/embed/../../etc").is_none());
    }

    #[test]
    fn the_provider_is_named_on_the_media() {
        let m = from_url("https://youtu.be/dQw4w9WgXcQ")
            .unwrap()
            .into_media();
        assert_eq!(m.kind, MediaKind::Video);
        assert_eq!(m.provider.as_deref(), Some("youtube"));
    }

    /// M9-T01.4 — **no code path downloads video bytes.** Pinned at the source so the commit
    /// that adds a video MIME to the fetcher, or stores a stream URL, fails here with the reason.
    #[test]
    fn no_code_path_downloads_video_bytes() {
        let fetch = include_str!("fetch.rs");
        let indexable_start = fetch
            .find("const INDEXABLE")
            .expect("the fetcher's MIME list");
        let indexable_end = fetch[indexable_start..].find("];").unwrap() + indexable_start;
        let indexable = &fetch[indexable_start..indexable_end];
        assert!(
            !indexable.contains("video/") && !indexable.contains("audio/"),
            "the fetcher must never accept a media MIME: a video is metadata here, never bytes \
             (M2-T10.8)"
        );
        // And this module never keeps a stream: the only `src` it touches is an iframe's.
        let me = include_str!("video.rs");
        let code = &me[..me.find("#[cfg(test)]").unwrap()];
        assert!(!code.contains(".mp4") && !code.contains("videoplayback"));
    }
}
