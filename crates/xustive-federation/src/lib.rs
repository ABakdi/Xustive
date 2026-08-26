//! Query-time federation with a self-hosted SearXNG aggregator ([[ADR-0017]], [[Federation
//! Gateway]], M7-T04).
//!
//! SearXNG is an open-source metasearch engine: give it a query and it returns a ranked,
//! de-duplicated list of results aggregated from many engines. We run our own instance (on the
//! egress network — see [[Federation Gateway]]), so a user query reaches third-party engines only
//! *through our SearXNG*, carrying no client identity, IP, cookie, or session.
//!
//! Unlike the Brave connector (`xustive_ingest::brave`), which takes only URLs for offline discovery, federation keeps the
//! **title and snippet** too: a federated hit is blended into the live answer and its URL is fed to
//! the crawler so the page is indexed and, thereafter, answered locally. The engine name rides along
//! as provenance, so a blended result stays distinguishable in ranking and on the console.
//!
//! This module is the client — the pure response parser and the HTTP call. Reaching SearXNG within
//! a latency budget, blending, the crawl-feed, and the allowlist belong to the [[Federation Gateway]]
//! that calls it; here we keep the part most likely to drift (SearXNG's JSON shape) pure and
//! fixture-tested.

pub mod llm;

use serde::{Deserialize, Serialize};

/// One federated result. Carries what a blended answer needs — the destination, the aggregator's
/// title and snippet, its rank, and which engine surfaced it (provenance).
///
/// Serialisable both ways: the [[Federation Gateway]] serialises these in its `/federate` response,
/// and the serving API deserialises them to blend — one shared shape, no drift between the two.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FederatedHit {
    pub url: String,
    pub title: String,
    pub snippet: String,
    /// The upstream engine SearXNG credits for this result (e.g. `duckduckgo`, `wikipedia`). Empty
    /// when SearXNG did not name one.
    pub engine: String,
    /// 1-based position in SearXNG's returned order.
    pub rank: usize,
    /// The image or video this hit *is*, for the Images and Videos categories (M9-T06). `None` for
    /// web hits, and defaulted on the wire so a gateway and an API of different builds still agree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<FederatedMedia>,
}

/// What an image or video hit carries beyond a page. Never bytes: SearXNG hands us URLs, and
/// every one of them is proxied or linked, not fetched, by the serving side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FederatedMedia {
    /// `image` or `video`.
    pub kind: String,
    /// The full-size image, or for video the watch page (same as `url`).
    pub src: String,
    /// A small preview when the engine offers one. Preferred for tiles: it is what the engine
    /// already serves at thumbnail size, and it spares the origin a full-size request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    /// `"3504 x 2336"` for images, seconds or `m:ss` for video — whatever the engine said, kept as
    /// text because the formats vary by engine and a tile only displays it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Which SearXNG category to ask. Web is the M7 default; Images and Videos arrived with M9-T06 and
/// travel through the same gateway, budget and fail-open path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    #[default]
    Web,
    Images,
    Videos,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Web => "general",
            Category::Images => "images",
            Category::Videos => "videos",
        }
    }

    /// From a vertical name; anything unknown is web, the safe default.
    pub fn from_vertical(v: Option<&str>) -> Self {
        match v {
            Some("images") => Category::Images,
            Some("videos") => Category::Videos,
            _ => Category::Web,
        }
    }
}

