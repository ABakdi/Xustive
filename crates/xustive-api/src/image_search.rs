//! Reverse image search — a picture in, pictures out ([[Milestone 10 - Reverse Image Search]],
//! [[Vector Index]]; first built as M3-T05, rebuilt for M10).
//!
//! An uploaded photo is embedded and *described* (CLIP, via the embed sidecar), the vector is
//! searched against Qdrant, and every matching **image** comes back with the page it lives on —
//! the image that matched, never a sibling from the same page. Three groups, in order:
//!
//! - `same`: a perceptual-hash match (Hamming ≤ [`SAME_HAMMING`]) or cosine ≥ [`SAME_COSINE`] —
//!   the picture itself, where it appears on the Algerian web;
//! - `similar`: everything else above the configured threshold;
//! - `web`: what a metasearch engine returns for the *words* the picture was described with.
//!
//! # The web leg sends words, never the picture ([[ADR-0028 - Reverse Image Search Sends Words to
//! the Web, Never the Picture]])
//!
//! The labels — the subjects CLIP is sure of, the style when it is telling — become one text
//! query in the images category, through the one gateway, exactly like a typed query. The hits
//! ride the eager index like every federated hit, the crawl follows, and next time they are local
//! and ranked visually. The image bytes do not reach that leg; a test pins it.
//!
//! # Isolation
//!
//! Vector search being unavailable must never affect text search ([[Vector Index]] §7). The
//! engine is an `Option` on the state: absent when `[vector] enabled = false` or the services are
//! unreachable, in which case the endpoint returns a clean 503 and nothing else is touched.
//!
//! # Privacy
//!
//! The uploaded image is embedded in memory and never stored; the vector and the words are
//! transient. The bytes arrive as a raw POST body so they never reach a URL or an access log.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::Json;
use serde::Serialize;
use serde_json::Value;

use xustive_vector::{Embedder, SearchFilter, Store};

use crate::error::ApiError;
use crate::search::{to_card, ResultCard};
use crate::state::AppState;

/// Hamming distance on the 64-bit dHash under which two images are the same picture — a
/// re-encode, a resize, a small crop. Six bits is the usual figure; eight starts admitting siblings.
pub const SAME_HAMMING: u32 = 6;
/// Cosine above which CLIP says "this is the picture" even when the hash disagrees (a heavier crop).
pub const SAME_COSINE: f32 = 0.92;
/// The local leg looks further than the page-collapsing M3 version did: the unit is the image now,
/// and the chips filter the set on screen without a second upload.
pub const IMAGE_SEARCH_LIMIT: usize = 80;
/// How many subjects become words for the web leg.
const WEB_LABELS: usize = 3;

/// The built image-similarity engine: a vector store and an embedder. Present only when enabled.
#[derive(Clone)]
pub struct ImageSearch {
    pub store: Store,
    pub embedder: Arc<dyn Embedder>,
    pub search_limit: usize,
    pub ef_search: usize,
    pub score_threshold: f32,
}

impl ImageSearch {
    /// Build from `[vector]` config, or `None` when disabled. Constructing the clients cannot reach
    /// the network yet, so this never blocks startup; `ensure_collection` is called separately.
    pub fn from_config(cfg: &xustive_core::config::VectorConfig) -> Option<Self> {
        if !cfg.enabled {
            return None;
        }
        let timeout = std::time::Duration::from_millis(cfg.timeout_ms);
        let key = (!cfg.qdrant_key.is_empty()).then(|| cfg.qdrant_key.clone());
        let store = Store::new(&cfg.qdrant_url, key, cfg.collection.clone(), timeout).ok()?;
        let embedder =
            xustive_vector::SidecarEmbedder::new(&cfg.embedder_endpoint, timeout).ok()?;
        Some(Self {
            store,
            embedder: Arc::new(embedder),
            search_limit: cfg.search_limit.max(IMAGE_SEARCH_LIMIT),
            ef_search: cfg.ef_search,
            score_threshold: cfg.score_threshold(),
        })
    }
}

/// One image on the page.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImageHit {
    pub url: String,
    /// The smaller picture to show, when the index has one; the web tier signs whichever it uses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Cosine for the local groups; absent for the web group, which is not ranked visually.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    pub group: Group,
    pub page: PageRef,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    Same,
    Similar,
    Web,
}

/// Where the image lives — enough for a tile, never the whole card.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PageRef {
    pub id: String,
    pub title: String,
    pub url: String,
    pub display_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_web: bool,
}

