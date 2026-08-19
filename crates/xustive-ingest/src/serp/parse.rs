//! SERP result-HTML parsers ([[ADR-0013]] §T16.15).
//!
//! One parser per engine, each turning a results page into the list of external result URLs, in
//! order, de-duplicated. Everything here is pure and driven from saved fixtures, because this is the
//! part that rots: a layout change should turn a fixture test red, not quietly yield nothing.
//!
//! Two cross-engine concerns handled centrally:
//!
//! - **Redirect unwrapping.** Engines wrap result links in their own redirector — Google as
//!   `/url?q=<real>`, DuckDuckGo as `//duckduckgo.com/l/?uddg=<real>`. [`clean_result_url`] returns
//!   the real destination or `None` for a link that is navigation, an ad redirector, or the engine's
//!   own domain.
//! - **Challenge detection.** A block or captcha page still parses as HTML and would otherwise look
//!   like a query with no results. [`is_challenge_page`] spots the common interstitials so the caller
//!   can retire the identity rather than record a false zero.

use std::collections::HashSet;

use scraper::{Html, Selector};

/// Hosts that are the engines themselves or known redirectors — never a discovery result.
const ENGINE_HOSTS: [&str; 6] = [
    "google.com",
    "google.",
    "bing.com",
    "microsoft.com",
    "duckduckgo.com",
    "duck.com",
];

/// Turn a raw result `href` into the external destination URL it points at, or `None` if it is not a
/// usable result link (an in-engine link, an ad, a fragment, a redirector we cannot unwrap).
pub fn clean_result_url(href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') {
        return None;
    }

    // Google wraps results as `/url?q=<encoded>&sa=…`. Unwrap to the real target.
    if href.starts_with("/url?") || href.contains("google.com/url?") {
        if let Some(real) = query_param(href, "q") {
            return clean_result_url(&real);
        }
        return None;
    }
    // DuckDuckGo's HTML endpoint wraps as `//duckduckgo.com/l/?uddg=<encoded>` (protocol-relative).
    if href.contains("duckduckgo.com/l/") {
        if let Some(real) = query_param(href, "uddg") {
            return clean_result_url(&real);
        }
        return None;
    }
    // Bing wraps every result as `bing.com/ck/a?…&u=a1<base64(real)>` — the real destination is
    // base64 in the `u` parameter, after a two-character `a1` type prefix. Unwrap it, or the whole
    // page reduces to Bing's own redirector host and yields nothing.
    if href.contains("bing.com/ck/a") {
        if let Some(u) = query_param(href, "u") {
            if let Some(real) = decode_bing_u(&u) {
                return clean_result_url(&real);
            }
        }
        return None;
    }

    // A bare `//host/path` protocol-relative link — treat as https.
    let normalised = if let Some(rest) = href.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        href.to_string()
    };

    let parsed = url::Url::parse(&normalised).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    let host = parsed
        .host_str()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    // Drop the engine's own links and redirectors — they are chrome, not results.
    if ENGINE_HOSTS
        .iter()
        .any(|e| host == *e || host.starts_with(e))
    {
        return None;
    }
    Some(parsed.to_string())
}

