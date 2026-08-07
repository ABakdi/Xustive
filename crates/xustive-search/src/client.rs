//! Thin Meilisearch client.
//!
//! Deliberately not a general-purpose SDK: it exposes exactly the operations Xustive performs,
//! with our timeout and error-classification policy baked in.

use std::time::Duration;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use xustive_core::{Classify, ErrorClass};

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("search backend unreachable: {0}")]
    Unreachable(String),
    #[error("search backend timed out after {0:?}")]
    Timeout(Duration),
    #[error("search backend returned {status}: {message}")]
    Backend {
        status: u16,
        message: String,
        code: String,
    },
    #[error("malformed response from search backend: {0}")]
    Malformed(String),
    #[error("indexing task {uid} failed: {message}")]
    TaskFailed { uid: u64, message: String },
    #[error("invalid configuration: {0}")]
    Config(String),
}

impl Classify for SearchError {
    fn class(&self) -> ErrorClass {
        match self {
            // Worth retrying: the backend may just be restarting.
            Self::Unreachable(_) | Self::Timeout(_) => ErrorClass::Transient,
            Self::Backend { status, .. } => xustive_core::error::class_for_status(*status),
            // A response we cannot parse will not parse on retry either.
            Self::Malformed(_) => ErrorClass::Poison,
            Self::TaskFailed { .. } => ErrorClass::Permanent,
            Self::Config(_) => ErrorClass::Fatal,
        }
    }
}

/// A Meilisearch search request. Only the fields we actually use.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Query {
    pub q: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub facets: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sort: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attributes_to_highlight: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight_pre_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight_post_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes_to_crop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop_length: Option<usize>,
}

impl Query {
    pub fn new(q: impl Into<String>) -> Self {
        Self {
            q: q.into(),
            // `<em>` is the only markup the client is allowed to render unescaped.
            highlight_pre_tag: Some("<em>".into()),
            highlight_post_tag: Some("</em>".into()),
            ..Default::default()
        }
    }

    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    pub fn offset(mut self, n: usize) -> Self {
        self.offset = Some(n);
        self
    }

    pub fn filter(mut self, f: impl Into<String>) -> Self {
        let f = f.into();
        if !f.is_empty() {
            self.filter = Some(f);
        }
        self
    }

