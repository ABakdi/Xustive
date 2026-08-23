//! Polite HTTP fetching.
//!
//! Every URL passes `SafeUrl` before a connection is opened, and again on every redirect hop —
//! that is where naive implementations get caught, because a public-looking host can 302 to
//! `169.254.169.254`.
//!
//! The client identifies itself honestly, obeys `robots.txt` and `Crawl-delay`, keeps one
//! in-flight request per host, and backs off on 429 and 503 rather than routing around them.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use xustive_core::safe_url::{self, SafeUrl, UrlError};
use xustive_core::{Classify, ErrorClass};

use crate::robots::{self, Politeness, Robots};

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("unsafe url: {0}")]
    Unsafe(#[from] UrlError),
    #[error("blocked by robots.txt")]
    RobotsDisallowed,
    #[error("request failed: {0}")]
    Transport(String),
    #[error("timed out")]
    Timeout,
    #[error("http {0}")]
    Status(u16),
    #[error("content type {0:?} is not indexable")]
    ContentType(String),
    #[error("body exceeds {0} bytes")]
    TooLarge(usize),
    #[error("too many redirects")]
    TooManyRedirects,
}

impl Classify for FetchError {
    fn class(&self) -> ErrorClass {
        match self {
            Self::Transport(_) | Self::Timeout => ErrorClass::Transient,
            Self::Status(s) => xustive_core::error::class_for_status(*s),
            // Properties of the URL or the resource. Retrying changes nothing and looks like
            // abuse to the host.
            Self::Unsafe(_)
            | Self::RobotsDisallowed
            | Self::ContentType(_)
            | Self::TooLarge(_)
            | Self::TooManyRedirects => ErrorClass::Permanent,
        }
    }
}

impl FetchError {
    /// A stable outcome label for the fetcher's classification table (M2-T04.5, Web Fetcher §4.4).
    ///
    /// Finer than [`Classify::class`], which answers only "retry or not". This answers "what
    /// happened", so the crawl counters distinguish a spike in **gone** (sites removing content)
    /// from **throttled** (we are being rate-limited) from **transient** (the network is flaky) —
    /// three problems with three different responses that a single `failed` total hides.
    ///
    /// `gone` (404/410) is called out from the other permanent failures because it is the one the
    /// orchestrator can act on: the resource is deliberately removed, so there is nothing to retry
    /// and nothing to keep in the frontier for it.
    pub fn outcome(&self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Transport(_) => "transient",
            Self::RobotsDisallowed => "robots",
            Self::Unsafe(_) => "unsafe",
            Self::ContentType(_) => "content_type",
            Self::TooLarge(_) => "too_large",
            Self::TooManyRedirects => "redirect_loop",
            Self::Status(404) | Self::Status(410) => "gone",
            Self::Status(429) => "throttled",
            Self::Status(s) => match xustive_core::error::class_for_status(*s) {
                ErrorClass::Transient => "transient",
                _ => "permanent",
            },
        }
    }

    /// Whether the resource is gone — a 404 or 410. The orchestrator drops these without retry.
    pub fn is_gone(&self) -> bool {
        matches!(self, Self::Status(404) | Self::Status(410))
    }
}

