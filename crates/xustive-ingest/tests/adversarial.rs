//! Hostile and pathological markup.
//!
//! The parser runs on pages nobody vetted, chosen by a crawler following links. Every input here
//! is something the open web actually contains — some by accident, some deliberately. The bar is
//! not that the parser extracts anything useful from these; it is that it **terminates, does not
//! panic, and does not consume unbounded memory or time**.
//!
//! A crawler that dies on one page loses the whole batch behind it. A crawler that hangs on one
//! page stops crawling entirely, and does so silently.

use std::time::{Duration, Instant};

use xustive_core::SourceType;
use xustive_ingest::{ParseConfig, Parser};

/// Anything slower than this on a single page is a denial of service, not a slow parse.
///
/// Debug builds run this code roughly an order of magnitude slower than the release binary that
/// actually ships, so the budget is scaled rather than either lying about release performance or
/// failing on a build nobody deploys. The interesting number is the release one: with the
/// complexity guard in place the whole suite runs in 0.07 s, against 47 s for a single page
/// before it.
const BUDGET: Duration = if cfg!(debug_assertions) {
    Duration::from_secs(5)
} else {
    Duration::from_millis(500)
};

fn parse_within_budget(html: &str, label: &str) {
    let started = Instant::now();
    let result = std::panic::catch_unwind(|| {
        Parser::new(ParseConfig::default()).parse(
            html,
            "https://example.dz/x",
            "adv",
            SourceType::Web,
        )
    });
    let elapsed = started.elapsed();

    assert!(result.is_ok(), "{label}: parser panicked");
    assert!(elapsed < BUDGET, "{label}: took {elapsed:?}");
}

#[test]
fn deeply_nested_elements_do_not_exhaust_the_stack() {
    // A recursive-descent walk over 50 000 nested divs is a stack overflow, which aborts the
    // process rather than failing the page — no catch_unwind saves that.
    let html = format!(
        "<html><body>{}<p>text</p>{}</body></html>",
        "<div>".repeat(50_000),
        "</div>".repeat(50_000)
    );
    parse_within_budget(&html, "50k nested divs");
}

#[test]
fn an_enormous_node_count_terminates() {
    // Half a million siblings. Not malicious — a paginated archive page can approach this.
    let html = format!(
        "<html><body>{}</body></html>",
        "<span>x</span>".repeat(500_000)
    );
    parse_within_budget(&html, "500k siblings");
}

#[test]
fn unclosed_tags_do_not_loop() {
    // The single most common malformation on the real web.
    let html = format!("<html><body>{}", "<div><p><span>".repeat(20_000));
    parse_within_budget(&html, "20k unclosed");
}

#[test]
fn a_billion_laughs_entity_bomb_is_bounded() {
    // The classic XML expansion attack. HTML parsers do not expand custom entities, so this
    // should be inert — but asserting it is inert is the point, since the failure mode is
    // memory exhaustion rather than an error.
    let html = r#"<!DOCTYPE html [
        <!ENTITY a "aaaaaaaaaa">
        <!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">
        <!ENTITY c "&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;">
        <!ENTITY d "&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;">
        <!ENTITY e "&d;&d;&d;&d;&d;&d;&d;&d;&d;&d;">
        ]><html><body><p>&e;</p></body></html>"#;
    parse_within_budget(html, "entity bomb");
}

#[test]
fn a_very_long_single_attribute_terminates() {
    let html = format!(
        r#"<html><body><div class="{}">text</div></body></html>"#,
        "a".repeat(2_000_000)
    );
    parse_within_budget(&html, "2MB attribute");
}

#[test]
fn a_very_long_single_text_node_terminates() {
    let html = format!("<html><body><p>{}</p></body></html>", "ا".repeat(500_000));
    parse_within_budget(&html, "500k Arabic characters");
}

#[test]
fn pathological_whitespace_terminates() {
    // Whitespace normalisation over a megabyte of nothing.
    let html = format!(
        "<html><body><p>{}text{}</p></body></html>",
        " ".repeat(500_000),
        "\n".repeat(500_000)
    );
    parse_within_budget(&html, "1MB whitespace");
}

