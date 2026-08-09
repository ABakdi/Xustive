//! `/bot` — who we are, and how to make us stop.
//!
//! The URL in the crawler's user-agent string points here, and that string is the only thing a
//! site operator has when they find us in their access log at three in the morning. If it leads
//! nowhere, their options are to guess or to block the whole IP range, and the second one is what
//! actually happens.
//!
//! # Why it is a page and not a paragraph in the docs
//!
//! Someone reading this is annoyed and in a hurry. They want three things in order — what is this,
//! how do I slow it down, how do I stop it entirely — and they want the exact lines to paste, not
//! a description of what to write. Anything that reads as an explanation of our position rather
//! than instructions for taking control is a page that gets closed.
//!
//! Served from the API rather than the frontend, because it must answer even when the UI is not
//! deployed, and it must never be behind JavaScript: this page is read by people looking at raw
//! logs, sometimes with `curl`.

use axum::http::header;
use axum::response::IntoResponse;

use xustive_ingest::robots::{DEFAULT_CRAWL_DELAY, MAX_CRAWL_DELAY, UA_TOKEN, USER_AGENT};

/// `GET /bot`
pub async fn page() -> impl IntoResponse {
    let default_delay = DEFAULT_CRAWL_DELAY.as_secs_f32();
    let max_delay = MAX_CRAWL_DELAY.as_secs();
    let body = format!(
        r#"<header class="site-header"><a class="wordmark" href="/">XUSTIVE</a>
  <span class="muted">crawler</span></header>
<main id="results">
  <h1>XustiveBot</h1>

  <p class="lede">Xustive is a search engine for Algeria. XustiveBot is the crawler that
  builds its index. If you found it in your logs, this page is how you control it.</p>

  <table class="admin">
    <tr><th>User-agent</th><td><code>{USER_AGENT}</code></td></tr>
    <tr><th>robots.txt token</th><td><code>{UA_TOKEN}</code></td></tr>
    <tr><th>Default delay between requests</th><td>{default_delay} s per host</td></tr>
    <tr><th>Concurrent requests per host</th><td>1</td></tr>
  </table>

  <h2>Slow it down</h2>
  <p>Add this to <code>robots.txt</code>. We honour it up to {max_delay} seconds; beyond that we
  reduce how often we visit instead, because holding a connection open for longer helps nobody.</p>
  <pre><code>User-agent: {UA_TOKEN}
Crawl-delay: 10</code></pre>

  <h2>Block part of the site</h2>
  <pre><code>User-agent: {UA_TOKEN}
Disallow: /search
Disallow: /cart</code></pre>

  <h2>Block it entirely</h2>
  <pre><code>User-agent: {UA_TOKEN}
Disallow: /</code></pre>
  <p class="muted">We re-read <code>robots.txt</code> at least once a day, so a change takes
  effect within 24 hours. An unreachable <code>robots.txt</code> — a timeout, a 403, a 5xx — is
  treated as a full block, not as permission.</p>

  <h2>Keep pages out of the index without blocking the crawl</h2>
  <p>Crawling and indexing are separate permissions. To let us follow links through a section but
  keep it out of results, use either of these — the header works for files with no HTML head, such
  as PDFs:</p>
  <pre><code>&lt;meta name="robots" content="noindex"&gt;</code></pre>
  <pre><code>X-Robots-Tag: noindex</code></pre>

  <h2>Ask us directly</h2>
  <p>To have a site removed, to report the crawler misbehaving, or if a block above is not being
  honoured, open an issue at
  <a href="https://github.com/ABakdi/Xustive/issues">github.com/ABakdi/Xustive</a>.
  A crawler ignoring <code>robots.txt</code> is a bug and we want to hear about it.</p>

  <h2>What we do not do</h2>
  <ul>
    <li>We do not submit forms, log in, or attempt to reach anything behind authentication.</li>
    <li>We do not run more than one request at a time against a host.</li>
    <li>We do not disguise the user-agent or rotate it to avoid blocks.</li>
  </ul>
</main>"#
    );

    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            // Cacheable. It changes rarely and is likely to be fetched by someone in a hurry.
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        crate::admin::admin_shell("XustiveBot", &body),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_page_names_the_token_a_site_would_actually_type() {
        // The whole page is useless if the token here differs from the one the parser matches. It
        // would look correct and block nothing.
        let html = render().await;
        assert!(html.contains(UA_TOKEN), "the robots.txt token is missing");
        assert!(
            html.contains(USER_AGENT),
            "the user-agent string is missing"
        );
    }

    #[tokio::test]
    async fn the_page_gives_pasteable_rules_not_prose() {
        // Someone reading this is annoyed and in a hurry. A description of what to write is not
        // the same as the lines to write.
        let html = render().await;
        assert!(html.contains("Disallow: /"), "no block-everything example");
        assert!(html.contains("Crawl-delay:"), "no slow-down example");
        assert!(
            html.contains("noindex"),
            "no way to stay crawlable but unindexed"
        );
    }

    #[tokio::test]
    async fn the_page_says_where_to_complain() {
        let html = render().await;
        assert!(html.contains("github.com"), "no route to a human");
    }

    async fn render() -> String {
        use axum::body::to_bytes;
        use axum::response::IntoResponse;
        let resp = page().await.into_response();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
        String::from_utf8(bytes.to_vec()).expect("utf-8")
    }
}