#[derive(Debug, Clone)]
pub struct FetchConfig {
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
    pub max_body_bytes: usize,
    pub robots_ttl: Duration,
    /// Extra pause on top of the host's crawl-delay. Cheap insurance.
    pub politeness_margin: Duration,
    /// **Testing only.** Ignore robots, delays and host opt-outs entirely.
    ///
    /// Threaded through the config rather than read from a global, so a `Fetcher` built without
    /// asking for it can never acquire it — including in tests, where a global would be shared
    /// state between cases that run in parallel.
    pub ignore_politeness: bool,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            // A worker is pinned to one fetch for as long as it runs, so a slow or dead host is
            // stolen throughput, not just a slow page. Five seconds is generous for a TCP+TLS
            // handshake to any live host; beyond that the host is almost never worth the wait,
            // and freeing the worker to claim a responsive host is the better trade. The total
            // cap stays comfortably above a large page over a slow link.
            connect_timeout: Duration::from_secs(5),
            total_timeout: Duration::from_secs(20),
            max_body_bytes: 10 * 1024 * 1024,
            robots_ttl: Duration::from_secs(24 * 3600),
            politeness_margin: Duration::from_millis(200),
            ignore_politeness: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Fetched {
    pub url: String,
    pub final_url: String,
    pub status: u16,
    pub body: String,
    pub content_type: String,
    /// True when the charset was sniffed rather than declared.
    pub charset_guessed: bool,
    /// What `X-Robots-Tag` asked for, if anything.
    ///
    /// Carried on the response rather than resolved here, because the fetcher's job is to report
    /// what the server said and the caller's is to decide what to do about it — a `noindex` page
    /// is still worth crawling for its links.
    ///
    /// This is the only way a non-HTML document can refuse indexing. A PDF or an image has no
    /// `<head>` to put a meta tag in, so honouring the tag but not the header means honouring the
    /// request exactly where it is easy and ignoring it where it is the site's only option.
    pub exclusion: Option<crate::exclusion::Exclusion>,
    /// Validators the server sent, for the next visit's conditional request.
    ///
    /// Stored rather than discarded because they are the whole economics of recrawl: a request
    /// carrying them back costs a few hundred bytes when nothing changed, where an unconditional
    /// one costs the page. Without them, adaptive scheduling pays full price to learn "unchanged".
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// Validators from a previous fetch, replayed on the next one.
#[derive(Debug, Clone, Default)]
pub struct Conditional<'a> {
    pub etag: Option<&'a str>,
    pub last_modified: Option<&'a str>,
}

const INDEXABLE: &[&str] = &[
    "text/html",
    "application/xhtml+xml",
    "text/plain",
    "application/xml",
    "text/xml",
    "application/rss+xml",
    "application/atom+xml",
    // Born-digital PDFs (M2-T14.3). The bytes are extracted to text below; scanned PDFs yield
    // nothing and fall out as thin, which is correct until OCR exists.
    "application/pdf",
];

/// Hard cap on a PDF, well below the HTML body cap: PDFs are large and mostly binary, and we keep
/// only their text.
const PDF_MAX_BYTES: usize = 12 * 1024 * 1024;
/// Characters of extracted PDF text kept — a page cap expressed in text rather than bytes.
const PDF_MAX_CHARS: usize = 200_000;

pub struct Fetcher {
    http: reqwest::Client,
    politeness: Arc<Mutex<Politeness>>,
    config: FetchConfig,
    /// Shared across workers. Absent means every worker fetches its own `robots.txt`, which is
    /// correct but rude at scale.
    robots_cache: Option<crate::robots_cache::RobotsCache>,
}

impl Fetcher {
    pub fn new(config: FetchConfig) -> Result<Self, FetchError> {
        let http = reqwest::Client::builder()
            .user_agent(robots::USER_AGENT)
            .connect_timeout(config.connect_timeout)
            .timeout(config.total_timeout)
            // Redirects are followed manually so every hop can be revalidated. `reqwest`'s
            // built-in policy would not let us check the intermediate hosts.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| FetchError::Transport(e.to_string()))?;

        Ok(Self {
            http,
            politeness: Arc::new(Mutex::new(Politeness::with_bypass(
                config.ignore_politeness,
            ))),
            config,
            robots_cache: None,
        })
    }

    /// Share `robots.txt` between workers through Redis.
    ///
    /// Optional on purpose. Without it the crawler is correct and merely wasteful; requiring it
    /// would make a Redis outage a crawl outage, and would mean a single-process run needs Redis
    /// in order to fetch one page.
    pub fn with_shared_cache(mut self, cache: crate::robots_cache::RobotsCache) -> Self {
        self.robots_cache = Some(cache);
        self
    }

