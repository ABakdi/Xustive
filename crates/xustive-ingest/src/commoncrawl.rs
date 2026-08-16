//! Common Crawl bootstrap (M2-T16.1–.3).
//!
//! Common Crawl has already fetched a quarter-trillion pages and publishes an index of them. Reading
//! that index for Algerian hosts is the cheapest discovery there is: it costs the sites *nothing* —
//! someone else already paid the bandwidth — and returns URLs by the million where a SERP query
//! returns ten. It is the volume floor of the discovery ladder ([[ADR-0013]]).
//!
//! What we take from it is only the **list of URLs**, entered into the ordinary frontier at a
//! discovered-tier trust and fetched under the ordinary rules — robots, politeness, `SafeUrl`,
//! dedup. Common Crawl's copy of a page is not served to anyone; it is a pointer to a URL worth
//! fetching ourselves.
//!
//! # Two filters, in the right places
//!
//! - **Host/domain** (T16.2) happens here, against the index: `.dz`, plus the known Algerian hosts
//!   that live on generic TLDs (`elkhabar.com`, `ouedkniss.com`, …). The index is three orders of
//!   magnitude larger than what we want, so filtering at the source is the whole point.
//! - **Language** (ar/fr) happens *downstream* at crawl time, where the language detector already
//!   runs — except when the index itself carries a `languages` field, in which case an obviously
//!   off-topic capture is dropped here too, before it costs a fetch.
//!
//! # Resumable (T16.3)
//!
//! The CDX index is paginated and a domain scan is a long job over a remote server that will be
//! interrupted. Progress is a single number per `(snapshot, pattern)` — the last page finished — so
//! a restart continues rather than re-ingesting. A monthly release is therefore ingested once.

use std::collections::HashSet;

use serde::Deserialize;

/// One capture from the CDX index — only the fields discovery needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdxRecord {
    pub url: String,
    pub status: u16,
    pub mime: String,
    /// ISO-639-3 codes, when the index carries them (newer snapshots do). Empty otherwise.
    pub languages: Vec<String>,
}

/// The raw JSON shape of a CDX line. Every field arrives as a string, including the numbers.
#[derive(Debug, Deserialize)]
struct CdxJson {
    url: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    mime: String,
    #[serde(default)]
    languages: String,
}

/// Parse one CDX JSON line. `None` for a blank line or malformed JSON — a corrupt line in a
/// million-line page is skipped, not fatal, because the remote index is not ours to trust byte for
/// byte and one bad row must not abort a snapshot.
pub fn parse_cdx_line(line: &str) -> Option<CdxRecord> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let raw: CdxJson = serde_json::from_str(line).ok()?;
    let languages = raw
        .languages
        .split([',', ' '])
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect();
    Some(CdxRecord {
        url: raw.url,
        status: raw.status.parse().unwrap_or(0),
        mime: raw.mime,
        languages,
    })
}

/// The languages we keep when the index tells us: Arabic, French, English (ISO-639-3). A capture
/// with no language recorded is *kept* — the crawl-time detector decides, since absence of data is
/// not evidence of the wrong language.
const KEEP_LANGS: [&str; 3] = ["ara", "fra", "eng"];

impl CdxRecord {
    /// Whether this capture is worth turning into a fetch: a `200` of HTML. A redirect, a 404, or a
    /// PDF from the index is not a discovery worth queueing — the redirect target, if it matters,
    /// is its own capture.
    pub fn is_fetch_candidate(&self) -> bool {
        self.status == 200 && (self.mime.contains("html") || self.mime.is_empty())
    }

    /// Whether the index's own language tags disqualify this capture. Kept when no language is
    /// recorded, or when any recorded language is one we want.
    pub fn language_allows(&self) -> bool {
        self.languages.is_empty()
            || self
                .languages
                .iter()
                .any(|l| KEEP_LANGS.contains(&l.as_str()))
    }
}

