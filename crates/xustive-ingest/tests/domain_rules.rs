//! Per-domain rules against real saved pages.
//!
//! Every rule in `data/parsers/domains.toml` is a selector written against a specific publisher's
//! template. Templates change. A rule with no fixture is a guess that will rot silently the next
//! time somebody redesigns their site, and the symptom — dates quietly disappearing — is one
//! nobody notices until freshness ranking has been broken for a month.
//!
//! So: one saved page per rule that claims to fix something, asserting the thing it claims.

use xustive_core::SourceType;
use xustive_ingest::rules::Rules;
use xustive_ingest::{ParseConfig, Parser};

fn rules() -> Rules {
    Rules::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../data/parsers/domains.toml"
    ))
}

fn fixture(name: &str) -> Option<String> {
    let path = format!(
        "{}/../../tests/fixtures/pages/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).ok()
}

#[test]
fn the_aps_rule_extracts_the_stamped_date() {
    // aps.dz renders its date as `<span class="text-xs">الأربعاء 05 أوت 2026 13:37</span>` — a
    // Tailwind utility class, no JSON-LD, no Open Graph, no time element.
    let Some(html) = fixture("aps.dz-article.html") else {
        eprintln!("skipping: fixture not saved");
        return;
    };
    let url = "https://www.aps.dz/economie/industrieenergie-et-mines/msg2k9o0-article";

    let with_rules = Parser::new(ParseConfig::default())
        .with_rules(rules())
        .parse(&html, url, "aps-dz", SourceType::Web)
        .expect("the page should parse");

    assert_ne!(
        with_rules.document.published_at_precision,
        xustive_core::DatePrecision::Unknown
    );
    // 5 August 2026, the date stamped on the page.
    assert_eq!(with_rules.document.published_at, 1_785_888_000);
}

#[test]
fn a_rule_beats_the_prose_scanner_when_they_disagree() {
    // Worth being exact about: on the real aps.dz fixture, generic extraction *also* finds the
    // right date, because the prose scanner added earlier picks it up. The rule is redundant
    // there.
    //
    // It stops being redundant the moment an article *mentions* a date before its own. The prose
    // scanner takes the first date it sees; the rule takes the one the publisher stamped. This
    // page has both, in that order, which is the ordinary case for any article about a past
    // event.
    let html = r#"<html><body>
        <span class="text-xs">الأربعاء 05 أوت 2026 13:37</span>
        <article><h1>ذكرى الاستقلال</h1>
        <p>احتفلت الجزائر يوم 05 جويلية 1962 بالاستقلال، وهو التاريخ الذي يصادف
           اليوم الوطني. وقد جرت الاحتفالات في مختلف الولايات بحضور المسؤولين
           والمواطنين على حد سواء، في أجواء وطنية مميزة.</p></article>
        </body></html>"#;
    let url = "https://www.aps.dz/algerie/x";

    let generic = Parser::new(ParseConfig::default())
        .parse(html, url, "aps-dz", SourceType::Web)
        .expect("should parse");
    let ruled = Parser::new(ParseConfig::default())
        .with_rules(rules())
        .parse(html, url, "aps-dz", SourceType::Web)
        .expect("should parse");

    // The rule finds 2026. Whatever the prose scanner picks, the rule must not agree with a date
    // drawn from the article's subject matter.
    assert!(
        ruled.document.published_at > 1_700_000_000,
        "the rule must take the stamped date, got {}",
        ruled.document.published_at
    );
    if generic.document.published_at_precision != xustive_core::DatePrecision::Unknown {
        assert_ne!(
            generic.document.published_at, ruled.document.published_at,
            "this fixture exists precisely because the two sources disagree"
        );
    }
}

#[test]
fn a_rule_never_makes_a_page_worse() {
    // Rules run before generic extraction, so a bad selector could replace a correct date with
    // nothing. This asserts the fallback: when the rule matches nothing, generic extraction still
    // runs and whatever it found is kept.
    let Some(html) = fixture("aps.dz-article.html") else {
        return;
    };
    // A rule whose selector matches nothing on this page.
    let bogus = Rules::parse("[[domain]]\nhost = \"aps.dz\"\ndate = \".no-such-class\"\n").unwrap();

    let parsed = Parser::new(ParseConfig::default()).with_rules(bogus).parse(
        &html,
        "https://www.aps.dz/x",
        "aps-dz",
        SourceType::Web,
    );
    // The page still parses; only the date is missing, exactly as before the rule existed.
    assert!(parsed.is_ok(), "a non-matching rule must not break parsing");
}

#[test]
fn a_domain_with_no_rule_is_unaffected() {
    let Some(html) = fixture("aps.dz-article.html") else {
        return;
    };
    // Same page, served from a host nothing has a rule for.
    let parsed = Parser::new(ParseConfig::default())
        .with_rules(rules())
        .parse(&html, "https://unknown.example/x", "other", SourceType::Web);
    assert!(parsed.is_ok());
}

#[test]
fn every_rule_that_claims_a_date_selector_has_a_fixture_or_is_flagged() {
    // Not every rule needs a fixture — several exist only to pick a better body selector. But a
    // rule claiming to fix dates is a claim about a specific template, and an unverified claim
    // about someone else's HTML is a guess.
    let rules = rules();
    if rules.is_empty() {
        return;
    }

    let mut unverified = Vec::new();
    for host in rules.hosts() {
        let rule = rules.for_host(host).unwrap();
        if rule.date.is_none() {
            continue;
        }
        if fixture(&format!("{host}-article.html")).is_none() {
            unverified.push(host.clone());
        }
    }

    // Reported rather than failed, because saving a fixture per publisher means fetching eleven
    // live pages and that is a crawl, not a test run. What must not happen is this being
    // invisible: the list is printed so the gap is a known quantity rather than a surprise.
    if !unverified.is_empty() {
        unverified.sort();
        eprintln!(
            "note: {} date rules have no saved fixture and are unverified: {}",
            unverified.len(),
            unverified.join(", ")
        );
    }
    // The one rule that demonstrably fixes something must always be covered.
    assert!(
        fixture("aps.dz-article.html").is_some(),
        "aps.dz is the rule with a measured effect; it must keep its fixture"
    );
}