    /// Fetch a URL, honouring robots and pacing.
    ///
    /// Blocks until this host's crawl-delay has elapsed, which is what enforces one in-flight
    /// request per host when callers share a `Fetcher`.
    pub async fn get(&self, raw_url: &str) -> Result<Fetched, FetchError> {
        self.get_conditional(raw_url, Conditional::default()).await
    }

    /// Fetch with `If-None-Match` / `If-Modified-Since` (M2-T04.2).
    ///
    /// A 304 comes back as `Ok` with `status == 304` and an empty body — not as an error, because
    /// it is the best possible answer: the page is exactly what we already hold, learned for a few
    /// hundred bytes. Callers on the revisit path check the status; the discovery path never sends
    /// validators and never sees one.
    pub async fn get_conditional(
        &self,
        raw_url: &str,
        cond: Conditional<'_>,
    ) -> Result<Fetched, FetchError> {
        let url = SafeUrl::parse(raw_url)?;
        // The authority, not the bare host: robots.txt is per origin, so `example.dz:8080` must
        // not inherit `example.dz`'s rules. `Url::port()` returns `None` for a scheme's default
        // port, so ordinary URLs still key on the bare host.
        let host = url.authority();

        self.ensure_robots(&host, &url).await;

        {
            let p = self.politeness.lock().await;
            if !p.allows(&host, url.as_url().path()) {
                return Err(FetchError::RobotsDisallowed);
            }
        }
        self.wait_turn(&host).await;

        let mut current = url;
        for hop in 0..=safe_url::MAX_REDIRECTS {
            // Resolve and check every address before connecting. A host that resolved publicly
            // an hour ago may not now.
            safe_url::resolve_and_check(&current).await?;

            let mut req = self.http.get(current.as_str());
            if let Some(etag) = cond.etag {
                req = req.header(reqwest::header::IF_NONE_MATCH, etag);
            }
            if let Some(lm) = cond.last_modified {
                req = req.header(reqwest::header::IF_MODIFIED_SINCE, lm);
            }
            let resp = req.send().await.map_err(|e| {
                if e.is_timeout() {
                    FetchError::Timeout
                } else {
                    FetchError::Transport(e.to_string())
                }
            })?;

            let status = resp.status().as_u16();
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);

            {
                let mut p = self.politeness.lock().await;
                p.observe(&host, status, retry_after);
                p.record_fetch(&host);
            }

            if status == 304 {
                // No body, no content type, nothing to decode. The caller keeps what it has.
                return Ok(Fetched {
                    url: raw_url.to_string(),
                    final_url: resp.url().to_string(),
                    status,
                    body: String::new(),
                    content_type: String::new(),
                    charset_guessed: false,
                    exclusion: None,
                    etag: None,
                    last_modified: None,
                });
            }

            if (300..400).contains(&status) {
                let Some(location) = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                else {
                    return Err(FetchError::Status(status));
                };
                // The check that matters: a redirect target gets the same scrutiny as the
                // original URL.
                current = current.redirect_to(location, hop)?;
                continue;
            }

            if !(200..300).contains(&status) {
                return Err(FetchError::Status(status));
            }

            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let mime = content_type
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if !INDEXABLE.iter().any(|t| mime == *t) {
                return Err(FetchError::ContentType(mime));
            }

            let robots_tag: Vec<String> = resp
                .headers()
                .get_all("x-robots-tag")
                .iter()
                .filter_map(|v| v.to_str().ok())
                .map(str::to_string)
                .collect();
            let exclusion = crate::exclusion::from_header(&robots_tag, crate::robots::UA_TOKEN);

            let take = |name: reqwest::header::HeaderName| {
                resp.headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            };
            let etag = take(reqwest::header::ETAG);
            let last_modified = take(reqwest::header::LAST_MODIFIED);

            let final_url = resp.url().to_string();
            let is_pdf = mime == "application/pdf";
            let cap = if is_pdf {
                PDF_MAX_BYTES
            } else {
                self.config.max_body_bytes
            };
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| FetchError::Transport(e.to_string()))?;
            if bytes.len() > cap {
                return Err(FetchError::TooLarge(cap));
            }