/// The Algeria host filter (T16.2): a `.dz` capture, or one on a known Algerian host that lives on a
/// generic TLD. Built from the registry, so "which `.com` hosts are Algerian" is the same curated
/// answer the crawler already uses rather than a second list to keep in step.
#[derive(Debug, Clone, Default)]
pub struct AlgeriaFilter {
    /// Registrable domains, lowercased, `www.` stripped — the non-`.dz` hosts we recognise.
    known_hosts: HashSet<String>,
}

impl AlgeriaFilter {
    /// Build from an iterator of known Algerian hosts (typically registry entry-point domains).
    pub fn new(hosts: impl IntoIterator<Item = String>) -> Self {
        Self {
            known_hosts: hosts.into_iter().map(|h| normalise_host(&h)).collect(),
        }
    }

    /// Whether a URL is one we want: on the `.dz` TLD, or on a known Algerian host.
    pub fn accepts_url(&self, url: &str) -> bool {
        let Some(host) = host_of(url) else {
            return false;
        };
        self.accepts_host(&host)
    }

    fn accepts_host(&self, host: &str) -> bool {
        let host = normalise_host(host);
        if host == "dz" || host.ends_with(".dz") {
            return true;
        }
        // Match the host or any parent domain against the known set, so `sub.elkhabar.com` is kept
        // when `elkhabar.com` is known.
        if self.known_hosts.contains(&host) {
            return true;
        }
        let mut rest = host.as_str();
        while let Some((_, parent)) = rest.split_once('.') {
            if self.known_hosts.contains(parent) {
                return true;
            }
            rest = parent;
        }
        false
    }

    /// The full per-record decision: a fetchable HTML capture, of an allowed language, on an
    /// Algerian host. This is the single predicate the ingester applies to each CDX row.
    pub fn keep(&self, rec: &CdxRecord) -> bool {
        rec.is_fetch_candidate() && rec.language_allows() && self.accepts_url(&rec.url)
    }
}

/// Turn one CDX page (JSON-Lines) into the distinct URLs worth queueing: parse each row, keep the
/// ones the filter accepts, and de-duplicate within the page. Pure, so the filtering that decides
/// what a snapshot contributes is testable without touching the network.
pub fn select_urls(page: &str, filter: &AlgeriaFilter) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in page.lines() {
        let Some(rec) = parse_cdx_line(line) else {
            continue;
        };
        if filter.keep(&rec) && seen.insert(rec.url.clone()) {
            out.push(rec.url);
        }
    }
    out
}

/// Why a CDX request failed.
#[derive(Debug, thiserror::Error)]
pub enum CcError {
    #[error("cdx request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("cdx index {status} for {pattern}")]
    Status { status: u16, pattern: String },
}

/// A client for one Common Crawl snapshot's CDX index.
///
/// The CDX server is a fixed, trusted host, so a plain client is fine here — unlike the crawl
/// fetcher, whose whole job is talking to untrusted sites. The URLs this returns are still routed
/// through the frontier and `SafeUrl` before anything fetches them.
#[derive(Clone)]
pub struct CdxClient {
    http: reqwest::Client,
    /// The snapshot id, e.g. `CC-MAIN-2026-05`.
    index: String,
    /// The index server base, overridable so a test can point at a local stub.
    base: String,
}

impl CdxClient {
    /// `index` is a snapshot id like `CC-MAIN-2026-05`.
    pub fn new(index: &str) -> Result<Self, reqwest::Error> {
        Ok(Self {
            http: reqwest::Client::builder()
                .user_agent(crate::robots::USER_AGENT)
                .timeout(std::time::Duration::from_secs(60))
                .build()?,
            index: index.to_string(),
            base: "https://index.commoncrawl.org".to_string(),
        })
    }

    /// Point at a different index server (a test stub, or a mirror).
    pub fn with_base(mut self, base: &str) -> Self {
        self.base = base.trim_end_matches('/').to_string();
        self
    }

    fn index_url(&self) -> String {
        format!("{}/{}-index", self.base, self.index)
    }