/// What the picture is, in words: the style, the file type, and the subjects worth saying.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct QueryInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext: Option<String>,
    pub labels: Vec<String>,
    /// The text the web leg was asked, so the page can say "from the web, by description".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_query: Option<String>,
}

/// Counts over the images on the page — exact for the set, which is what a chip must show.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Facets {
    pub ext: Vec<(String, usize)>,
    pub style: Vec<(String, usize)>,
}

#[derive(Serialize)]
pub struct ImageSearchResponse {
    pub images: Vec<ImageHit>,
    pub query: QueryInfo,
    pub facets: Facets,
    /// The pages, collapsed and in similarity order — the M3 shape, kept one release for the OCR
    /// page's "find similar" list.
    pub results: Vec<ResultCard>,
    /// How many local images matched before grouping — "12 similar images across 5 pages".
    pub matched_images: usize,
}

/// `POST /api/v1/search/image` — the images that are, or look like, the uploaded one.
pub async fn handler(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<ImageSearchResponse>, ApiError> {
    let Some(engine) = state.image_search.clone() else {
        // Feature off or services down: a clean, specific unavailable — never a 500, and never a
        // side effect on text search.
        return Err(ApiError::model_unavailable("image_search_unavailable"));
    };
    if body.is_empty() {
        return Err(ApiError::BadImage {
            code: "empty_image",
        });
    }
    if body.len() > state.config.media.max_image_bytes {
        return Err(ApiError::BadImage {
            code: "image_too_large",
        });
    }

    // What the picture is: its hash and type from the bytes, its vector and words from CLIP. An
    // embedder failure (sidecar down, undecodable image) is image search being unavailable.
    let query_hash = xustive_media::phash::dhash(&body, xustive_media::ocr::MAX_PIXELS);
    let query_ext = xustive_media::ext::from_bytes(&body).map(str::to_string);
    let described = engine.embedder.describe(body.to_vec()).await.map_err(|e| {
        tracing::warn!(error = %e, "CLIP describe failed");
        ApiError::model_unavailable("image_search_unavailable")
    })?;
    // The bytes are not needed past this line; the web leg below sees only `query`.
    drop(body);

    let hits = engine
        .store
        .search(
            &described.vector,
            engine.search_limit,
            engine.ef_search,
            engine.score_threshold,
            &SearchFilter::safe(),
        )
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "vector search failed");
            ApiError::model_unavailable("image_search_unavailable")
        })?;
    let matched_images = hits.len();

    // The words, and the query they make for the web.
    let labels = described.description.subject_labels(WEB_LABELS);
    let style = described.description.style_label();
    let web_query = web_query(&labels, style.as_deref());
    let query = QueryInfo {
        style,
        ext: query_ext,
        labels,
        web_query: web_query.clone(),
    };

    // Pages, one filtered query; then one ImageHit per vector hit, in similarity order.
    let order: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        hits.iter()
            .filter(|h| seen.insert(h.payload.document_id.clone()))
            .map(|h| h.payload.document_id.clone())
            .collect()
    };
    let pages = resolve_pages(&state, &order).await;
    let mut images: Vec<ImageHit> = hits
        .iter()
        .filter_map(|h| {
            let (card, raw) = pages.get(&h.payload.document_id)?;
            let group = group_of(h.score, query_hash.as_deref(), h.payload.phash.as_deref());
            Some(ImageHit {
                thumb_url: thumb_for(raw, &h.payload.media_url),
                url: h.payload.media_url.clone(),
                ext: h.payload.ext.clone().or_else(|| {
                    xustive_media::ext::from_url(&h.payload.media_url).map(str::to_string)
                }),
                style: h.payload.style.clone(),
                score: Some(h.score),
                group,
                page: PageRef {
                    id: card.id.clone(),
                    title: card.title.clone(),
                    url: card.url.clone(),
                    display_url: card.display_url.clone(),
                    source_name: Some(card.source_name.clone()),
                    from_web: card.from_web,
                },
            })
        })
        .collect();
    // `same` first, then `similar`, each by score — the ANN order interleaves them.
    images.sort_by(|a, b| {
        rank(a.group)
            .cmp(&rank(b.group))
            .then_with(|| b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)))
    });

    // The web leg: words in, pictures out, through the gateway, only when federation is on.
    if let (Some(q), Some(client)) = (&web_query, state.federator.as_ref()) {
        if state
            .federation_enabled
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            let hits = client
                .federate_in(
                    q,
                    xustive_ingest::federation::Category::Images,
                    Some(state.config.federation.budget_ms),
                )
                .await;
            crate::search::ingest_federated(&state, &hits);
            images.extend(hits.iter().filter_map(web_hit));
        }
    }

    let facets = facets_of(&images);

    // The M3 shape: pages collapsed by best score, in that order.
    let mut best: HashMap<String, f32> = HashMap::new();
    for h in &hits {
        let e = best
            .entry(h.payload.document_id.clone())
            .or_insert(f32::MIN);
        *e = e.max(h.score);
    }
    let results = order
        .iter()
        .filter_map(|id| {
            let (_, raw) = pages.get(id)?;
            let mut card = to_card(raw);
            card.score = best.get(id).copied().unwrap_or(0.0);
            Some(card)
        })
        .collect();

    Ok(Json(ImageSearchResponse {
        images,
        query,
        facets,
        results,
        matched_images,
    }))
}