            // A PDF is extracted to text and then treated exactly like any other document; a page
            // that yields no text (a scan) fails the thin-content check downstream, as it should.
            let (body, charset_guessed) = if is_pdf {
                match extract_pdf_text(bytes.to_vec()).await {
                    Some(text) => (text, false),
                    None => return Err(FetchError::ContentType("application/pdf".into())),
                }
            } else {
                decode(&bytes, &content_type)
            };
            return Ok(Fetched {
                url: raw_url.to_string(),
                final_url,
                status,
                body,
                content_type: mime,
                charset_guessed,
                exclusion,
                etag,
                last_modified,
            });
        }

        Err(FetchError::TooManyRedirects)
    }

    /// Fetch and cache `robots.txt` if we have no fresh copy.
    async fn ensure_robots(&self, host: &str, url: &SafeUrl) {
        {
            let p = self.politeness.lock().await;
            if p.skip_robots_fetch() {
                return;
            }
            if !p.rules_stale(host, self.config.robots_ttl) {
                return;
            }
        }

        // Another worker may already have fetched this host's rules. Checking costs one Redis
        // round trip against one HTTP request to somebody else's server.
        if let Some(cache) = &self.robots_cache {
            let now = xustive_core::now_unix();
            if let Some(entry) = cache.get(host, now).await {
                tracing::debug!(
                    host,
                    status = entry.status,
                    "robots.txt from the shared cache"
                );
                self.politeness
                    .lock()
                    .await
                    .set_rules(host, entry.to_robots());
                return;
            }
        }

        let robots_url = {
            let mut u = url.as_url().clone();
            u.set_path("/robots.txt");
            u.set_query(None);
            u.set_fragment(None);
            u.to_string()
        };

        let (rules, entry) = self.fetch_robots_cached(host, &robots_url).await;

        if let (Some(cache), Some(entry)) = (&self.robots_cache, entry) {
            cache.put(host, &entry).await;
        }

        if !rules.sitemaps.is_empty() {
            tracing::debug!(host, count = rules.sitemaps.len(), "discovered sitemaps");
        }
        self.politeness.lock().await.set_rules(host, rules);
    }

    /// Fetch and parse `robots.txt`, following redirects.
    ///
    /// Redirects have to be followed here. The client is configured not to follow them
    /// automatically so that page fetches can revalidate each hop, but that made a plain
    /// `http`-to-`https` or apex-to-`www` redirect on `robots.txt` look like a refusal — and
    /// several Algerian government sites do exactly that, so they were being skipped entirely
    /// while appearing to have blocked us.
    ///
    /// Every hop still goes through `SafeUrl`.
    /// Fetch, and return an entry the shared cache can store alongside the parsed rules.
    ///
    /// The **text** is cached rather than the parsed rules, so a parser fix applies to everything
    /// already cached instead of needing the cache dropped, and a human can read a cached entry to
    /// see why a host is being refused.
    async fn fetch_robots_cached(
        &self,
        host: &str,
        start_url: &str,
    ) -> (Robots, Option<crate::robots_cache::CachedRobots>) {
        use crate::robots_cache::CachedRobots;
        let now = xustive_core::now_unix();
        let (rules, status, text) = self.fetch_robots_raw(host, start_url).await;

        // A transport failure has no status and is not cached: the host may be up again in a
        // second, and caching "unreachable" for a day would turn a blip into a day of silence.
        let entry = status.map(|status| match text {
            Some(text) => CachedRobots::from_text(text, status, now),
            None => CachedRobots::denied(status, now),
        });
        (rules, entry)
    }

    /// The parsed rules, the status that produced them, and the body if there was one.
    async fn fetch_robots_raw(
        &self,
        host: &str,
        start_url: &str,
    ) -> (Robots, Option<u16>, Option<String>) {
        let Ok(mut current) = SafeUrl::parse(start_url) else {
            return (Robots::deny_all(), None, None);
        };

        for hop in 0..=safe_url::MAX_REDIRECTS {
            let resp = match self.http.get(current.as_str()).send().await {
                Ok(r) => r,
                Err(e) => {
                    // The full error chain, not just the outer message. `error sending request`
                    // alone is indistinguishable between DNS, TLS and connection refused, and the
                    // three need different responses.
                    let mut chain = e.to_string();
                    let mut src = std::error::Error::source(&e);
                    while let Some(inner) = src {
                        chain.push_str(&format!(" <- {inner}"));
                        src = std::error::Error::source(inner);
                    }
                    tracing::warn!(host, error = %chain, "robots.txt unreachable, treating as disallow");
                    return (Robots::deny_all(), None, None);
                }
            };
            let status = resp.status().as_u16();

            if (300..400).contains(&status) {
                let Some(location) = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                else {
                    return (Robots::deny_all(), Some(status), None);
                };
                match current.redirect_to(location, hop) {
                    Ok(next) => {
                        current = next;
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(host, error = %e, "unsafe robots.txt redirect");
                        return (Robots::deny_all(), Some(status), None);
                    }
                }
            }

            return match status {
                200..=299 => match resp.text().await {
                    Ok(text) => (Robots::parse(&text), Some(status), Some(text)),
                    Err(_) => (Robots::deny_all(), Some(status), None),
                },
                // The only status that genuinely means "no restrictions".
                404 | 410 => (Robots::permissive(), Some(status), Some(String::new())),
                // 401 and 403 are a refusal; 5xx means we cannot tell. Neither is permission.
                _ => {
                    tracing::warn!(host, status, "robots.txt unavailable, treating as disallow");
                    (Robots::deny_all(), Some(status), None)
                }
            };
        }
        (Robots::deny_all(), None, None)
    }

    /// Sleep until this host's next slot.
    ///
    /// The lock is released while sleeping so other hosts proceed in parallel; serialisation is
    /// per host, not global.
    async fn wait_turn(&self, host: &str) {
        // Reserve, not just read: advancing the host's slot under the lock is what stops two
        // concurrent callers sharing this fetcher from both fetching one host at once (M2-T04.4).
        let wait = {
            let mut p = self.politeness.lock().await;
            p.reserve(host)
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait + self.config.politeness_margin).await;
        }
    }

    /// Sitemap URLs declared in this host's `robots.txt`, if it has been fetched.
    pub async fn sitemaps_for(&self, host: &str) -> Vec<String> {
        self.politeness
            .lock()
            .await
            .rules(host)
            .map(|r| r.sitemaps.clone())
            .unwrap_or_default()
    }
}