/// The slice of SearXNG's `format=json` response we read: `results[].{url,title,content,engine}`.
#[derive(Debug, Deserialize)]
struct SearxngResponse {
    /// Untyped on purpose, and parsed one result at a time below: SearXNG's engines are uneven,
    /// and one result with a field of an unexpected shape used to fail the deserialisation of the
    /// *whole* response — a hundred good video hits thrown away for one odd one (M9-T06).
    #[serde(default)]
    results: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SearxngResult {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    /// SearXNG calls the snippet `content`.
    #[serde(default)]
    content: String,
    /// Either a single `engine` or a list of `engines`; SearXNG emits both across versions.
    #[serde(default)]
    engine: String,
    #[serde(default)]
    engines: Vec<String>,
    /// `images.html` / `videos.html` — how SearXNG says what kind of result this is.
    // `Option`, not `String` with a default: several engines send `null` rather than omitting a
    // field, and `#[serde(default)]` on a `String` refuses `null`.
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    img_src: Option<String>,
    #[serde(default)]
    thumbnail_src: Option<String>,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    resolution: Option<String>,
    /// Video length. A number of seconds or `m:ss`, depending on the engine.
    #[serde(default)]
    length: serde_json::Value,
}

/// Parse SearXNG's JSON into federated hits, dropping blank URLs and preserving order as rank. Pure,
/// so the shape-handling is tested without a network — the part most likely to drift when SearXNG
/// changes its response between versions.
pub fn parse_results(body: &str) -> Vec<FederatedHit> {
    let Ok(resp) = serde_json::from_str::<SearxngResponse>(body) else {
        return Vec::new();
    };
    resp.results
        .into_iter()
        .filter_map(|v| serde_json::from_value::<SearxngResult>(v).ok())
        .filter(|r| !r.url.trim().is_empty())
        .enumerate()
        .map(|(i, r)| {
            let engine = if !r.engine.trim().is_empty() {
                r.engine
            } else {
                r.engines
                    .into_iter()
                    .find(|e| !e.trim().is_empty())
                    .unwrap_or_default()
            };
            let img_src = non_empty(r.img_src.unwrap_or_default());
            let media = match r.template.as_deref().unwrap_or("") {
                // An image hit without an image is a web hit that lost its way; dropped below.
                "images.html" if img_src.is_some() => Some(FederatedMedia {
                    kind: "image".into(),
                    src: img_src.unwrap_or_default(),
                    thumb: non_empty(r.thumbnail_src.unwrap_or_default()),
                    detail: non_empty(r.resolution.unwrap_or_default()),
                }),
                "videos.html" => Some(FederatedMedia {
                    kind: "video".into(),
                    // The watch page. Never `iframe_src`: an embed URL is a player, and a player
                    // is a third-party page load the reader did not choose (ADR-0021).
                    src: r.url.clone(),
                    thumb: non_empty(r.thumbnail.unwrap_or_default()),
                    detail: match r.length {
                        serde_json::Value::String(s) => non_empty(s),
                        serde_json::Value::Number(n) => Some(n.to_string()),
                        _ => None,
                    },
                }),
                _ => None,
            };
            FederatedHit {
                url: r.url,
                title: r.title,
                snippet: r.content,
                engine,
                rank: i + 1,
                media,
            }
        })
        .collect()
}

fn non_empty(s: String) -> Option<String> {
    let t = s.trim();
    (!t.is_empty() && t != "None").then(|| t.to_string())
}

/// Why a federation request failed.
#[derive(Debug, thiserror::Error)]
pub enum FederationError {
    #[error("searxng request failed: {0}")]
    Http(reqwest::Error),
    #[error("searxng returned {status}")]
    Status { status: u16 },
}

/// The URL is stripped off every wrapped transport error (BUG-033). reqwest embeds the full
/// request URL in its `Display` — and the SearXNG request carries the **query text** as a `?q=`
/// parameter — so a plain `#[from]` meant every `error = %e` log line during an outage printed the
/// user's query verbatim, the exact thing ADR-0008 forbids. Scrubbing here, at the one conversion
/// boundary, makes the leak inexpressible downstream rather than a per-call-site discipline.
impl From<reqwest::Error> for FederationError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e.without_url())
    }
}

/// A client for a self-hosted SearXNG instance. Holds the endpoint and a per-query hit cap.
#[derive(Clone)]
pub struct SearxngClient {
    http: reqwest::Client,
    base: String,
    max_hits: usize,
}

impl SearxngClient {
    /// Build a client. Returns `None` for an empty endpoint — federation is inert without one rather
    /// than erroring, so a deployment with federation off simply does nothing. `timeout` bounds a
    /// single call; the caller layers its own (tighter) query-time budget on top.
    pub fn new(base: &str, max_hits: usize, timeout: std::time::Duration) -> Option<Self> {
        let base = base.trim().trim_end_matches('/');
        if base.is_empty() {
            return None;
        }
        Some(Self {
            http: reqwest::Client::builder().timeout(timeout).build().ok()?,
            base: base.to_string(),
            max_hits: max_hits.clamp(1, 50),
        })
    }