fn rank(g: Group) -> u8 {
    match g {
        Group::Same => 0,
        Group::Similar => 1,
        Group::Web => 2,
    }
}

/// Which group a local hit belongs to: the hash decides "same" when it can, the cosine otherwise.
pub fn group_of(score: f32, query_hash: Option<&str>, hit_hash: Option<&str>) -> Group {
    let same_hash = match (query_hash, hit_hash) {
        (Some(a), Some(b)) => {
            xustive_media::phash::hamming(a, b).is_some_and(|d| d <= SAME_HAMMING)
        }
        _ => false,
    };
    if same_hash || score >= SAME_COSINE {
        Group::Same
    } else {
        Group::Similar
    }
}

/// The text the web is asked. Subjects first; the style only when it says something a subject
/// does not ("photo" is what most pictures are). `None` when there is nothing worth asking — a
/// picture with no confident subject gets no web leg rather than a random one.
pub fn web_query(labels: &[String], style: Option<&str>) -> Option<String> {
    let mut words: Vec<String> = labels.iter().map(|l| l.replace('_', " ")).collect();
    match style {
        Some("photo") | None => {}
        Some(s) => words.push(s.replace('_', " ").replace("render 3d", "3d render")),
    }
    (!words.is_empty()).then(|| words.join(" "))
}

