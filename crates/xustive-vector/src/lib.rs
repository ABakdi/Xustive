//! Qdrant vector index client — CLIP image-embedding ANN search ([[Vector Index]], C07).
//!
//! Reverse image search: "find posts containing this image". One point per media item (a post with
//! four images is four points sharing a `document_id`), 512-d CLIP embeddings, cosine distance on
//! L2-normalised vectors, int8-quantised so 5M vectors fit ~2.5 GB resident.
//!
//! # Why a hand-written REST client
//!
//! The `qdrant-client` crate pulls gRPC, `tonic`, and a large transitive tree. The operations here
//! are a handful of JSON POSTs, and the rest of the system already speaks `reqwest`. A thin REST
//! client keeps the dependency surface small and the wire format reviewable in a diff — the same
//! reasoning [[Vector Index]] gives for keeping this separate from the lexical index.
//!
//! # Isolation
//!
//! Vector search being down must never affect text search ([[Vector Index]] §7). Every method
//! returns a `Result`; the caller treats an error as "no similar images", not as a failed request.
//! This client never panics and never blocks the search path on the vector store's health.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub mod embed;
pub use embed::{Embedder, SidecarEmbedder};

pub const DEFAULT_COLLECTION: &str = "image_clip";
/// CLIP ViT-B/32 embedding dimensionality.
pub const DIM: usize = 512;

#[derive(Debug, thiserror::Error)]
pub enum VectorError {
    #[error("qdrant unreachable: {0}")]
    Unreachable(String),
    #[error("qdrant returned {status}: {body}")]
    Backend { status: u16, body: String },
    #[error("malformed response: {0}")]
    Decode(String),
}

/// One image embedding to store.
///
/// `id` is derived from the media URL ([`point_id`]) so re-indexing the same image overwrites its
/// point rather than duplicating it. The vector must be **L2-normalised** by the caller — cosine on
/// a normalised vector is a dot product, and normalising once at write time keeps query-time cheap
/// ([[Vector Index]] §4).
#[derive(Debug, Clone)]
pub struct Point {
    pub id: u64,
    pub vector: Vec<f32>,
    pub payload: Payload,
}

/// The filterable payload stored with each point.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Payload {
    pub document_id: String,
    pub media_url: String,
    #[serde(default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub published_at: Option<i64>,
    #[serde(default)]
    pub is_nsfw: bool,
    #[serde(default)]
    pub phash: Option<String>,
}

/// A search result: the stored payload and its similarity score.
#[derive(Debug, Clone)]
pub struct Hit {
    pub id: u64,
    pub score: f32,
    pub payload: Payload,
}

/// Filters applied at search time. All default to the permissive/safe choice.
#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    /// Drop NSFW-scored images from results. Defaults to true via [`SearchFilter::safe`].
    pub exclude_nsfw: bool,
    pub source_type: Option<String>,
    /// Only images whose parent document was published at/after this unix-seconds instant.
    pub published_after: Option<i64>,
}

impl SearchFilter {
    /// The default a user-facing search uses: NSFW excluded, no other constraint.
    pub fn safe() -> Self {
        Self {
            exclude_nsfw: true,
            ..Default::default()
        }
    }

    fn to_qdrant(&self) -> Option<Value> {
        let mut must = Vec::new();
        if self.exclude_nsfw {
            must.push(json!({ "key": "is_nsfw", "match": { "value": false } }));
        }
        if let Some(st) = &self.source_type {
            must.push(json!({ "key": "source_type", "match": { "value": st } }));
        }
        if let Some(after) = self.published_after {
            must.push(json!({ "key": "published_at", "range": { "gte": after } }));
        }
        if must.is_empty() {
            None
        } else {
            Some(json!({ "must": must }))
        }
    }
}