    /// How many pages the index will return for `pattern`. `None` if the server does not say, in
    /// which case the caller walks pages until one comes back empty.
    pub async fn num_pages(&self, pattern: &str) -> Result<Option<usize>, CcError> {
        let resp = self
            .http
            .get(self.index_url())
            .query(&[
                ("url", pattern),
                ("output", "json"),
                ("showNumPages", "true"),
            ])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(CcError::Status {
                status: resp.status().as_u16(),
                pattern: pattern.to_string(),
            });
        }
        let text = resp.text().await?;
        // The reply is a small JSON object: {"pages": N, "pageSize": ..., "blocks": ...}.
        Ok(serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("pages").and_then(|p| p.as_u64()))
            .map(|n| n as usize))
    }

    /// Fetch one page of the index for `pattern`, as raw JSON-Lines. An empty string means the page
    /// held no captures — the signal to stop when the page count is unknown.
    pub async fn fetch_page(&self, pattern: &str, page: usize) -> Result<String, CcError> {
        let resp = self
            .http
            .get(self.index_url())
            .query(&[
                ("url", pattern),
                ("output", "json"),
                ("page", &page.to_string()),
            ])
            .send()
            .await?;
        let status = resp.status();
        if status.as_u16() == 404 {
            // The index returns 404 for a pattern with no captures at all — not an error, just an
            // empty result. A domain we have no coverage for is a normal outcome.
            return Ok(String::new());
        }
        if !status.is_success() {
            return Err(CcError::Status {
                status: status.as_u16(),
                pattern: pattern.to_string(),
            });
        }
        Ok(resp.text().await?)
    }
}

/// Resumable snapshot progress (T16.3): the last page finished for a `(snapshot, pattern)`, so an
/// interrupted domain scan continues instead of re-ingesting. One integer per key in Redis.
#[derive(Clone)]
pub struct CcProgress {
    client: redis::Client,
    namespace: String,
}

impl CcProgress {
    pub fn connect_in(url: &str, namespace: &str) -> Option<Self> {
        Some(Self {
            client: redis::Client::open(url).ok()?,
            namespace: namespace.to_string(),
        })
    }

    async fn conn(&self) -> Option<redis::aio::MultiplexedConnection> {
        self.client.get_multiplexed_async_connection().await.ok()
    }

    fn key(&self, index: &str, pattern: &str) -> String {
        format!("{}:cc:{index}:{pattern}", self.namespace)
    }

    /// The last page fully ingested, or `None` if this scan has never run. The caller resumes at
    /// `last + 1`.
    pub async fn last_page(&self, index: &str, pattern: &str) -> Option<usize> {
        let mut conn = self.conn().await?;
        let raw: Option<usize> = redis::cmd("GET")
            .arg(self.key(index, pattern))
            .query_async(&mut conn)
            .await
            .ok()
            .flatten();
        raw
    }

    /// Record that `page` is fully ingested. Written after the page's URLs are queued, never before
    /// — a crash mid-page then re-does that page, which is safe (the frontier dedups) rather than
    /// skipping it (which would silently lose URLs).
    pub async fn set_last_page(&self, index: &str, pattern: &str, page: usize) {
        let Some(mut conn) = self.conn().await else {
            return;
        };
        let _: Result<(), _> = redis::cmd("SET")
            .arg(self.key(index, pattern))
            .arg(page)
            .query_async::<()>(&mut conn)
            .await;
    }
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
}