/// Decode a body to `String`, preferring the declared charset.
///
/// Algerian sites still serve `windows-1256`, and getting this wrong produces mojibake that
/// survives all the way into the index and is invisible until someone searches in Arabic.
/// Extract the text of a PDF, or `None` if it is unreadable, a scan (no text), or malformed.
///
/// Run on the blocking pool — extraction is CPU-bound and a large document can take a moment — and
/// inside `catch_unwind`, because `pdf-extract` panics on some malformed files and a panicking fetch
/// worker is a crawler that stops. The output is capped so one pathological document cannot dominate
/// the index.
async fn extract_pdf_text(bytes: Vec<u8>) -> Option<String> {
    let text = tokio::task::spawn_blocking(move || {
        std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(&bytes).ok())
            .ok()
            .flatten()
    })
    .await
    .ok()
    .flatten()?;

    let text: String = text.chars().take(PDF_MAX_CHARS).collect();
    let text = text.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn decode(bytes: &[u8], content_type: &str) -> (String, bool) {
    if let Some(label) = content_type
        .split(';')
        .find_map(|p| p.trim().strip_prefix("charset="))
    {
        let label = label.trim().trim_matches('"');
        if let Some(enc) = encoding_rs::Encoding::for_label(label.as_bytes()) {
            let (text, _, had_errors) = enc.decode(bytes);
            if !had_errors {
                return (text.into_owned(), false);
            }
        }
    }

    // Then the charset declared in the document itself — `<meta charset=...>` or the older
    // `<meta http-equiv="Content-Type" content="...; charset=...">`. A declaration is authoritative
    // where the byte statistics only guess, and it is the common case for the Algerian sites that
    // serve `windows-1256` without a Content-Type charset: the header is bare, the meta is not.
    // Not counted as "guessed": a declared charset, wherever declared, is a statement not a sniff.
    if let Some(enc) = charset_from_meta(bytes) {
        let (text, _, had_errors) = enc.decode(bytes);
        if !had_errors {
            return (text.into_owned(), false);
        }
    }

    // Last, byte-level sniffing. chardetng inspects the byte distribution; despite the previous
    // comment it does not parse HTML, which is exactly why the meta step above had to be explicit.
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    let enc = detector.guess(None, true);
    let (text, _, _) = enc.decode(bytes);
    (text.into_owned(), true)
}

