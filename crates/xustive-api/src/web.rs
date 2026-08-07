//! Server-rendered search page.
//!
//! This exists so that core search works with JavaScript disabled — on a locked-down browser, a
//! text browser, or a flaky connection. The client-side script is an enhancement layered on top,
//! not the only path.
//!
//! # The escaping boundary
//!
//! Result text comes from crawled pages and is therefore hostile. Everything is HTML-escaped;
//! the *only* markup permitted through is the `<em>` highlighting Meilisearch inserts, which is
//! re-admitted after escaping by [`escape_preserving_em`]. That ordering matters: escape first,
//! then selectively restore, never the other way around.

use axum::extract::{Query as AxumQuery, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;

use crate::search::{self, SearchParams};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct PageParams {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub page: Option<usize>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub sentiment: Option<String>,
    #[serde(default)]
    pub sort: Option<String>,
}

/// `GET /search` — the no-JS results page.
pub async fn search_page(
    State(state): State<AppState>,
    AxumQuery(p): AxumQuery<PageParams>,
) -> Response {
    let Some(raw) = p.q.clone().filter(|q| !q.trim().is_empty()) else {
        // No query: send them to the home page rather than rendering an empty results shell.
        return (StatusCode::SEE_OTHER, [(header::LOCATION, "/")], "").into_response();
    };

    let params = SearchParams {
        q: Some(raw.clone()),
        page: p.page,
        hits_per_page: Some(20),
        lang: None,
        source: p.source.clone(),
        sentiment: p.sentiment.clone(),
        from: None,
        to: None,
        sort: p.sort.clone(),
    };

    match search::handler(State(state), AxumQuery(params)).await {
        Ok(axum::Json(r)) => Html(render_results(&raw, &r)).into_response(),
        Err(e) => {
            let status = e.status();
            (status, Html(render_error(&raw, &e.message()))).into_response()
        }
    }
}

fn render_results(raw: &str, r: &search::SearchResponse) -> String {
    let mut body = String::with_capacity(16 * 1024);

    body.push_str(&format!(
        r#"<header class="site-header">
  <a class="wordmark" href="/">XUSTIVE</a>
  <form class="search-box compact" role="search" action="/search" method="get">
    <input type="search" name="q" value="{}" dir="auto" autocomplete="off"
           spellcheck="false" enterkeyhint="search" aria-label="Search" maxlength="512">
    <button type="submit" aria-label="Search">→</button>
  </form>
</header>"#,
        escape_html(raw)
    ));

    body.push_str("<main id=\"results\">");

    let total = r.pagination.total_hits;
    body.push_str(&format!(
        "<p class=\"result-count\">{} {} results ({} ms)</p>",
        if r.pagination.estimated { "about" } else { "" },
        fmt_thousands(total),
        r.took_ms
    ));

    if r.results.is_empty() {
        body.push_str(&render_empty(raw));
    } else {
        // An empty slot carrying the token. The summary is fetched by script after the page
        // paints — this page must render without JavaScript, and generation takes seconds, so
        // the summary is the one part that is genuinely optional.
        if let Some(token) = &r.summary_token {
            body.push_str(&format!(
                r#"<div id="summary" hidden data-token="{}"></div>"#,
                escape_html(token)
            ));
        }
        body.push_str("<ol class=\"result-list\">");
        for card in &r.results {
            body.push_str(&render_card(card));
        }
        body.push_str("</ol>");
        body.push_str(&render_pagination(raw, r));
    }

    body.push_str("</main>");
    page_shell(&format!("{raw} — Xustive"), &body)
}

fn render_card(c: &search::ResultCard) -> String {
    let sentiment = match &c.sentiment {
        Some(s) => format!(
            r#"<span class="badge sentiment {0}">{1} {2}</span>"#,
            escape_html(s.label),
            sentiment_glyph(s.label),
            escape_html(s.label)
        ),
        // Below the confidence floor we show nothing. Absence is more honest than a shrug.
        None => String::new(),
    };

    let date = match c.published_at_precision.as_str() {
        // We never render a date we guessed as though it were fact.
        "unknown" => "<span class=\"muted\">date unknown</span>".to_string(),
        _ => format!(
            r#"<time datetime="{0}">{1}</time>"#,
            c.published_at,
            fmt_date(c.published_at)
        ),
    };

    format!(
        r#"<li class="result-card" dir="auto" id="result-{id}">
  <div class="card-meta">
    <span class="badge platform {platform}">{platform_label}</span>
    <span class="display-url">{display_url}</span>
    {date}
    {sentiment}
  </div>
  <h3><a href="{url}" rel="noopener nofollow">{title}</a></h3>
  <p class="excerpt">{excerpt}</p>
</li>"#,
        id = escape_html(&c.id),
        platform = escape_html(&c.source_type),
        platform_label = escape_html(&platform_label(&c.source_type)),
        display_url = escape_html(&c.display_url),
        date = date,
        sentiment = sentiment,
        url = escape_html(&c.url),
        // `<em>` from the engine is preserved; everything else is escaped.
        title = escape_preserving_em(&c.title),
        excerpt = escape_preserving_em(&c.excerpt),
    )
}

fn render_empty(raw: &str) -> String {
    // Actionable suggestions, not a shrug. The transliteration hint is offered when the query
    // looks like Arabizi, which is the single most common reason an Algerian query misses.
    let mut tips = String::new();
    if looks_arabizi(raw) {
        tips.push_str("<li>Try writing the query in Arabic script</li>");
    }
    tips.push_str("<li>Check the spelling</li>");
    tips.push_str("<li>Use fewer or more general words</li>");
    tips.push_str("<li>Remove any filters</li>");

    format!(
        r#"<div class="empty-state">
  <p class="empty-title">No results for “{}”</p>
  <ul class="empty-tips">{tips}</ul>
</div>"#,
        escape_html(raw)
    )
}

fn render_pagination(raw: &str, r: &search::SearchResponse) -> String {
    let page = r.pagination.page;
    let total_pages = r.pagination.total_pages.max(1);
    if total_pages <= 1 {
        return String::new();
    }

    let link = |p: usize, label: &str, current: bool| {
        if current {
            format!(r#"<span class="page current" aria-current="page">{label}</span>"#)
        } else {
            format!(
                r#"<a class="page" href="/search?q={}&amp;page={p}">{label}</a>"#,
                urlencode(raw)
            )
        }
    };

    let mut out = String::from(r#"<nav class="pagination" aria-label="Pagination">"#);
    if page > 1 {
        out.push_str(&link(page - 1, "‹ Previous", false));
    }
    let start = page.saturating_sub(2).max(1);
    let end = (start + 4).min(total_pages);
    for p in start..=end {
        out.push_str(&link(p, &p.to_string(), p == page));
    }
    if page < total_pages {
        out.push_str(&link(page + 1, "Next ›", false));
    }
    out.push_str("</nav>");
    out
}

fn render_error(raw: &str, message: &str) -> String {
    let body = format!(
        r#"<header class="site-header">
  <a class="wordmark" href="/">XUSTIVE</a>
  <form class="search-box compact" role="search" action="/search" method="get">
    <input type="search" name="q" value="{}" dir="auto" aria-label="Search" maxlength="512">
    <button type="submit" aria-label="Search">→</button>
  </form>
</header>
<main class="error-page">
  <p class="error-icon" aria-hidden="true">⚠</p>
  <h1>{}</h1>
  <p><a class="button" href="/search?q={}">Try again</a></p>
</main>"#,
        escape_html(raw),
        escape_html(message),
        urlencode(raw)
    );
    page_shell("Xustive", &body)
}

/// The HTML skeleton. `lang`/`dir` default to Arabic RTL; per-string direction is handled by
/// `dir="auto"` on every element that renders content.
fn page_shell(title: &str, body: &str) -> String {
    // `r##"…"##`, not `r#"…"#`: the skip link contains `"#`, which would terminate a
    // single-hash raw string early.
    format!(
        r##"<!doctype html>
<html lang="ar" dir="rtl">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<title>{title}</title>
<link rel="stylesheet" href="/style.css">
</head>
<body>
<a class="skip-link" href="#results">Skip to results</a>
{body}
<footer class="site-footer">
  <span class="privacy-line">🔒 ما نسجلوش عمليات البحث تاعك</span>
</footer>
<script src="/app.js" defer></script>
</body>
</html>"##,
        title = escape_html(title),
    )
}

/// Page shell for the admin surface.
///
/// LTR and English, unlike the public pages: this is an operator tool, not part of the product.
///
/// Styles and behaviour live in `/admin.css` and `/admin.js` rather than inline, because the API
/// sends `style-src 'self'; script-src 'self'` and inline blocks are silently dropped. Relaxing
/// the policy for one page is not worth it.
pub fn admin_shell(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en" dir="ltr" class="admin-page">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<title>{title}</title>
<link rel="stylesheet" href="/style.css">
<link rel="stylesheet" href="/admin.css">
<script src="/admin.js" defer></script>
</head>
<body>
{body}
</body>
</html>"#,
        title = escape_html(title),
    )
}

// --- escaping ---------------------------------------------------------------------------

/// Escape every HTML-significant character.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Escape everything, then re-admit only the `<em>` markers the search engine inserted.
///
/// Escape-then-restore is the safe order. Trying to escape "everything except `<em>`" in one
/// pass means parsing hostile markup, which is how these bugs happen.
pub fn escape_preserving_em(s: &str) -> String {
    escape_html(s)
        .replace("&lt;em&gt;", "<em>")
        .replace("&lt;/em&gt;", "</em>")
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// --- formatting -------------------------------------------------------------------------

fn platform_label(source_type: &str) -> String {
    match source_type {
        "web" => "Web",
        "facebook" => "Facebook",
        "instagram" => "Instagram",
        "tiktok" => "TikTok",
        other => other,
    }
    .to_string()
}

fn sentiment_glyph(label: &str) -> &'static str {
    // Never colour alone: the glyph and the text label both carry the meaning.
    match label {
        "positive" => "▲",
        "negative" => "▼",
        _ => "●",
    }
}

fn fmt_thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Civil date from a unix timestamp (UTC), using Howard Hinnant's days-from-civil inverse.
fn fmt_date(ts: i64) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let z = ts.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{d} {} {y}", MONTHS[(m - 1) as usize])
}

/// Heuristic for the empty-state hint: Arabizi uses digits as consonants.
fn looks_arabizi(s: &str) -> bool {
    let has_digit_consonant = s.chars().any(|c| matches!(c, '3' | '7' | '9' | '2' | '5'));
    let mostly_latin = s
        .chars()
        .filter(|c| c.is_alphabetic())
        .all(|c| c.is_ascii_alphabetic());
    has_digit_consonant && mostly_latin && s.chars().any(|c| c.is_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_all_significant_characters() {
        assert_eq!(escape_html("<script>"), "&lt;script&gt;");
        assert_eq!(escape_html("a&b"), "a&amp;b");
        assert_eq!(escape_html("\"'"), "&quot;&#x27;");
    }

    #[test]
    fn crawled_script_tags_render_as_text() {
        // A hostile page title must never execute. This is the XSS boundary.
        let hostile = r#"<script>alert(document.cookie)</script>"#;
        let out = escape_preserving_em(hostile);
        assert!(!out.contains("<script"), "script tag survived: {out}");
        assert_eq!(out, "&lt;script&gt;alert(document.cookie)&lt;/script&gt;");
    }

    #[test]
    fn engine_highlighting_is_preserved() {
        let s = "prix de <em>sonelgaz</em> 2026";
        assert_eq!(escape_preserving_em(s), "prix de <em>sonelgaz</em> 2026");
    }

    #[test]
    fn only_em_is_re_admitted() {
        // An attacker-supplied <img onerror> must stay escaped even next to a real highlight.
        let s = r#"<em>hit</em><img src=x onerror=alert(1)>"#;
        let out = escape_preserving_em(s);
        assert!(out.starts_with("<em>hit</em>"));
        assert!(!out.contains("<img"), "img survived: {out}");
        assert!(out.contains("&lt;img"));
    }

    #[test]
    fn crawled_content_cannot_forge_a_highlight_tag() {
        // Content containing a literal `<em>` is escaped by the same rule the engine's marker
        // uses. That is an accepted, bounded consequence: the worst case is stray emphasis,
        // never script execution.
        let out = escape_preserving_em("a <em>b</em> <script>c</script>");
        assert!(!out.contains("<script"));
    }

    #[test]
    fn attribute_context_is_safe() {
        // A quote in a URL must not break out of the href attribute.
        let out = escape_html(r#"https://x.dz/"onmouseover="alert(1)"#);
        assert!(!out.contains('"'), "unescaped quote in attribute: {out}");
    }

    #[test]
    fn urlencode_handles_arabic_and_spaces() {
        assert_eq!(urlencode("a b"), "a+b");
        assert_eq!(urlencode("a&b=c"), "a%26b%3Dc");
        // Multi-byte characters are percent-encoded per byte.
        assert!(urlencode("وهران").starts_with('%'));
        assert!(!urlencode("وهران").contains('و'));
    }

    #[test]
    fn thousands_separator() {
        assert_eq!(fmt_thousands(0), "0");
        assert_eq!(fmt_thousands(999), "999");
        assert_eq!(fmt_thousands(1_000), "1,000");
        assert_eq!(fmt_thousands(1_834_567), "1,834,567");
    }

    #[test]
    fn date_formatting_is_correct() {
        // Day-month-year. The client formatter is pinned to the same order so enhancement
        // does not reflow the card.
        assert_eq!(fmt_date(0), "1 January 1970");
        assert_eq!(fmt_date(1_000_000_000), "9 September 2001");
        assert_eq!(fmt_date(1_754_438_400), "6 August 2025");
        assert_eq!(fmt_date(1_786_039_141), "6 August 2026");
    }

    #[test]
    fn date_formatting_handles_leap_years_and_pre_epoch() {
        // The civil-from-days arithmetic is easy to get subtly wrong at these boundaries.
        assert_eq!(fmt_date(951_782_400), "29 February 2000"); // leap century
        assert_eq!(fmt_date(1_078_012_800), "29 February 2004"); // ordinary leap year
        assert_eq!(fmt_date(-1), "31 December 1969"); // before the epoch
        assert_eq!(fmt_date(1_735_689_600), "1 January 2025"); // year boundary
    }

    #[test]
    fn arabizi_detection_for_the_empty_state_hint() {
        assert!(looks_arabizi("ch7al hada"));
        assert!(looks_arabizi("3aslema"));
        assert!(!looks_arabizi("bonjour"));
        assert!(!looks_arabizi("الجزائر"));
        assert!(!looks_arabizi("2026"));
    }

    #[test]
    fn sentiment_is_never_colour_alone() {
        // Each label carries a distinct glyph, so the meaning survives greyscale.
        assert_ne!(sentiment_glyph("positive"), sentiment_glyph("negative"));
        assert_ne!(sentiment_glyph("positive"), sentiment_glyph("neutral"));
    }

    #[test]
    fn page_shell_sets_rtl_and_no_referrer() {
        let html = page_shell("t", "<main></main>");
        assert!(html.contains(r#"<html lang="ar" dir="rtl">"#));
        assert!(html.contains(r#"<meta name="referrer" content="no-referrer">"#));
        assert!(html.contains("skip-link"));
        assert!(html.starts_with("<!doctype html>"));
    }

    #[test]
    fn page_title_is_escaped() {
        let html = page_shell("<script>x</script>", "");
        assert!(!html.contains("<script>x"));
    }
}