fn normalise_host(host: &str) -> String {
    host.trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .split('/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_cdx_line_and_skips_junk() {
        let line = r#"{"urlkey":"dz,aps)/","timestamp":"20260101","url":"https://www.aps.dz/x","mime":"text/html","status":"200","languages":"ara,fra"}"#;
        let r = parse_cdx_line(line).unwrap();
        assert_eq!(r.url, "https://www.aps.dz/x");
        assert_eq!(r.status, 200);
        assert_eq!(r.languages, vec!["ara", "fra"]);
        assert!(parse_cdx_line("").is_none());
        assert!(parse_cdx_line("not json").is_none());
    }

    #[test]
    fn only_html_200s_are_fetch_candidates() {
        let ok = CdxRecord {
            url: "https://x.dz/".into(),
            status: 200,
            mime: "text/html".into(),
            languages: vec![],
        };
        assert!(ok.is_fetch_candidate());
        let redirect = CdxRecord {
            status: 301,
            ..ok.clone()
        };
        assert!(!redirect.is_fetch_candidate());
        let pdf = CdxRecord {
            mime: "application/pdf".into(),
            ..ok.clone()
        };
        assert!(!pdf.is_fetch_candidate());
    }

    #[test]
    fn language_tags_drop_the_obviously_off_topic_but_keep_the_untagged() {
        let untagged = CdxRecord {
            url: "https://x.dz/".into(),
            status: 200,
            mime: "text/html".into(),
            languages: vec![],
        };
        assert!(
            untagged.language_allows(),
            "no tag means the detector decides later"
        );
        let arabic = CdxRecord {
            languages: vec!["ara".into()],
            ..untagged.clone()
        };
        assert!(arabic.language_allows());
        let russian = CdxRecord {
            languages: vec!["rus".into()],
            ..untagged.clone()
        };
        assert!(!russian.language_allows());
    }

    #[test]
    fn the_algeria_filter_accepts_dz_and_known_hosts_only() {
        let f = AlgeriaFilter::new(["elkhabar.com".into(), "ouedkniss.com".into()]);
        assert!(f.accepts_url("https://www.aps.dz/article"));
        assert!(f.accepts_url("https://sub.ministere.gov.dz/x"));
        assert!(
            f.accepts_url("https://www.elkhabar.com/a"),
            "known .com host"
        );
        assert!(
            f.accepts_url("https://news.elkhabar.com/a"),
            "subdomain of a known host"
        );
        assert!(
            !f.accepts_url("https://www.lemonde.fr/a"),
            "unknown foreign host"
        );
        assert!(
            !f.accepts_url("https://notelkhabar.com/a"),
            "no shared-suffix false match"
        );
        assert!(!f.accepts_url("not a url"));
    }

    #[test]
    fn select_urls_filters_and_dedups_a_page() {
        let f = AlgeriaFilter::new(["elkhabar.com".into()]);
        let page = [
            r#"{"url":"https://www.aps.dz/a","status":"200","mime":"text/html","languages":"ara"}"#,
            r#"{"url":"https://www.aps.dz/a","status":"200","mime":"text/html"}"#, // dup
            r#"{"url":"https://www.elkhabar.com/b","status":"200","mime":"text/html"}"#,
            r#"{"url":"https://www.lemonde.fr/c","status":"200","mime":"text/html"}"#, // foreign
            r#"{"url":"https://www.aps.dz/d.pdf","status":"200","mime":"application/pdf"}"#, // not html
            r#"{"url":"https://www.aps.dz/e","status":"301","mime":"text/html"}"#, // redirect
            "garbage line",
            "",
        ]
        .join("\n");
        let urls = select_urls(&page, &f);
        assert_eq!(
            urls,
            vec!["https://www.aps.dz/a", "https://www.elkhabar.com/b"]
        );
    }

    #[test]
    fn keep_combines_all_three_gates() {
        let f = AlgeriaFilter::new(["elkhabar.com".into()]);
        let good = CdxRecord {
            url: "https://www.elkhabar.com/a".into(),
            status: 200,
            mime: "text/html".into(),
            languages: vec!["ara".into()],
        };
        assert!(f.keep(&good));
        // Right host, wrong status.
        assert!(!f.keep(&CdxRecord {
            status: 404,
            ..good.clone()
        }));
        // Right host and status, wrong language.
        assert!(!f.keep(&CdxRecord {
            languages: vec!["zho".into()],
            ..good.clone()
        }));
        // Fetchable and Algerian-language but foreign host.
        assert!(!f.keep(&CdxRecord {
            url: "https://x.fr/".into(),
            ..good.clone()
        }));
    }
}