#[test]
fn mixed_and_broken_encodings_do_not_panic() {
    // Bytes that are not valid UTF-8 reach the parser as replacement characters. Slicing a
    // multi-byte sequence at the wrong boundary is a panic, and Arabic is entirely multi-byte.
    let cases = [
        "<html><body><p>\u{fffd}\u{fffd}\u{fffd} نص عربي</p></body></html>",
        "<html><body><p>نص\u{0}عربي</p></body></html>",
        // A right-to-left override inside markup, which reorders everything after it.
        "<html><body><p>\u{202e}نص عربي\u{202c}</p></body></html>",
        // Zero-width joiners, which Arabic legitimately uses and which break naive slicing.
        "<html><body><p>ن\u{200d}ص\u{200c}عربي</p></body></html>",
    ];
    for (i, html) in cases.iter().enumerate() {
        parse_within_budget(html, &format!("encoding case {i}"));
    }
}

#[test]
fn a_page_that_is_only_script_yields_nothing_rather_than_script_text() {
    // Indexing minified JavaScript as article text is worse than indexing nothing: it matches
    // queries it has no relation to, and the excerpt is unreadable.
    let html = format!(
        "<html><body><script>{}</script></body></html>",
        "var x=1;function f(){{return x}};".repeat(5_000)
    );
    let parsed = Parser::new(ParseConfig::default()).parse(
        &html,
        "https://example.dz/x",
        "adv",
        SourceType::Web,
    );
    if let Ok(p) = parsed {
        assert!(
            !p.document.body.contains("function"),
            "script text reached the body"
        );
    }
}

#[test]
fn style_and_noscript_content_is_not_indexed() {
    let html = r#"<html><head><style>.a{color:red}</style></head>
        <body><noscript>Enable JavaScript</noscript>
        <article><p>هذا هو النص الحقيقي للمقال وهو طويل بما يكفي لكي يعتبر محتوى صالحا للفهرسة
        ويحتوي على معلومات مفيدة للقارئ.</p></article></body></html>"#;
    let parsed = Parser::new(ParseConfig::default()).parse(
        html,
        "https://example.dz/x",
        "adv",
        SourceType::Web,
    );
    if let Ok(p) = parsed {
        assert!(!p.document.body.contains("color:red"));
        assert!(!p.document.body.contains("Enable JavaScript"));
        assert!(p.document.body.contains("النص الحقيقي"));
    }
}

#[test]
fn an_empty_or_trivial_document_does_not_panic() {
    for html in [
        "",
        " ",
        "<html>",
        "<!DOCTYPE html>",
        "<html></html>",
        "\u{0}",
        "<<<>>>",
    ] {
        parse_within_budget(html, &format!("trivial {html:?}"));
    }
}

#[test]
fn a_link_farm_does_not_produce_unbounded_outlinks() {
    // A page with a hundred thousand links would otherwise enqueue a hundred thousand crawl
    // jobs from one fetch, which is how a crawler turns into a problem for somebody else.
    let links: String = (0..100_000)
        .map(|i| format!(r#"<a href="https://example.dz/{i}">l</a>"#))
        .collect();
    let html = format!("<html><body>{links}</body></html>");

    let started = Instant::now();
    let parsed = Parser::new(ParseConfig::default()).parse(
        &html,
        "https://example.dz/x",
        "adv",
        SourceType::Web,
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "took {:?}",
        started.elapsed()
    );

    if let Ok(p) = parsed {
        assert!(
            p.outlinks.len() <= ParseConfig::default().max_outlinks,
            "emitted {} outlinks",
            p.outlinks.len()
        );
    }
}

#[test]
fn hidden_text_stuffing_does_not_dominate_the_excerpt() {
    // Keyword stuffing in a hidden element is old-fashioned and still present. The parser is not
    // required to strip it — that is the spam scorer's job — but it must not panic on it and the
    // visible article must survive.
    let stuffed = "الجزائر ".repeat(5_000);
    let html = format!(
        r#"<html><body><div style="display:none">{stuffed}</div>
        <article><p>مقال حقيقي عن الاقتصاد الوطني يتضمن معلومات مفيدة وتفاصيل عن القطاع
        الصناعي في الجزائر خلال السنة الجارية.</p></article></body></html>"#
    );
    let parsed = Parser::new(ParseConfig::default()).parse(
        &html,
        "https://example.dz/x",
        "adv",
        SourceType::Web,
    );
    if let Ok(p) = parsed {
        assert!(p.document.body.contains("مقال حقيقي") || p.document.body.contains("الاقتصاد"));
    }
}
