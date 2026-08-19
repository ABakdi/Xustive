//! SERP parsers against real saved result pages (M2-T16.15).
//!
//! A live Bing results page, captured from a real query. The point of a fixture is that a layout or
//! redirector change turns this red rather than silently zeroing the channel — SERP markup is the
//! part that rots, so the parser is pinned to a real page, not a hand-built one.

use xustive_ingest::serp::Engine;

fn fixture(name: &str) -> Option<String> {
    let path = format!(
        "{}/../../tests/fixtures/serp/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).ok()
}

#[test]
fn the_bing_parser_extracts_real_result_urls_and_unwraps_the_redirector() {
    let Some(html) = fixture("bing-elkhabar.html") else {
        eprintln!("skipping: fixture not saved");
        return;
    };
    let urls = Engine::Bing.extract(&html);

    assert!(
        urls.len() >= 5,
        "a full results page should yield several URLs, got {}",
        urls.len()
    );
    // Every extracted URL is the real destination, never Bing's own redirector or chrome.
    for u in &urls {
        assert!(u.starts_with("http"), "absolute URL: {u}");
        assert!(
            !u.contains("bing.com") && !u.contains("microsoft.com"),
            "the bing.com/ck/a redirector must be unwrapped, leaked: {u}"
        );
    }
    // The results de-duplicate.
    let mut sorted = urls.clone();
    sorted.sort();
    let before = sorted.len();
    sorted.dedup();
    assert_eq!(before, sorted.len(), "results must be de-duplicated");
}

#[test]
fn the_duckduckgo_lite_parser_extracts_real_result_urls() {
    // A live `lite.duckduckgo.com/lite/?q=paracetamol` page — the endpoint the client actually hits.
    // This pins the lite markup (`a.result-link`, `//duckduckgo.com/l/?uddg=` redirectors): a layout
    // change turns this red instead of silently zeroing the one engine that answers a direct IP.
    let Some(html) = fixture("ddglite-paracetamol.html") else {
        eprintln!("skipping: fixture not saved");
        return;
    };
    let urls = Engine::DuckDuckGo.extract(&html);

    assert!(
        urls.len() >= 5,
        "a full results page should yield several URLs, got {}",
        urls.len()
    );
    for u in &urls {
        assert!(u.starts_with("http"), "absolute URL: {u}");
        assert!(
            !u.contains("duckduckgo.com") && !u.contains("duck.com"),
            "the //duckduckgo.com/l/ redirector must be unwrapped, leaked: {u}"
        );
    }
    // The real destinations are present — the page is genuine results, not a decoy.
    assert!(
        urls.iter().any(|u| u.contains("wikipedia.org")),
        "expected a wikipedia result among {urls:?}"
    );
}