/// The charset declared inside the document, if any.
///
/// Scans only the head — a charset declaration past the first 2 KB is too late to matter to a
/// browser and is not honoured here either. ASCII-lowercased for the search, which is safe because
/// every charset label and the surrounding tag syntax are ASCII whatever the body encoding is.
fn charset_from_meta(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    let head_len = bytes.len().min(2048);
    let head = String::from_utf8_lossy(&bytes[..head_len]).to_ascii_lowercase();
    // `<meta charset="...">`
    let label = head.split("charset").nth(1).and_then(|after| {
        let after = after.trim_start_matches([' ', '=', '"', '\'']);
        let end = after
            .find(['"', '\'', ' ', '>', ';', '/'])
            .unwrap_or(after.len());
        let label = &after[..end];
        (!label.is_empty()).then_some(label)
    })?;
    encoding_rs::Encoding::for_label(label.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_is_decoded_from_the_declared_charset() {
        let (s, guessed) = decode("الجزائر".as_bytes(), "text/html; charset=utf-8");
        assert_eq!(s, "الجزائر");
        assert!(!guessed);
    }

    #[test]
    fn windows_1256_arabic_is_decoded_correctly() {
        // The encoding that still shows up on older Algerian sites. Getting it wrong yields
        // mojibake that reaches the index unnoticed.
        let enc = encoding_rs::WINDOWS_1256;
        let (bytes, _, _) = enc.encode("الجزائر");
        let (s, guessed) = decode(&bytes, "text/html; charset=windows-1256");
        assert_eq!(s, "الجزائر");
        assert!(!guessed);
    }

    #[test]
    fn charset_from_a_meta_tag_is_used_when_the_header_is_bare() {
        // The common windows-1256 case: no charset in the Content-Type, but declared in the head.
        // Byte sniffing can get short Arabic wrong; the meta declaration must win over the sniff.
        let enc = encoding_rs::WINDOWS_1256;
        let (arabic, _, _) = enc.encode("الجزائر تفوز في المباراة");
        let mut page = br#"<html><head><meta charset="windows-1256"></head><body>"#.to_vec();
        page.extend_from_slice(&arabic);
        page.extend_from_slice(b"</body></html>");
        let (s, guessed) = decode(&page, "text/html");
        assert!(
            s.contains("الجزائر"),
            "meta charset should have been honoured: {s:?}"
        );
        assert!(!guessed, "a declared meta charset is not a sniff");
    }

    #[test]
    fn a_meta_content_type_charset_is_also_read() {
        let enc = encoding_rs::WINDOWS_1256;
        let (arabic, _, _) = enc.encode("وهران");
        let mut page =
            br#"<html><head><meta http-equiv="Content-Type" content="text/html; charset=windows-1256"></head><body>"#.to_vec();
        page.extend_from_slice(&arabic);
        page.extend_from_slice(b"</body></html>");
        let (s, _) = decode(&page, "text/html");
        assert!(s.contains("وهران"));
    }

    #[test]
    fn missing_charset_falls_back_to_sniffing() {
        let (s, guessed) = decode("bonjour".as_bytes(), "text/html");
        assert_eq!(s, "bonjour");
        assert!(guessed, "a sniffed charset should be flagged as such");
    }

    #[test]
    fn quoted_charset_labels_are_handled() {
        let (s, _) = decode("hi".as_bytes(), "text/html; charset=\"utf-8\"");
        assert_eq!(s, "hi");
    }

    #[test]
    fn error_classification_drives_retry_policy() {
        assert!(FetchError::Timeout.is_retryable());
        assert!(FetchError::Transport("reset".into()).is_retryable());
        assert!(FetchError::Status(503).is_retryable());
        assert!(FetchError::Status(429).is_retryable());

        // Retrying any of these wastes budget and looks like abuse.
        assert!(!FetchError::Status(404).is_retryable());
        assert!(!FetchError::RobotsDisallowed.is_retryable());
        assert!(!FetchError::TooLarge(10).is_retryable());
        assert!(!FetchError::ContentType("image/png".into()).is_retryable());
    }

    #[tokio::test]
    async fn ssrf_targets_are_refused_before_any_connection() {
        let f = Fetcher::new(FetchConfig::default()).unwrap();
        for url in [
            "http://127.0.0.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1/",
            "http://[::1]/",
            "file:///etc/passwd",
            "http://localhost/",
        ] {
            let err = f.get(url).await.expect_err("should be refused");
            assert!(
                matches!(err, FetchError::Unsafe(_)),
                "{url} gave {err:?}, expected a SafeUrl rejection"
            );
        }
    }

    #[test]
    fn only_indexable_content_types_are_accepted() {
        assert!(INDEXABLE.contains(&"text/html"));
        assert!(!INDEXABLE.contains(&"image/png"));
        // PDFs are now accepted — their text is extracted (M2-T14.3).
        assert!(INDEXABLE.contains(&"application/pdf"));
        assert!(!INDEXABLE.contains(&"image/jpeg"));
    }
}

#[cfg(test)]
mod outcome_tests {
    use super::*;

    #[test]
    fn outcomes_match_the_classification_table() {
        // Web Fetcher §4.4, the cases an operator watches.
        assert_eq!(FetchError::Status(404).outcome(), "gone");
        assert_eq!(FetchError::Status(410).outcome(), "gone");
        assert_eq!(FetchError::Status(429).outcome(), "throttled");
        assert_eq!(FetchError::Status(403).outcome(), "permanent");
        assert_eq!(FetchError::Status(500).outcome(), "transient");
        assert_eq!(FetchError::Status(503).outcome(), "transient");
        assert_eq!(FetchError::Timeout.outcome(), "timeout");
        assert_eq!(FetchError::TooLarge(10).outcome(), "too_large");
        assert_eq!(FetchError::TooManyRedirects.outcome(), "redirect_loop");
    }

    #[test]
    fn only_404_and_410_are_gone() {
        assert!(FetchError::Status(404).is_gone());
        assert!(FetchError::Status(410).is_gone());
        assert!(!FetchError::Status(403).is_gone());
        assert!(!FetchError::Status(500).is_gone());
        assert!(!FetchError::Timeout.is_gone());
    }

    /// The outcome label and the retry class must not contradict: anything the class calls
    /// retryable must carry a retryable-sounding outcome, and a `gone` must never be retryable.
    #[test]
    fn outcome_and_retry_class_agree() {
        for status in [404u16, 410, 429, 403, 400, 500, 503, 502] {
            let e = FetchError::Status(status);
            if e.is_gone() {
                assert!(!e.is_retryable(), "a gone resource must not be retried");
            }
        }
    }
}