    pub fn facets(mut self, f: &[&str]) -> Self {
        self.facets = f.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn highlight(mut self, attrs: &[&str]) -> Self {
        self.attributes_to_highlight = attrs.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn sort(mut self, s: &[&str]) -> Self {
        self.sort = s.iter().map(|x| x.to_string()).collect();
        self
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hits<T> {
    #[serde(default = "Vec::new")]
    pub hits: Vec<T>,
    #[serde(default)]
    pub estimated_total_hits: usize,
    #[serde(default)]
    pub processing_time_ms: u64,
    #[serde(default)]
    pub facet_distribution: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRef {
    pub task_uid: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub uid: u64,
    pub status: String,
    #[serde(default)]
    pub error: Option<Value>,
}

impl TaskStatus {
    pub fn is_done(&self) -> bool {
        matches!(self.status.as_str(), "succeeded" | "failed" | "canceled")
    }
    pub fn is_success(&self) -> bool {
        self.status == "succeeded"
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    #[serde(default)]
    pub number_of_documents: u64,
    #[serde(default)]
    pub is_indexing: bool,
}

#[derive(Debug, Deserialize)]
struct MeiliError {
    #[serde(default)]
    message: String,
    #[serde(default)]
    code: String,
}

/// A key we intend to exist.
#[derive(Debug, Clone)]
pub struct KeySpec {
    pub name: &'static str,
    pub description: &'static str,
    /// Meilisearch action patterns, e.g. `search`, `documents.add`.
    pub actions: &'static [&'static str],
    /// Index patterns the key applies to. `*` is every index.
    pub indexes: &'static [&'static str],
}

/// The two keys the running system needs.
///
/// Deliberately not one key with both action sets. The whole value here is that the process
/// serving public traffic physically cannot write, so a bug or a leak in the API cannot corrupt
/// or delete the corpus.
pub const SEARCH_KEY: KeySpec = KeySpec {
    name: "xustive-search",
    description: "Read-only. Held by xustive-api.",
    // `search` alone. Not `indexes.get`, not `stats.get` — the API never needs them, and every
    // extra action is one more thing a leaked key can do.
    actions: &["search"],
    indexes: &["*"],
};

pub const INDEX_KEY: KeySpec = KeySpec {
    name: "xustive-index",
    description: "Write-only. Held by xustive-worker and the crawler.",
    // No `indexes.delete`: a worker never needs to drop an index, and the one operation that
    // loses every document should require the master key and a person.
    actions: &[
        "documents.add",
        "documents.get",
        "documents.delete",
        "tasks.get",
        "indexes.get",
    ],
    indexes: &["*"],
};

/// A key as Meilisearch reports it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ScopedKey {
    pub name: Option<String>,
    pub description: Option<String>,
    /// The credential itself. Printed once at issue and never logged.
    pub key: String,
    pub uid: String,
    pub actions: Vec<String>,
    pub indexes: Vec<String>,
}

#[derive(Clone)]
pub struct MeiliClient {
    http: reqwest::Client,
    base: Url,
    key: String,
    timeout: Duration,
}

impl MeiliClient {
    pub fn new(base_url: &str, key: &str, timeout: Duration) -> Result<Self, SearchError> {
        let base = Url::parse(base_url)
            .map_err(|e| SearchError::Config(format!("bad meili_url {base_url:?}: {e}")))?;
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| SearchError::Config(e.to_string()))?;
        Ok(Self {
            http,
            base,
            key: key.to_string(),
            timeout,
        })
    }

    fn url(&self, path: &str) -> Result<Url, SearchError> {
        self.base
            .join(path)
            .map_err(|e| SearchError::Config(format!("bad path {path}: {e}")))
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.key.is_empty() {
            rb
        } else {
            rb.bearer_auth(&self.key)
        }
    }

    async fn send<T: DeserializeOwned>(
        &self,
        rb: reqwest::RequestBuilder,
    ) -> Result<T, SearchError> {
        let resp = self.auth(rb).send().await.map_err(|e| {
            if e.is_timeout() {
                SearchError::Timeout(self.timeout)
            } else {
                SearchError::Unreachable(e.to_string())
            }
        })?;

        let status = resp.status().as_u16();
        let body = resp
            .text()
            .await
            .map_err(|e| SearchError::Malformed(format!("cannot read body: {e}")))?;

        if !(200..300).contains(&status) {
            let (message, code) = serde_json::from_str::<MeiliError>(&body)
                .map(|e| (e.message, e.code))
                .unwrap_or_else(|_| (body.clone(), String::new()));
            return Err(SearchError::Backend {
                status,
                message,
                code,
            });
        }

        serde_json::from_str(&body)
            .map_err(|e| SearchError::Malformed(format!("{e}; body was: {}", truncate(&body, 400))))
    }

    /// Liveness of the backend. Used by `/readyz`.
    pub async fn health(&self) -> Result<bool, SearchError> {
        #[derive(Deserialize)]
        struct Health {
            status: String,
        }
        let url = self.url("/health")?;
        let h: Health = self.send(self.http.get(url)).await?;
        Ok(h.status == "available")
    }

    /// Run one search.
    pub async fn search<T: DeserializeOwned>(
        &self,
        index: &str,
        query: &Query,
    ) -> Result<Hits<T>, SearchError> {
        let url = self.url(&format!("/indexes/{index}/search"))?;
        self.send(self.http.post(url).json(query)).await
    }

    /// Whether an index exists.
    pub async fn index_exists(&self, index: &str) -> Result<bool, SearchError> {
        let url = self.url(&format!("/indexes/{index}"))?;
        match self.send::<Value>(self.http.get(url)).await {
            Ok(_) => Ok(true),
            Err(SearchError::Backend { status: 404, .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Create an index if it does not exist. Idempotent.
    ///
    /// Existence is checked first rather than creating and tolerating the error, because
    /// Meilisearch reports "already exists" through a **failed task** rather than an immediate
    /// HTTP error. Catching only the immediate form made a second `migrate` run abort, which is
    /// exactly the case an idempotent migration has to survive.
    pub async fn ensure_index(&self, index: &str, primary_key: &str) -> Result<(), SearchError> {
        if self.index_exists(index).await? {
            return Ok(());
        }
        let url = self.url("/indexes")?;
        let body = serde_json::json!({ "uid": index, "primaryKey": primary_key });
        match self.send::<TaskRef>(self.http.post(url).json(&body)).await {
            Ok(t) => match self.wait_task(t.task_uid).await {
                Ok(_) => Ok(()),
                // Lost a race with another migrate: the desired end state still holds.
                Err(SearchError::TaskFailed { ref message, .. })
                    if message.contains("already exists") =>
                {
                    Ok(())
                }
                Err(e) => Err(e),
            },
            Err(SearchError::Backend { ref code, .. }) if code == "index_already_exists" => Ok(()),
            Err(e) => Err(e),
        }
    }

    // --- aliases -----------------------------------------------------------------------
    //
    // Everything reads and writes through an alias, never a concrete index name. That costs
    // nothing today and is the only thing that makes a live reindex possible later: build
    // `documents_v2` alongside `documents_v1`, verify it, then flip the alias in one atomic
    // swap. Rollback is the same operation in reverse.
    //
    // Retrofitting this after the fact means a migration with downtime, which for a search
    // engine is the one kind of migration you cannot schedule quietly.

    /// Meilisearch has no alias primitive, so the alias is a naming convention: `documents` is
    /// whichever `documents_vN` has the highest N. Deriving it from the index list keeps the
    /// mapping in one place rather than in a pointer document that can disagree with reality.
    pub async fn versions_of(&self, alias: &str) -> Result<Vec<(u32, String)>, SearchError> {
        // `/indexes` returns `results`, not the `hits` that search returns. Deserialising it
        // into the search shape yields an empty list rather than an error, so every alias
        // silently resolved to itself — which is a working system serving the wrong index.
        #[derive(serde::Deserialize)]
        struct IndexList {
            results: Vec<Value>,
        }

        // The default page size is 20 and there is no reason to assume an installation has
        // fewer indexes than that.
        let url = self.url("/indexes?limit=1000")?;
        let list: IndexList = self.send(self.http.get(url)).await?;
        let prefix = format!("{alias}_v");
        let mut out: Vec<(u32, String)> = list
            .results
            .iter()
            .filter_map(|e| e.get("uid")?.as_str())
            .filter_map(|uid| {
                let n = uid.strip_prefix(&prefix)?.parse::<u32>().ok()?;
                Some((n, uid.to_string()))
            })
            .collect();
        out.sort_unstable();
        Ok(out)
    }

    /// Resolve an alias to the concrete index that should be read and written.
    ///
    /// A **plain index named exactly the alias wins over any versioned one.** That ordering is
    /// deliberate and is the only thing making this change safe to deploy: this project indexed
    /// into `documents` directly before aliases existed, and preferring `documents_v1` would
    /// point a running system at an empty index the moment a migration created one. An empty
    /// index does not look like a failed migration — it looks like a search engine that has
    /// forgotten everything.
    pub async fn resolve(&self, alias: &str) -> Result<String, SearchError> {
        if self.index_exists(alias).await? {
            return Ok(alias.to_string());
        }
        Ok(self
            .versions_of(alias)
            .await?
            .pop()
            .map(|(_, uid)| uid)
            .unwrap_or_else(|| alias.to_string()))
    }

    // --- scoped keys -------------------------------------------------------------------
    //
    // Meilisearch's master key can do anything, including deleting every index. `xustive-api`
    // only ever searches and `xustive-worker` only ever writes, so neither needs it. Issuing
    // narrow keys means a leaked API credential cannot destroy the corpus — and the master key
    // then exists in exactly one place, the migration job.
    //
    // Provisioning is idempotent and keyed by `name`, so running it twice returns the same key
    // rather than accumulating credentials nobody can account for.

    /// Create or fetch a scoped key.
    pub async fn ensure_key(&self, spec: &KeySpec) -> Result<ScopedKey, SearchError> {
        if let Some(existing) = self.find_key(spec.name).await? {
            return Ok(existing);
        }
        let url = self.url("/keys")?;
        let body = serde_json::json!({
            "name": spec.name,
            "description": spec.description,
            "actions": spec.actions,
            "indexes": spec.indexes,
            // No expiry. Rotation is a deliberate operator action (Security and Privacy §7), and
            // a key that silently expires takes search down at an hour nobody chose.
            "expiresAt": Value::Null,
        });
        self.send(self.http.post(url).json(&body)).await
    }

    pub async fn find_key(&self, name: &str) -> Result<Option<ScopedKey>, SearchError> {
        #[derive(serde::Deserialize)]
        struct KeyList {
            results: Vec<ScopedKey>,
        }
        let url = self.url("/keys?limit=1000")?;
        let list: KeyList = self.send(self.http.get(url)).await?;
        Ok(list
            .results
            .into_iter()
            .find(|k| k.name.as_deref() == Some(name)))
    }

    /// Drop an index and everything in it.
    ///
    /// Used by the alias tests and, later, by reindex cleanup once a flip has been verified.
    /// Deliberately not wired to any command: there is no operator path to this that does not
    /// go through a deliberate code change.
    pub async fn delete_index(&self, index: &str) -> Result<(), SearchError> {
        let url = self.url(&format!("/indexes/{index}"))?;
        match self.send::<TaskRef>(self.http.delete(url)).await {
            Ok(t) => {
                self.wait_task(t.task_uid).await?;
                Ok(())
            }
            Err(SearchError::Backend { status: 404, .. }) => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub async fn apply_settings(&self, index: &str, settings: &Value) -> Result<(), SearchError> {
        let url = self.url(&format!("/indexes/{index}/settings"))?;
        let t: TaskRef = self.send(self.http.patch(url).json(settings)).await?;
        self.wait_task(t.task_uid).await?;
        Ok(())
    }

    pub async fn get_settings(&self, index: &str) -> Result<Value, SearchError> {
        let url = self.url(&format!("/indexes/{index}/settings"))?;
        self.send(self.http.get(url)).await
    }

    /// Upsert documents. Meilisearch keys on the primary key, so this is idempotent — which is
    /// what makes at-least-once queue delivery safe.
    pub async fn add_documents<T: Serialize>(
        &self,
        index: &str,
        docs: &[T],
    ) -> Result<u64, SearchError> {
        let url = self.url(&format!("/indexes/{index}/documents"))?;
        let t: TaskRef = self.send(self.http.post(url).json(docs)).await?;
        Ok(t.task_uid)
    }

    pub async fn delete_document(&self, index: &str, id: &str) -> Result<u64, SearchError> {
        let url = self.url(&format!("/indexes/{index}/documents/{id}"))?;
        let t: TaskRef = self.send(self.http.delete(url)).await?;
        Ok(t.task_uid)
    }

    pub async fn task(&self, uid: u64) -> Result<TaskStatus, SearchError> {
        let url = self.url(&format!("/tasks/{uid}"))?;
        self.send(self.http.get(url)).await
    }

    /// Poll a task to completion with capped exponential backoff.
    ///
    /// The indexer acknowledges its queue message only after this returns successfully, so a
    /// crash here costs re-work rather than data.
    pub async fn wait_task(&self, uid: u64) -> Result<TaskStatus, SearchError> {
        let mut delay = Duration::from_millis(50);
        let deadline = std::time::Instant::now() + Duration::from_secs(60);

        loop {
            let t = self.task(uid).await?;
            if t.is_done() {
                if !t.is_success() {
                    let message = t
                        .error
                        .as_ref()
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    return Err(SearchError::TaskFailed { uid, message });
                }
                return Ok(t);
            }
            if std::time::Instant::now() >= deadline {
                return Err(SearchError::Timeout(Duration::from_secs(60)));
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(1));
        }
    }

    pub async fn stats(&self, index: &str) -> Result<IndexStats, SearchError> {
        let url = self.url(&format!("/indexes/{index}/stats"))?;
        self.send(self.http.get(url)).await
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_serializes_only_set_fields() {
        let q = Query::new("سونلغاز").limit(20).offset(40);
        let v = serde_json::to_value(&q).unwrap();
        assert_eq!(v["q"], "سونلغاز");
        assert_eq!(v["limit"], 20);
        assert_eq!(v["offset"], 40);
        // Unset optionals must be absent, not null — Meilisearch rejects some nulls.
        assert!(v.get("filter").is_none());
        assert!(v.get("sort").is_none());
        assert!(v.get("facets").is_none());
    }

    #[test]
    fn query_uses_camel_case_for_meili() {
        let q = Query::new("x").highlight(&["excerpt"]);
        let v = serde_json::to_value(&q).unwrap();
        assert_eq!(v["attributesToHighlight"][0], "excerpt");
        assert_eq!(v["highlightPreTag"], "<em>");
        assert_eq!(v["highlightPostTag"], "</em>");
    }

    #[test]
    fn empty_filter_is_omitted() {
        let q = Query::new("x").filter("");
        assert!(serde_json::to_value(&q).unwrap().get("filter").is_none());
    }

    #[test]
    fn error_classification_drives_retry_policy() {
        let unreachable = SearchError::Unreachable("refused".into());
        assert_eq!(unreachable.class(), ErrorClass::Transient);
        assert!(unreachable.is_retryable());

        let bad_request = SearchError::Backend {
            status: 400,
            message: "bad filter".into(),
            code: "invalid_search_filter".into(),
        };
        assert_eq!(bad_request.class(), ErrorClass::Permanent);
        assert!(
            !bad_request.is_retryable(),
            "a bad filter will not fix itself"
        );

        let malformed = SearchError::Malformed("bad json".into());
        assert!(malformed.class().is_dead_letter());

        assert!(SearchError::Config("x".into()).class().is_fatal());
    }

    #[test]
    fn server_errors_are_retryable() {
        let e = SearchError::Backend {
            status: 503,
            message: String::new(),
            code: String::new(),
        };
        assert!(e.is_retryable());
    }

    #[test]
    fn task_status_transitions() {
        let mk = |s: &str| TaskStatus {
            uid: 1,
            status: s.into(),
            error: None,
        };
        assert!(!mk("enqueued").is_done());
        assert!(!mk("processing").is_done());
        assert!(mk("succeeded").is_done() && mk("succeeded").is_success());
        assert!(mk("failed").is_done() && !mk("failed").is_success());
        assert!(mk("canceled").is_done() && !mk("canceled").is_success());
    }

    #[test]
    fn hits_deserialize_with_missing_optional_fields() {
        let h: Hits<Value> = serde_json::from_str(r#"{"hits":[]}"#).unwrap();
        assert_eq!(h.hits.len(), 0);
        assert_eq!(h.estimated_total_hits, 0);
    }

    #[test]
    fn client_rejects_a_bad_base_url() {
        let Err(e) = MeiliClient::new("not a url", "", Duration::from_secs(1)) else {
            panic!("a malformed base url should be rejected at construction");
        };
        assert!(
            e.class().is_fatal(),
            "bad config must be fatal, not retried"
        );
    }

    #[test]
    fn truncate_is_char_safe() {
        // Multi-byte characters must not be split mid-codepoint.
        assert_eq!(truncate("الجزائر", 3), "الج…");
        assert!(truncate("الجزائر", 3).is_char_boundary(truncate("الجزائر", 3).len()));
        assert_eq!(truncate("ab", 5), "ab");
        assert_eq!(truncate("", 5), "");
    }
}
