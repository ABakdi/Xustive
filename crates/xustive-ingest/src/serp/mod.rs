//! Direct SERP collection for discovery (M2-T16.9–.15, [[ADR-0013]]).
//!
//! The last and narrowest rung of the discovery ladder: for a query the corpus cannot answer and
//! that Common Crawl and the API sources did not resolve, ask a general search engine what URLs
//! exist and feed those into the ordinary frontier. As with every other channel we take **only the
//! list of URLs** — a SERP result is a pointer to a page worth fetching ourselves, never content to
//! serve. The engine's terms are disregarded by my direction; the sites the results point at are
//! still crawled politely.
//!
//! # Why a parser module, tested against fixtures
//!
//! SERP markup changes without notice and this will break repeatedly — that maintenance tail is the
//! whole reason it is the last resort. The defence is that the fragile part, turning result HTML
//! into URLs, is **pure and fixture-tested** (§T16.15): each engine has a saved page and a test that
//! asserts the URLs come out, so a layout change is a red test rather than a silent zero-yield
//! channel. `serp_parse_miss_total` (a caller metric) plus these fixtures are the rot alarm.
//!
//! # What lives here vs. what needs infrastructure
//!
//! The **parsing, engine ladder, and challenge detection** are here and complete. The **live fetch**
//! of a SERP — especially Google's — needs the residential egress and browser fingerprint machinery
//! ([[Proxy Manager]], [[Fingerprint Engine]]): a datacentre IP is challenged within a few requests.
//! So [`SerpClient`] fetches through an injected HTTP client, and the proxy/fingerprint wiring is the
//! caller's to supply; the parsing works the moment a request gets through.

mod client;
mod parse;

pub use client::{SerpClient, SerpOutcome};
pub use parse::{clean_result_url, is_challenge_page};

use serde::{Deserialize, Serialize};

/// A general search engine we can read a results page from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// The most scraper-tolerant: a plain HTML endpoint, no JS. Tried first.
    DuckDuckGo,
    Bing,
    /// The most valuable and the most defended — needs residential egress to work at all. Last.
    Google,
}

impl Engine {
    /// The engines in ladder order — lightest and most tolerant first, most-defended last (§T16.11).
    /// Most discovery only needs a list of URLs, which the plainest endpoint gives at a fraction of
    /// the cost and exposure of the hardest one.
    pub const LADDER: [Engine; 3] = [Engine::DuckDuckGo, Engine::Bing, Engine::Google];

    pub const fn as_str(self) -> &'static str {
        match self {
            Engine::DuckDuckGo => "duckduckgo",
            Engine::Bing => "bing",
            Engine::Google => "google",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "duckduckgo" | "ddg" => Some(Engine::DuckDuckGo),
            "bing" => Some(Engine::Bing),
            "google" => Some(Engine::Google),
            _ => None,
        }
    }

    /// The URL of the results page for `query`. `query` is percent-encoded here.
    pub fn results_url(self, query: &str) -> String {
        let q = urlencode(query);
        match self {
            // The no-JS HTML endpoint, not the JS app. Results are plain anchors.
            Engine::DuckDuckGo => format!("https://html.duckduckgo.com/html/?q={q}"),
            Engine::Bing => format!("https://www.bing.com/search?q={q}&count=20"),
            Engine::Google => format!("https://www.google.com/search?q={q}&num=20"),
        }
    }

    /// Extract the result URLs from this engine's results HTML. Pure — the fixture-tested core.
    pub fn extract(self, html: &str) -> Vec<String> {
        match self {
            Engine::DuckDuckGo => parse::duckduckgo(html),
            Engine::Bing => parse::bing(html),
            Engine::Google => parse::google(html),
        }
    }
}

/// Minimal percent-encoding for a query string value — enough for the `q=` parameter.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ladder_puts_the_tolerant_engine_first_and_google_last() {
        assert_eq!(Engine::LADDER[0], Engine::DuckDuckGo);
        assert_eq!(
            Engine::LADDER[2],
            Engine::Google,
            "the most-defended engine is the last resort"
        );
    }

    #[test]
    fn engine_names_round_trip() {
        for e in Engine::LADDER {
            assert_eq!(Engine::parse(e.as_str()), Some(e));
        }
        assert_eq!(Engine::parse("ddg"), Some(Engine::DuckDuckGo));
        assert_eq!(Engine::parse("yahoo"), None);
    }

    #[test]
    fn a_query_is_encoded_into_the_results_url() {
        let url = Engine::Google.results_url("قانون المالية 2027");
        assert!(url.starts_with("https://www.google.com/search?q="));
        assert!(url.contains("%D9"), "arabic is percent-encoded");
        assert!(!url.contains(' '), "no raw spaces in the url");
        // A space becomes '+'.
        assert!(Engine::Bing.results_url("a b").contains("q=a+b"));
    }
}