/// A stable point id for a media URL.
///
/// The first 8 bytes of a BLAKE3 hash. Deterministic, so the same image maps to the same point and
/// a re-index overwrites rather than duplicates; collisions at our scale (millions) are negligible,
/// and orphan cleanup filters by the `document_id` payload regardless, not by id.
pub fn point_id(media_url: &str) -> u64 {
    let hash = blake3::hash(media_url.as_bytes());
    let bytes = hash.as_bytes();
    u64::from_le_bytes(bytes[..8].try_into().expect("blake3 is 32 bytes"))
}

/// The Qdrant client.
#[derive(Clone)]
pub struct Store {
    http: reqwest::Client,
    base: String,
    collection: String,
    api_key: Option<String>,
}

impl Store {
    /// Build a client. Does not connect — the first request does. `api_key` is sent as
    /// `api-key` on every request when present ([[Vector Index]] §10).
    pub fn new(
        url: &str,
        api_key: Option<String>,
        collection: impl Into<String>,
        timeout: std::time::Duration,
    ) -> Result<Self, VectorError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| VectorError::Unreachable(e.to_string()))?;
        Ok(Self {
            http,
            base: url.trim_end_matches('/').to_string(),
            collection: collection.into(),
            api_key,
        })
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut b = self.http.request(method, format!("{}{}", self.base, path));
        if let Some(key) = &self.api_key {
            b = b.header("api-key", key);
        }
        b
    }

    /// Create the collection if it does not exist, then ensure its payload indexes. Idempotent, so
    /// it is safe to call at every startup — a missing collection is re-created and embeddings are
    /// re-derivable from stored media ([[Vector Index]] §7).
    pub async fn ensure_collection(&self) -> Result<(), VectorError> {
        if self.collection_exists().await? {
            self.ensure_payload_indexes().await?;
            return Ok(());
        }
        // int8 scalar quantisation with always_ram: 512×int8 = 512 B/vector resident; float32 stays
        // on disk for rescoring the top candidates ([[Vector Index]] §4).
        let body = json!({
            "vectors": { "size": DIM, "distance": "Cosine", "on_disk": true },
            "hnsw_config": { "m": 16, "ef_construct": 128, "full_scan_threshold": 10000, "on_disk": true },
            "optimizers_config": { "default_segment_number": 4, "indexing_threshold": 20000 },
            "quantization_config": { "scalar": { "type": "int8", "quantile": 0.99, "always_ram": true } }
        });
        let resp = self
            .req(
                reqwest::Method::PUT,
                &format!("/collections/{}", self.collection),
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| VectorError::Unreachable(e.to_string()))?;
        self.check(resp).await?;
        self.ensure_payload_indexes().await?;
        Ok(())
    }

    async fn collection_exists(&self) -> Result<bool, VectorError> {
        let resp = self
            .req(
                reqwest::Method::GET,
                &format!("/collections/{}", self.collection),
            )
            .send()
            .await
            .map_err(|e| VectorError::Unreachable(e.to_string()))?;
        Ok(resp.status().is_success())
    }

    /// Payload indexes on the filtered fields — an unindexed payload filter forces a full scan and
    /// blows the latency budget ([[Vector Index]] §4). Creating an index that exists is a no-op.
    async fn ensure_payload_indexes(&self) -> Result<(), VectorError> {
        for (field, schema) in [
            ("source_type", "keyword"),
            ("published_at", "integer"),
            ("is_nsfw", "bool"),
            ("document_id", "keyword"),
            ("phash", "keyword"),
        ] {
            let resp = self
                .req(
                    reqwest::Method::PUT,
                    &format!("/collections/{}/index", self.collection),
                )
                .json(&json!({ "field_name": field, "field_schema": schema }))
                .send()
                .await
                .map_err(|e| VectorError::Unreachable(e.to_string()))?;
            // A 4xx here usually means "already exists", which is fine; only a 5xx is a real error.
            if resp.status().is_server_error() {
                return Err(self.err(resp).await);
            }
        }
        Ok(())
    }

    /// Upsert a batch of points, waiting for the write to be applied.
    pub async fn upsert(&self, points: &[Point]) -> Result<(), VectorError> {
        if points.is_empty() {
            return Ok(());
        }
        let points_json: Vec<Value> = points
            .iter()
            .map(|p| {
                json!({
                    "id": p.id,
                    "vector": p.vector,
                    "payload": p.payload,
                })
            })
            .collect();
        let resp = self
            .req(
                reqwest::Method::PUT,
                &format!("/collections/{}/points?wait=true", self.collection),
            )
            .json(&json!({ "points": points_json }))
            .send()
            .await
            .map_err(|e| VectorError::Unreachable(e.to_string()))?;
        self.check(resp).await
    }

    /// ANN search. `ef` tunes recall vs latency per request (64 default, 128 for large limits).
    /// `threshold` drops weak matches so "no similar images" is a real answer, not a wall of noise.
    pub async fn search(
        &self,
        vector: &[f32],
        limit: usize,
        ef: usize,
        threshold: f32,
        filter: &SearchFilter,
    ) -> Result<Vec<Hit>, VectorError> {
        let mut body = json!({
            "vector": vector,
            "limit": limit,
            "with_payload": true,
            "params": { "hnsw_ef": ef },
            "score_threshold": threshold,
        });
        if let Some(f) = filter.to_qdrant() {
            body["filter"] = f;
        }
        let resp = self
            .req(
                reqwest::Method::POST,
                &format!("/collections/{}/points/search", self.collection),
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| VectorError::Unreachable(e.to_string()))?;
        let value = self.json(resp).await?;
        parse_hits(&value)
    }

    /// Delete every point belonging to a document — the takedown / orphan-reconciliation path. A
    /// document removed from the lexical index must not remain findable by image similarity
    /// ([[Vector Index]] §7, [[Security and Privacy]] §8).
    pub async fn delete_by_document(&self, document_id: &str) -> Result<(), VectorError> {
        let body = json!({
            "filter": { "must": [ { "key": "document_id", "match": { "value": document_id } } ] }
        });
        let resp = self
            .req(
                reqwest::Method::POST,
                &format!("/collections/{}/points/delete?wait=true", self.collection),
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| VectorError::Unreachable(e.to_string()))?;
        self.check(resp).await
    }

    /// Every distinct `document_id` in the collection.
    ///
    /// Pages through the whole collection with the scroll API, pulling only the `document_id`
    /// payload (no vectors), and de-duplicates. This is the enumeration the orphan-reconciliation
    /// job walks — deliberately a full scan, run off the serving path on a cadence, never per query.
    pub async fn all_document_ids(&self, batch: usize) -> Result<Vec<String>, VectorError> {
        let mut ids = std::collections::HashSet::new();
        let mut offset: Option<Value> = None;
        loop {
            let mut body = json!({
                "limit": batch,
                "with_payload": ["document_id"],
                "with_vector": false,
            });
            if let Some(o) = &offset {
                body["offset"] = o.clone();
            }
            let resp = self
                .req(
                    reqwest::Method::POST,
                    &format!("/collections/{}/points/scroll", self.collection),
                )
                .json(&body)
                .send()
                .await
                .map_err(|e| VectorError::Unreachable(e.to_string()))?;
            let value = self.json(resp).await?;
            let points = value["result"]["points"]
                .as_array()
                .ok_or_else(|| VectorError::Decode("missing result.points".into()))?;
            for p in points {
                if let Some(id) = p["payload"]["document_id"].as_str() {
                    ids.insert(id.to_string());
                }
            }
            // `next_page_offset` is null when the scan is exhausted.
            match value["result"].get("next_page_offset") {
                Some(next) if !next.is_null() => offset = Some(next.clone()),
                _ => break,
            }
        }
        Ok(ids.into_iter().collect())
    }

    /// Total points in the collection — for metrics and tests.
    pub async fn count(&self) -> Result<u64, VectorError> {
        let resp = self
            .req(
                reqwest::Method::POST,
                &format!("/collections/{}/points/count", self.collection),
            )
            .json(&json!({ "exact": true }))
            .send()
            .await
            .map_err(|e| VectorError::Unreachable(e.to_string()))?;
        let value = self.json(resp).await?;
        value["result"]["count"]
            .as_u64()
            .ok_or_else(|| VectorError::Decode("missing result.count".into()))
    }

    async fn check(&self, resp: reqwest::Response) -> Result<(), VectorError> {
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(self.err(resp).await)
        }
    }

    async fn json(&self, resp: reqwest::Response) -> Result<Value, VectorError> {
        if !resp.status().is_success() {
            return Err(self.err(resp).await);
        }
        resp.json::<Value>()
            .await
            .map_err(|e| VectorError::Decode(e.to_string()))
    }

    async fn err(&self, resp: reqwest::Response) -> VectorError {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        VectorError::Backend { status, body }
    }
}