/// Decode Bing's `u` redirect parameter: a two-char type prefix (`a1`) followed by the real URL in
/// URL-safe base64, usually unpadded. `None` if it does not decode to valid UTF-8.
fn decode_bing_u(u: &str) -> Option<String> {
    use base64::Engine as _;
    let payload = u.strip_prefix("a1").unwrap_or(u);
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(payload))
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// Read a query-string parameter from a possibly-relative URL, percent-decoded.
fn query_param(href: &str, key: &str) -> Option<String> {
    // Parse against a dummy base so a relative `/url?q=…` still yields its query pairs.
    let base = url::Url::parse("https://serp.local/").ok()?;
    let parsed = base.join(href).ok()?;
    parsed
        .query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

/// Collect result URLs from anchors matching `selector`, cleaned and de-duplicated in order.
fn collect(html: &str, selector: &str) -> Vec<String> {
    let doc = Html::parse_document(html);
    let Ok(sel) = Selector::parse(selector) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for el in doc.select(&sel) {
        if let Some(href) = el.value().attr("href") {
            if let Some(url) = clean_result_url(href) {
                if seen.insert(url.clone()) {
                    out.push(url);
                }
            }
        }
    }
    out
}

/// DuckDuckGo: the lite endpoint's results are `a.result-link`; the older `html.` endpoint used
/// `a.result__a`. Match both so either page parses. Both wrap the target in `//duckduckgo.com/l/`,
/// which the central cleaner unwraps.
pub fn duckduckgo(html: &str) -> Vec<String> {
    collect(html, "a.result-link, a.result__a")
}

/// Bing: organic results are the anchor inside `li.b_algo h2`.
pub fn bing(html: &str) -> Vec<String> {
    collect(html, "li.b_algo h2 a")
}

/// Google: the organic result link is the anchor wrapping the `h3` title. Google's classes are
/// obfuscated and change, so this keys on the stable structure — an `<a>` that contains an `<h3>` —
/// rather than a class, and the central cleaner drops Google's own links.
pub fn google(html: &str) -> Vec<String> {
    let doc = Html::parse_document(html);
    let (Ok(a_sel), Ok(h3_sel)) = (Selector::parse("a"), Selector::parse("h3")) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for a in doc.select(&a_sel) {
        // Only anchors that title a result (contain an h3).
        if a.select(&h3_sel).next().is_none() {
            continue;
        }
        if let Some(href) = a.value().attr("href") {
            if let Some(url) = clean_result_url(href) {
                if seen.insert(url.clone()) {
                    out.push(url);
                }
            }
        }
    }
    out
}

/// Whether the page is a block/challenge interstitial rather than real results. A challenge parses
/// as valid HTML and would otherwise be read as a genuine empty result — the false zero that
/// silent-degradation detection exists to prevent.
pub fn is_challenge_page(html: &str) -> bool {
    let h = html.to_ascii_lowercase();
    const MARKERS: [&str; 7] = [
        "unusual traffic",
        "/sorry/", // google.com/sorry/ captcha
        "recaptcha",
        "captcha-delivery", // datadome
        "are you a robot",
        "verify you are human",
        "detected unusual activity",
    ];
    MARKERS.iter().any(|m| h.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_url_redirects_are_unwrapped_and_engine_links_dropped() {
        assert_eq!(
            clean_result_url("/url?q=https%3A%2F%2Fwww.aps.dz%2Farticle&sa=U&ved=x"),
            Some("https://www.aps.dz/article".to_string())
        );
        // An engine's own link is not a result.
        assert!(clean_result_url("https://google.com/preferences").is_none());
        assert!(clean_result_url("/search?q=next").is_none());
        assert!(clean_result_url("#top").is_none());
        // A plain external link passes through.
        assert!(clean_result_url("https://elkhabar.com/a").is_some());
    }

    #[test]
    fn duckduckgo_redirects_are_unwrapped() {
        assert_eq!(
            clean_result_url(
                "//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.elwatan%2Ddz.com%2Fx&rut=y"
            ),
            Some("https://www.elwatan-dz.com/x".to_string())
        );
    }

    #[test]
    fn duckduckgo_html_is_parsed() {
        let html = r#"<html><body>
            <div class="result"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Faps.dz%2Fa">A</a></div>
            <div class="result"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Felkhabar.com%2Fb">B</a></div>
            <div class="result"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Faps.dz%2Fa">dup</a></div>
            <a class="header__logo" href="https://duckduckgo.com/">logo</a>
        </body></html>"#;
        assert_eq!(
            duckduckgo(html),
            vec!["https://aps.dz/a", "https://elkhabar.com/b"]
        );
    }

    #[test]
    fn bing_organic_results_are_parsed() {
        let html = r#"<html><body>
            <ol id="b_results">
              <li class="b_algo"><h2><a href="https://www.ennaharonline.com/x">t1</a></h2></li>
              <li class="b_ad"><h2><a href="https://ad.example/paid">ad</a></h2></li>
              <li class="b_algo"><h2><a href="https://elbilad.net/y">t2</a></h2></li>
            </ol>
        </body></html>"#;
        assert_eq!(
            bing(html),
            vec!["https://www.ennaharonline.com/x", "https://elbilad.net/y"]
        );
    }

    #[test]
    fn google_results_key_on_the_h3_structure_not_a_class() {
        let html = r#"<html><body>
            <div class="g"><a href="/url?q=https%3A%2F%2Ftsa-algerie.com%2Fa&sa=U"><h3>Title A</h3></a></div>
            <div class="g"><a href="https://www.aps.dz/b"><h3>Title B</h3></a></div>
            <a href="https://google.com/imghp"><h3>Images</h3></a>
            <a href="https://footer.example/nav">no h3, ignored</a>
        </body></html>"#;
        assert_eq!(
            google(html),
            vec!["https://tsa-algerie.com/a", "https://www.aps.dz/b"]
        );
    }

    #[test]
    fn a_challenge_page_is_recognised() {
        assert!(is_challenge_page(
            "<html><body>Our systems have detected unusual traffic…</body></html>"
        ));
        assert!(is_challenge_page(
            r#"<script src="https://www.google.com/recaptcha/api.js"></script>"#
        ));
        assert!(is_challenge_page("<form action=\"/sorry/index\">"));
        assert!(!is_challenge_page(
            "<html><body><div class='g'>real results</div></body></html>"
        ));
    }
}