/// Counts of the file types and styles among the images, most common first.
pub fn facets_of(images: &[ImageHit]) -> Facets {
    fn count<'a>(it: impl Iterator<Item = &'a str>) -> Vec<(String, usize)> {
        let mut m: HashMap<&str, usize> = HashMap::new();
        for k in it {
            *m.entry(k).or_default() += 1;
        }
        let mut v: Vec<(String, usize)> = m.into_iter().map(|(k, n)| (k.to_string(), n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v
    }
    Facets {
        ext: count(images.iter().filter_map(|i| i.ext.as_deref())),
        style: count(images.iter().filter_map(|i| i.style.as_deref())),
    }
}

/// A federated image hit as a tile: the engine's order, no score, flagged from the web.
fn web_hit(hit: &xustive_ingest::federation::FederatedHit) -> Option<ImageHit> {
    let media = hit.media.as_ref().filter(|m| m.kind == "image")?;
    Some(ImageHit {
        url: media.src.clone(),
        thumb_url: media.thumb.clone(),
        ext: xustive_media::ext::from_url(&media.src).map(str::to_string),
        style: None,
        score: None,
        group: Group::Web,
        page: PageRef {
            id: String::new(),
            title: hit.title.clone(),
            url: hit.url.clone(),
            display_url: hit.url.clone(),
            source_name: Some(hit.engine.clone()),
            from_web: true,
        },
    })
}

/// The index's thumbnail for this image on this page, if it recorded one.
fn thumb_for(raw: &Value, media_url: &str) -> Option<String> {
    raw.get("media")?
        .as_array()?
        .iter()
        .find(|m| m.get("url").and_then(Value::as_str) == Some(media_url))?
        .get("thumb_url")?
        .as_str()
        .map(str::to_string)
}

/// Fetch the pages by id in one filtered query: the card, and the raw hit for its `media[]`.
async fn resolve_pages(state: &AppState, order: &[String]) -> HashMap<String, (ResultCard, Value)> {
    if order.is_empty() {
        return HashMap::new();
    }
    let quoted: Vec<String> = order.iter().map(|id| format!("\"{id}\"")).collect();
    let filter = format!("id IN [{}]", quoted.join(", "));
    let query = xustive_search::Query::new("")
        .filter(filter)
        .limit(order.len());
    let index = state.documents_index();
    let hits = match state.search.search::<Value>(&index, &query).await {
        Ok(h) => h.hits,
        Err(e) => {
            tracing::warn!(error = %e, "resolving image-search documents failed");
            return HashMap::new();
        }
    };
    hits.into_iter()
        .filter_map(|h| {
            let id = h.get("id").and_then(Value::as_str)?.to_string();
            Some((id, (to_card(&h), h)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(ext: Option<&str>, style: Option<&str>, group: Group) -> ImageHit {
        ImageHit {
            url: "https://x.dz/a.jpg".into(),
            thumb_url: None,
            ext: ext.map(str::to_string),
            style: style.map(str::to_string),
            score: Some(0.8),
            group,
            page: PageRef {
                id: "d1".into(),
                title: "t".into(),
                url: "https://x.dz/p".into(),
                display_url: "x.dz/p".into(),
                source_name: None,
                from_web: false,
            },
        }
    }

    #[test]
    fn the_hash_says_same_and_the_cosine_says_same_and_the_rest_is_similar() {
        // Two bits apart: a re-encode of the same picture, whatever CLIP thinks of it.
        assert_eq!(
            group_of(0.80, Some("ff00ff00ff00ff00"), Some("ff00ff00ff00ff03")),
            Group::Same
        );
        // Far apart hashes, but CLIP is nearly certain: a heavy crop.
        assert_eq!(
            group_of(0.95, Some("ff00ff00ff00ff00"), Some("00ff00ff00ff00ff")),
            Group::Same
        );
        // Neither.
        assert_eq!(
            group_of(0.80, Some("ff00ff00ff00ff00"), Some("00ff00ff00ff00ff")),
            Group::Similar
        );
        assert_eq!(group_of(0.80, None, None), Group::Similar);
    }

    #[test]
    fn the_web_is_asked_in_words_and_only_when_there_are_some() {
        assert_eq!(
            web_query(&["casbah".into(), "mosque".into()], Some("photo")).as_deref(),
            Some("casbah mosque")
        );
        assert_eq!(
            web_query(&["city_street".into()], Some("render_3d")).as_deref(),
            Some("city street 3d render")
        );
        assert_eq!(web_query(&[], Some("photo")), None);
        assert_eq!(
            web_query(&[], Some("screenshot")).as_deref(),
            Some("screenshot")
        );
    }

    #[test]
    fn facets_count_the_set_on_screen_exactly() {
        let f = facets_of(&[
            hit(Some("png"), Some("logo"), Group::Same),
            hit(Some("jpg"), Some("photo"), Group::Similar),
            hit(Some("png"), None, Group::Web),
        ]);
        assert_eq!(f.ext, vec![("png".to_string(), 2), ("jpg".to_string(), 1)]);
        assert_eq!(
            f.style,
            vec![("logo".to_string(), 1), ("photo".to_string(), 1)]
        );
    }

    #[test]
    fn response_serialises_with_snake_case_fields_and_groups_in_lowercase() {
        let resp = ImageSearchResponse {
            images: vec![hit(Some("png"), None, Group::Same)],
            query: QueryInfo::default(),
            facets: Facets::default(),
            results: Vec::new(),
            matched_images: 3,
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["matched_images"], 3);
        assert_eq!(v["images"][0]["group"], "same");
        assert!(v["results"].is_array());
    }

    #[test]
    fn disabled_config_builds_no_engine() {
        let cfg = xustive_core::config::VectorConfig::default(); // enabled = false
        assert!(ImageSearch::from_config(&cfg).is_none());
    }

    #[test]
    fn enabled_config_builds_an_engine_that_looks_further_than_before() {
        let cfg = xustive_core::config::VectorConfig {
            enabled: true,
            ..Default::default()
        };
        let engine = ImageSearch::from_config(&cfg).expect("engine builds when enabled");
        assert_eq!(engine.ef_search, 64);
        assert!((engine.score_threshold - 0.75).abs() < 1e-6);
        assert_eq!(engine.search_limit, IMAGE_SEARCH_LIMIT);
    }

    #[test]
    fn the_web_leg_never_sees_the_body() {
        // The handler drops `body` before the federation call; this reads the source to keep it so.
        let src = include_str!("image_search.rs");
        let handler =
            &src[src.find("pub async fn handler").unwrap()..src.find("fn rank(").unwrap()];
        let drop_at = handler.find("drop(body)").expect("the body is dropped");
        let web_at = handler.find("federate_in(").expect("the web leg exists");
        assert!(
            drop_at < web_at,
            "the body must be gone before the web leg runs"
        );
        assert!(
            !handler[web_at..].contains("body"),
            "nothing after the web leg names the body"
        );
    }
}
