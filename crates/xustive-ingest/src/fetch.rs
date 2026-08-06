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

#[derive(Debug, Clone)]
pub struct FetchConfig {
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
    pub max_body_bytes: usize,
    pub robots_ttl: Duration,
    /// Extra pause on top of the host's crawl-delay. Cheap insurance.
    pub politeness_margin: Duration,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            total_timeout: Duration::from_secs(30),
            max_body_bytes: 10 * 1024 * 1024,
            robots_ttl: Duration::from_secs(24 * 3600),
            politeness_margin: Duration::from_millis(200),
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
}

const INDEXABLE: &[&str] = &[
    "text/html",
    "application/xhtml+xml",
    "text/plain",
    "application/xml",
    "text/xml",
    "application/rss+xml",
    "application/atom+xml",
];

pub struct Fetcher {
    http: reqwest::Client,
    politeness: Arc<Mutex<Politeness>>,
    config: FetchConfig,
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
            politeness: Arc::new(Mutex::new(Politeness::new())),
            config,
        })
    }

    /// Fetch a URL, honouring robots and pacing.
    ///
    /// Blocks until this host's crawl-delay has elapsed, which is what enforces one in-flight
    /// request per host when callers share a `Fetcher`.
    pub async fn get(&self, raw_url: &str) -> Result<Fetched, FetchError> {
        let url = SafeUrl::parse(raw_url)?;
        let host = url.host_str().to_string();

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

            let resp = self.http.get(current.as_str()).send().await.map_err(|e| {
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

            let final_url = resp.url().to_string();
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| FetchError::Transport(e.to_string()))?;
            if bytes.len() > self.config.max_body_bytes {
                return Err(FetchError::TooLarge(self.config.max_body_bytes));
            }

            let (body, charset_guessed) = decode(&bytes, &content_type);
            return Ok(Fetched {
                url: raw_url.to_string(),
                final_url,
                status,
                body,
                content_type: mime,
                charset_guessed,
            });
        }

        Err(FetchError::TooManyRedirects)
    }

    /// Fetch and cache `robots.txt` if we have no fresh copy.
    async fn ensure_robots(&self, host: &str, url: &SafeUrl) {
        {
            let p = self.politeness.lock().await;
            if !p.rules_stale(host, self.config.robots_ttl) {
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

        let rules = self.fetch_robots(host, &robots_url).await;

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
    async fn fetch_robots(&self, host: &str, start_url: &str) -> Robots {
        let Ok(mut current) = SafeUrl::parse(start_url) else {
            return Robots::deny_all();
        };

        for hop in 0..=safe_url::MAX_REDIRECTS {
            let resp = match self.http.get(current.as_str()).send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(host, error = %e, "robots.txt unreachable, treating as disallow");
                    return Robots::deny_all();
                }
            };
            let status = resp.status().as_u16();

            if (300..400).contains(&status) {
                let Some(location) = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                else {
                    return Robots::deny_all();
                };
                match current.redirect_to(location, hop) {
                    Ok(next) => {
                        current = next;
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(host, error = %e, "unsafe robots.txt redirect");
                        return Robots::deny_all();
                    }
                }
            }

            return match status {
                200..=299 => match resp.text().await {
                    Ok(text) => Robots::parse(&text),
                    Err(_) => Robots::deny_all(),
                },
                // The only status that genuinely means "no restrictions".
                404 | 410 => Robots::permissive(),
                // 401 and 403 are a refusal; 5xx means we cannot tell. Neither is permission.
                _ => {
                    tracing::warn!(host, status, "robots.txt unavailable, treating as disallow");
                    Robots::deny_all()
                }
            };
        }
        Robots::deny_all()
    }

    /// Sleep until this host's next slot.
    ///
    /// The lock is released while sleeping so other hosts proceed in parallel; serialisation is
    /// per host, not global.
    async fn wait_turn(&self, host: &str) {
        let wait = {
            let p = self.politeness.lock().await;
            p.wait_for(host)
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

    // Fall back to sniffing, which reads any `<meta charset>` in the head as a side effect of
    // scanning the bytes.
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    let enc = detector.guess(None, true);
    let (text, _, _) = enc.decode(bytes);
    (text.into_owned(), true)
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
        assert!(!INDEXABLE.contains(&"application/pdf"));
    }
}
