//! The SERP client: walk the engine ladder, fetch a results page, parse it ([[ADR-0013]]
//! §T16.11–.14).
//!
//! Given a query, this tries the engines lightest-first and returns the first engine's result URLs.
//! It presents as a browser, not as `XustiveBot` — the whole point is to read a page a general
//! engine serves to a person, so an honest bot User-Agent would be refused immediately. A challenge
//! or block is detected (not solved: a challenge means the identity is already classified, and
//! pushing through burns it faster than resting it, §T16.13) and the ladder moves on to the next
//! engine.
//!
//! The HTTP client is **injected**, which is the seam for the proxy/fingerprint machinery: a bare
//! client from here works for the tolerant engines and gets Google's `/sorry/` page from a
//! datacentre IP, exactly as documented; a client built over a residential proxy and a coherent
//! fingerprint gets through. The parsing and ladder logic do not change either way.

use std::time::Duration;

use super::parse::is_challenge_page;
use super::Engine;

/// Why a SERP fetch did not yield results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SerpOutcome {
    /// Results parsed from the page.
    Results(Vec<String>),
    /// A block/challenge interstitial — the identity is classified; back off, do not push.
    Challenged,
    /// A 2xx page that parsed to no results — a genuine miss, or a layout change (the rot alarm).
    Empty,
    /// The request failed (network, non-2xx that is not a challenge).
    Failed,
}

/// A client for reading search-engine result pages.
#[derive(Clone)]
pub struct SerpClient {
    http: reqwest::Client,
    engines: Vec<Engine>,
    /// Politeness between engine attempts within one query — human-shaped pacing (§T16.12).
    step_delay: Duration,
}

impl SerpClient {
    /// Build a client for `engines` (in the given order), presenting a browser fingerprint. A bad
    /// or empty engine list falls back to the full ladder.
    ///
    /// `proxy` is the seam that actually unblocks results. Empty/`None` is a direct connection —
    /// which a datacentre IP gets challenge pages on. A `http(s)://` or `socks5://` URL (credentials
    /// allowed inline) routes every fetch through it; a residential/rotating proxy there is what gets
    /// the engines to serve real pages. A malformed proxy URL fails the build rather than silently
    /// falling back to a direct connection that would just get blocked.
    pub fn new(engines: Vec<Engine>, proxy: Option<&str>) -> Option<Self> {
        let engines = if engines.is_empty() {
            Engine::LADDER.to_vec()
        } else {
            engines
        };
        // A current, ordinary Chrome-on-Windows identity, with headers a real browser sends. Coherent
        // enough to be served a normal page; a full deployment injects a complete fingerprint profile
        // ([[Fingerprint Engine]]). Algeria-shaped Accept-Language (Arabic/French/English).
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
            ),
        );
        headers.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            reqwest::header::HeaderValue::from_static("ar,fr;q=0.9,en;q=0.8"),
        );
        let mut builder = reqwest::Client::builder()
            .user_agent(ua)
            .default_headers(headers)
            .timeout(Duration::from_secs(20));
        // A configured proxy is the whole point on a datacentre host — route through it or fail loud.
        if let Some(url) = proxy.map(str::trim).filter(|s| !s.is_empty()) {
            builder = builder.proxy(reqwest::Proxy::all(url).ok()?);
        }
        let http = builder.build().ok()?;
        Some(Self {
            http,
            engines,
            step_delay: Duration::from_millis(800),
        })
    }

    /// Override the between-engine delay (tests set it to zero).
    pub fn with_step_delay(mut self, d: Duration) -> Self {
        self.step_delay = d;
        self
    }

    /// Resolve `query` to result URLs, trying engines in ladder order. Returns the URLs from the
    /// first engine that answers; an engine that challenges or fails is skipped to the next.
    pub async fn search(&self, query: &str) -> Vec<String> {
        for (i, engine) in self.engines.iter().enumerate() {
            if i > 0 && !self.step_delay.is_zero() {
                tokio::time::sleep(self.step_delay).await;
            }
            match self.fetch_one(*engine, query).await {
                SerpOutcome::Results(urls) if !urls.is_empty() => return urls,
                // A challenge or a failure: this engine is not usable right now, try the next.
                SerpOutcome::Challenged => {
                    tracing::warn!(
                        engine = engine.as_str(),
                        "serp challenge; moving down the ladder"
                    );
                }
                SerpOutcome::Empty => {
                    tracing::debug!(engine = engine.as_str(), "serp returned no results");
                }
                SerpOutcome::Results(_) | SerpOutcome::Failed => {}
            }
        }
        Vec::new()
    }

    /// Fetch and classify one engine's results page. Public-ish for the caller's per-engine metrics.
    pub async fn fetch_one(&self, engine: Engine, query: &str) -> SerpOutcome {
        let url = engine.results_url(query);
        let resp = match self.http.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return SerpOutcome::Failed,
        };
        let status = resp.status();
        // 429/503 are rate-limit/challenge signals even before reading the body.
        if status.as_u16() == 429 || status.as_u16() == 503 {
            return SerpOutcome::Challenged;
        }
        let body = match resp.text().await {
            Ok(b) => b,
            Err(_) => return SerpOutcome::Failed,
        };
        if is_challenge_page(&body) {
            return SerpOutcome::Challenged;
        }
        if !status.is_success() {
            return SerpOutcome::Failed;
        }
        let urls = engine.extract(&body);
        if urls.is_empty() {
            SerpOutcome::Empty
        } else {
            SerpOutcome::Results(urls)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_engine_list_falls_back_to_the_full_ladder() {
        let c = SerpClient::new(vec![], None).unwrap();
        assert_eq!(c.engines, Engine::LADDER.to_vec());
    }

    #[test]
    fn a_custom_engine_order_is_respected() {
        let c = SerpClient::new(vec![Engine::Google, Engine::Bing], None).unwrap();
        assert_eq!(c.engines, vec![Engine::Google, Engine::Bing]);
    }

    #[test]
    fn a_proxy_url_is_accepted_and_an_empty_one_is_a_direct_connection() {
        // No proxy and an empty/whitespace proxy both build a (direct) client.
        assert!(SerpClient::new(vec![Engine::Bing], None).is_some());
        assert!(SerpClient::new(vec![Engine::Bing], Some("")).is_some());
        assert!(SerpClient::new(vec![Engine::Bing], Some("   ")).is_some());
        // http and socks5 proxy URLs (with and without inline credentials) build.
        assert!(SerpClient::new(
            vec![Engine::Bing],
            Some("http://user:pass@gate.example:7000")
        )
        .is_some());
        assert!(SerpClient::new(vec![Engine::Bing], Some("socks5://gate.example:1080")).is_some());
    }

    #[tokio::test]
    async fn a_search_completes_without_panicking() {
        // Network-tolerant: with no network every engine fails and this returns empty; with network
        // it may return real result URLs. Either way it must not panic, and any URL it does return
        // is a well-formed absolute http(s) URL (the cleaner's contract).
        let c = SerpClient::new(vec![Engine::DuckDuckGo], None)
            .unwrap()
            .with_step_delay(Duration::ZERO);
        for url in c.search("قانون المالية الجزائر").await {
            assert!(
                url.starts_with("http"),
                "a result must be an absolute URL: {url}"
            );
        }
    }
}