fn parse_hits(value: &Value) -> Result<Vec<Hit>, VectorError> {
    let result = value["result"]
        .as_array()
        .ok_or_else(|| VectorError::Decode("missing result array".into()))?;
    let mut hits = Vec::with_capacity(result.len());
    for item in result {
        let id = item["id"].as_u64().unwrap_or(0);
        let score = item["score"].as_f64().unwrap_or(0.0) as f32;
        let payload: Payload = serde_json::from_value(item["payload"].clone())
            .map_err(|e| VectorError::Decode(e.to_string()))?;
        hits.push(Hit { id, score, payload });
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_id_is_stable_and_url_specific() {
        assert_eq!(
            point_id("https://a.dz/x.jpg"),
            point_id("https://a.dz/x.jpg")
        );
        assert_ne!(
            point_id("https://a.dz/x.jpg"),
            point_id("https://a.dz/y.jpg")
        );
    }

    #[test]
    fn safe_filter_excludes_nsfw() {
        let f = SearchFilter::safe();
        let q = f.to_qdrant().expect("safe filter is non-empty");
        assert_eq!(q["must"][0]["key"], "is_nsfw");
        assert_eq!(q["must"][0]["match"]["value"], false);
    }

    #[test]
    fn empty_filter_is_none() {
        let f = SearchFilter::default();
        assert!(f.to_qdrant().is_none(), "no constraints → no filter clause");
    }

    #[test]
    fn filter_combines_source_and_date() {
        let f = SearchFilter {
            exclude_nsfw: true,
            source_type: Some("web".into()),
            published_after: Some(1_700_000_000),
        };
        let q = f.to_qdrant().unwrap();
        let must = q["must"].as_array().unwrap();
        assert_eq!(must.len(), 3);
    }

    #[test]
    fn hits_parse_from_a_qdrant_response() {
        let value = json!({
            "result": [
                { "id": 42, "score": 0.91, "payload": {
                    "document_id": "doc1", "media_url": "https://a.dz/x.jpg", "is_nsfw": false } },
                { "id": 7, "score": 0.80, "payload": {
                    "document_id": "doc2", "media_url": "https://a.dz/y.jpg", "is_nsfw": false } }
            ]
        });
        let hits = parse_hits(&value).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, 42);
        assert!((hits[0].score - 0.91).abs() < 1e-6);
        assert_eq!(hits[0].payload.document_id, "doc1");
    }

    #[test]
    fn payload_round_trips_through_json() {
        let p = Payload {
            document_id: "d".into(),
            media_url: "u".into(),
            source_type: Some("web".into()),
            published_at: Some(123),
            is_nsfw: false,
            phash: Some("abcd".into()),
        };
        let v = serde_json::to_value(&p).unwrap();
        let back: Payload = serde_json::from_value(v).unwrap();
        assert_eq!(p, back);
    }
}