    /// Search one query and return the federated hits. Asks SearXNG for JSON; `country`/`language`
    /// are left unset — the query text carries the intent, and over-constraining loses the
    /// mixed-script and dialect queries federation most exists to help.
    pub async fn search(&self, query: &str) -> Result<Vec<FederatedHit>, FederationError> {
        self.search_in(query, Category::Web).await
    }

    /// Search one category. Images and Videos are the same call with a `categories` parameter;
    /// SearXNG fans out to its image and video engines and answers in the same JSON shape with a
    /// `template` naming the kind.
    pub async fn search_in(
        &self,
        query: &str,
        category: Category,
    ) -> Result<Vec<FederatedHit>, FederationError> {
        let url = format!("{}/search", self.base);
        let resp = self
            .http
            .get(&url)
            .header("Accept", "application/json")
            .query(&[
                ("q", query),
                ("format", "json"),
                ("categories", category.as_str()),
            ])
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(FederationError::Status {
                status: status.as_u16(),
            });
        }
        let mut hits = parse_results(&resp.text().await?);
        hits.truncate(self.max_hits);
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hits_with_rank_and_provenance() {
        let body = r#"{
            "results": [
                {"url": "https://www.aps.dz/a", "title": "A", "content": "snippet a", "engine": "duckduckgo"},
                {"url": "", "title": "blank"},
                {"url": "https://elkhabar.com/b", "title": "B", "content": "snippet b", "engines": ["bing", "brave"]}
            ]
        }"#;
        let hits = parse_results(body);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://www.aps.dz/a");
        assert_eq!(hits[0].engine, "duckduckgo");
        assert_eq!(hits[0].rank, 1);
        // Blank URL dropped; the next kept hit is rank 2, and `engines[]` fills in for `engine`.
        assert_eq!(hits[1].url, "https://elkhabar.com/b");
        assert_eq!(hits[1].snippet, "snippet b");
        assert_eq!(hits[1].engine, "bing");
        assert_eq!(hits[1].rank, 2);
    }

    #[test]
    fn a_malformed_or_empty_response_yields_nothing() {
        // An error page, a query SearXNG had nothing for, or garbage must be an empty list, not a
        // panic — the fail-open contract starts here.
        assert!(parse_results("not json").is_empty());
        assert!(parse_results(r#"{"results":[]}"#).is_empty());
        assert!(parse_results(r#"{"error":"upstream"}"#).is_empty());
    }

    #[test]
    fn an_empty_endpoint_makes_an_inert_client() {
        let t = std::time::Duration::from_secs(5);
        assert!(SearxngClient::new("", 10, t).is_none());
        assert!(SearxngClient::new("   ", 10, t).is_none());
        assert!(SearxngClient::new("http://xustive-searxng:8080", 10, t).is_some());
        // Trailing slash is normalised so the `/search` join never doubles it.
        assert!(SearxngClient::new("http://xustive-searxng:8080/", 10, t).is_some());
    }

    #[tokio::test]
    async fn a_transport_error_never_renders_the_query() {
        // BUG-033 regression. The SearXNG request carries the query as `?q=`, and reqwest embeds
        // the request URL in its error Display — so an unscrubbed error rendered the query text
        // into every `error = %e` log line during an outage. Port 9 (discard) refuses immediately:
        // a real transport error, offline and fast.
        let client =
            SearxngClient::new("http://127.0.0.1:9", 10, std::time::Duration::from_secs(2))
                .expect("client builds");
        let err = client
            .search("SECRET_QUERY_MARKER")
            .await
            .expect_err("connection to port 9 must fail");
        let rendered = format!("{err} / {err:?}");
        assert!(
            !rendered.contains("SECRET_QUERY_MARKER"),
            "query text leaked into the rendered error: {rendered}"
        );
    }

    #[test]
    fn an_image_result_carries_its_image_and_a_video_result_its_watch_page() {
        // Shapes copied from a live SearXNG answer for "alger" (M9-T06).
        let body = r#"{"results":[
          {"template":"images.html","url":"https://observalgerie.com/visiter-alger","title":"Visiter Alger",
           "content":"…","thumbnail_src":"https://tse1.mm.bing.net/th/id/OIP.z","img_src":"https://observalgerie.com/wp-content/uploads/vue.jpg",
           "resolution":"1980 x 1200","engine":"duckduckgo images"},
          {"template":"videos.html","url":"https://www.youtube.com/watch?v=TqIensHhtyY","title":"Mali - Algérie",
           "content":"…","iframe_src":"https://www.youtube-nocookie.com/embed/TqIensHhtyY",
           "thumbnail":"https://i.ytimg.com/vi/TqIensHhtyY/hqdefault.jpg","length":"184.0","engine":"google videos"},
          {"template":"videos.html","url":"https://www.tiktok.com/@x/video/1","title":"t","thumbnail":"https://t/x.jpg","length":"0:49","engine":"duckduckgo videos"},
          {"template":"images.html","url":"https://example.com/no-image","title":"broken","img_src":"","engine":"bing images"}
        ]}"#;
        let hits = parse_results(body);
        assert_eq!(hits.len(), 4);
        let img = hits[0].media.as_ref().unwrap();
        assert_eq!(img.kind, "image");
        assert_eq!(
            img.src,
            "https://observalgerie.com/wp-content/uploads/vue.jpg"
        );
        assert_eq!(
            img.thumb.as_deref(),
            Some("https://tse1.mm.bing.net/th/id/OIP.z")
        );
        assert_eq!(img.detail.as_deref(), Some("1980 x 1200"));

        let vid = hits[1].media.as_ref().unwrap();
        assert_eq!(vid.kind, "video");
        // The watch page, never the iframe: an embed is a player.
        assert_eq!(vid.src, "https://www.youtube.com/watch?v=TqIensHhtyY");
        assert!(!vid.src.contains("embed"));
        assert_eq!(vid.detail.as_deref(), Some("184.0"));
        assert_eq!(
            hits[2].media.as_ref().unwrap().detail.as_deref(),
            Some("0:49")
        );

        // An image result with no image is not an image.
        assert!(hits[3].media.is_none());
    }

    #[test]
    fn a_null_field_or_one_malformed_result_does_not_drop_the_rest() {
        // The live failure: SearXNG's video engines send `"thumbnail": null` and the like, and a
        // strict String field failed the deserialisation of the WHOLE body — 118 video hits became
        // zero. Every result is now parsed on its own, and a null reads as absent.
        let body = r#"{"results":[
          {"template":"videos.html","url":"https://www.youtube.com/watch?v=a","title":"ok","thumbnail":null,"img_src":null,"length":184,"engine":"youtube"},
          {"template":"videos.html","url":12345,"title":"malformed url type"},
          {"template":"images.html","url":"https://p/x","title":"img","img_src":"https://p/x.jpg","thumbnail_src":null,"resolution":null,"engine":"bing images"}
        ]}"#;
        let hits = parse_results(body);
        assert_eq!(hits.len(), 2, "the malformed result is skipped, not fatal");
        assert_eq!(hits[0].media.as_ref().unwrap().kind, "video");
        assert!(hits[0].media.as_ref().unwrap().thumb.is_none());
        assert_eq!(
            hits[0].media.as_ref().unwrap().detail.as_deref(),
            Some("184")
        );
        assert_eq!(hits[1].media.as_ref().unwrap().kind, "image");
    }

    #[test]
    fn a_web_hit_serialises_without_a_media_field_so_older_builds_still_read_it() {
        let hit = FederatedHit {
            url: "https://a".into(),
            title: "t".into(),
            snippet: "s".into(),
            engine: "e".into(),
            rank: 1,
            media: None,
        };
        let json = serde_json::to_string(&hit).unwrap();
        assert!(!json.contains("media"));
        // And a gateway reply without the field still parses.
        let back: FederatedHit =
            serde_json::from_str(r#"{"url":"u","title":"t","snippet":"s","engine":"e","rank":1}"#)
                .unwrap();
        assert!(back.media.is_none());
    }

    #[test]
    fn the_category_names_are_what_searxng_expects() {
        assert_eq!(Category::from_vertical(Some("images")).as_str(), "images");
        assert_eq!(Category::from_vertical(Some("videos")).as_str(), "videos");
        assert_eq!(Category::from_vertical(Some("news")).as_str(), "general");
        assert_eq!(Category::from_vertical(None), Category::Web);
    }
}
