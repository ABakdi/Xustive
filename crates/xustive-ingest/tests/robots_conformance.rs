//! RFC 9309 conformance for the `robots.txt` parser.
//!
//! Written against the standard rather than against the implementation. The distinction matters:
//! the parser already had unit tests and they all passed, so a test suite derived from the code
//! would have confirmed the code and found nothing.
//!
//! Getting this wrong is not a bug like other bugs. Fetching a path a site disallowed is the thing
//! that gets a crawler blocked permanently and named in an abuse report, and the site operator has
//! no way to tell a parser defect from contempt.
//!
//! Cases come from RFC 9309 §2.2 (formal syntax), §2.2.2 (the user-agent line), §2.2.3 (rules and
//! longest-match), and from the shapes real servers actually emit — which is where the BOM, the
//! CRLF and the truncation cases come from.

use std::time::Duration;

use xustive_ingest::robots::{Politeness, Robots, UA_TOKEN};

/// Parse and ask about a path in one step.
fn allows(robots_txt: &str, path: &str) -> bool {
    Robots::parse(robots_txt).allows(path)
}

// ── §2.2.3 Longest match ────────────────────────────────────────────────────────────────────

#[test]
fn the_longest_matching_rule_wins() {
    let txt = "User-agent: *\nDisallow: /\nAllow: /public/\n";
    assert!(
        allows(txt, "/public/page"),
        "a longer Allow must beat a shorter Disallow"
    );
    assert!(!allows(txt, "/private/page"));
}

#[test]
fn a_longer_disallow_beats_a_shorter_allow() {
    // The reverse direction, which a parser that always prefers Allow would get wrong.
    let txt = "User-agent: *\nAllow: /docs/\nDisallow: /docs/internal/\n";
    assert!(allows(txt, "/docs/public"));
    assert!(!allows(txt, "/docs/internal/secret"));
}

#[test]
fn allow_wins_a_tie() {
    // RFC 9309 §2.2.3: at equal length the least restrictive rule applies. This is what lets a
    // site carve an exception out of a broad block.
    let txt = "User-agent: *\nDisallow: /page\nAllow: /page\n";
    assert!(allows(txt, "/page"));
}

#[test]
fn no_matching_rule_means_allowed() {
    assert!(allows("User-agent: *\nDisallow: /admin\n", "/about"));
    // And an entirely empty file allows everything.
    assert!(allows("", "/anything"));
}

// ── §2.2.2 Wildcards and anchors ────────────────────────────────────────────────────────────

#[test]
fn a_star_matches_any_run_of_characters() {
    let txt = "User-agent: *\nDisallow: /*.pdf\n";
    assert!(!allows(txt, "/reports/2026.pdf"));
    assert!(allows(txt, "/reports/2026.html"));
}

#[test]
fn a_dollar_anchors_the_end() {
    let txt = "User-agent: *\nDisallow: /*.php$\n";
    assert!(!allows(txt, "/index.php"));
    // Anchored, so a query string after the extension is a different path and stays allowed.
    assert!(allows(txt, "/index.php?id=1"));
    assert!(allows(txt, "/index.phps"));
}

#[test]
fn a_bare_dollar_and_a_bare_star_behave() {
    // `Disallow: /$` blocks only the root, not everything under it — a common way to hide a
    // landing page while leaving the site crawlable.
    let txt = "User-agent: *\nDisallow: /$\n";
    assert!(!allows(txt, "/"));
    assert!(allows(txt, "/page"));

    // `Disallow: *` with no leading slash is malformed but common; treating it as "everything" is
    // the conservative reading and the one the site plainly intended.
    assert!(!allows("User-agent: *\nDisallow: /*\n", "/anything"));
}

#[test]
fn multiple_stars_in_one_pattern() {
    let txt = "User-agent: *\nDisallow: /a/*/b/*/c\n";
    assert!(!allows(txt, "/a/x/b/y/c"));
    assert!(!allows(txt, "/a/xx/b/yy/c/d"));
    assert!(allows(txt, "/a/x/b/y/d"));
}

// ── §2.2.1 Empty disallow ───────────────────────────────────────────────────────────────────

#[test]
fn an_empty_disallow_means_allow_everything() {
    assert!(allows("User-agent: *\nDisallow:\n", "/anything"));
    // Even alongside a real rule: the empty line contributes nothing rather than blocking all.
    assert!(allows(
        "User-agent: *\nDisallow:\nDisallow: /admin\n",
        "/about"
    ));
    assert!(!allows(
        "User-agent: *\nDisallow:\nDisallow: /admin\n",
        "/admin/x"
    ));
}

// ── §2.2.2 Group selection ──────────────────────────────────────────────────────────────────

#[test]
fn a_group_naming_us_replaces_the_wildcard_group() {
    // RFC 9309 §2.2.2: a crawler obeys the most specific group that matches it, and does **not**
    // merge it with `*`. A parser that merged them would apply the wildcard's Disallow to us on
    // top of our own permissions.
    let txt = format!("User-agent: *\nDisallow: /\n\nUser-agent: {UA_TOKEN}\nDisallow: /admin\n");
    assert!(
        allows(&txt, "/public"),
        "our own group should govern, not the wildcard's"
    );
    assert!(!allows(&txt, "/admin/x"));
}

#[test]
fn user_agent_matching_is_case_insensitive() {
    let txt = format!("User-agent: {}\nDisallow: /x\n", UA_TOKEN.to_uppercase());
    assert!(!allows(&txt, "/x"));
}

#[test]
fn consecutive_user_agent_lines_share_one_group() {
    // `User-agent: a` / `User-agent: b` / `Disallow: /x` is one group addressing both.
    let txt = format!("User-agent: someoneelse\nUser-agent: {UA_TOKEN}\nDisallow: /x\n");
    assert!(!allows(&txt, "/x"));
}

#[test]
fn a_rule_line_closes_the_run_of_user_agent_lines() {
    // After a rule, a further `User-agent:` starts a *new* group. Without this the second group's
    // rules would leak into the first.
    let txt = format!("User-agent: {UA_TOKEN}\nDisallow: /a\nUser-agent: other\nDisallow: /b\n");
    assert!(!allows(&txt, "/a"));
    assert!(
        allows(&txt, "/b"),
        "the other agent's rule must not apply to us"
    );
}

#[test]
fn duplicate_groups_for_the_same_agent_are_merged() {
    // RFC 9309 §2.2.2: groups with the same product token are combined.
    let txt =
        format!("User-agent: {UA_TOKEN}\nDisallow: /a\n\nUser-agent: {UA_TOKEN}\nDisallow: /b\n");
    assert!(!allows(&txt, "/a"));
    assert!(!allows(&txt, "/b"));
}

#[test]
fn a_group_for_a_different_agent_does_not_apply() {
    let txt = "User-agent: GPTBot\nDisallow: /\n";
    assert!(
        allows(txt, "/anything"),
        "another crawler's block is not ours"
    );
}

// ── Real-world syntax ───────────────────────────────────────────────────────────────────────

#[test]
fn a_byte_order_mark_does_not_break_the_first_line() {
    // Servers that write robots.txt from a Windows editor emit one, and it lands on the first
    // `User-agent:` — so a parser that misses it ignores the entire first group.
    let txt = "\u{FEFF}User-agent: *\nDisallow: /admin\n";
    assert!(!allows(txt, "/admin/x"));
}

#[test]
fn crlf_line_endings_parse() {
    let txt = "User-agent: *\r\nDisallow: /admin\r\n";
    assert!(!allows(txt, "/admin/x"));
    assert!(allows(txt, "/about"));
}

#[test]
fn comments_are_stripped_wherever_they_appear() {
    let txt = "# leading comment\nUser-agent: *  # trailing\nDisallow: /admin # why\n";
    assert!(!allows(txt, "/admin/x"));
    assert!(allows(txt, "/about"));
}

#[test]
fn whitespace_around_the_colon_is_tolerated() {
    assert!(!allows("User-agent :  *\nDisallow :  /admin\n", "/admin/x"));
}

#[test]
fn unknown_fields_are_ignored_without_ending_a_group() {
    // `Host:` and `Clean-param:` are common non-standard fields. Treating one as a rule line would
    // silently split a group in two.
    let txt = format!("User-agent: {UA_TOKEN}\nHost: example.dz\nDisallow: /a\n");
    assert!(
        !allows(&txt, "/a"),
        "an unknown field must not detach the rules that follow"
    );
}

#[test]
fn a_file_that_is_entirely_junk_allows_everything() {
    // Some hosts serve an HTML error page with a 200. It parses to no rules, which means no
    // restrictions — the correct reading of a file that says nothing.
    let txt = "<!DOCTYPE html><html><body>404 Not Found</body></html>";
    assert!(allows(txt, "/anything"));
}

// ── Crawl-delay ─────────────────────────────────────────────────────────────────────────────

#[test]
fn crawl_delay_is_read_and_bounded() {
    let r = Robots::parse("User-agent: *\nCrawl-delay: 10\n");
    assert_eq!(r.crawl_delay(), Duration::from_secs(10));

    // A hostile or mistaken value must not park a worker for a day.
    let r = Robots::parse("User-agent: *\nCrawl-delay: 999999\n");
    assert!(r.crawl_delay() <= Duration::from_secs(3600));

    // Fractional delays are widely used and must not truncate to zero.
    let r = Robots::parse("User-agent: *\nCrawl-delay: 0.5\n");
    assert_eq!(r.crawl_delay(), Duration::from_millis(500));
}

#[test]
fn a_nonsense_crawl_delay_falls_back_to_the_default() {
    for value in ["abc", "-5", "", "NaN", "inf"] {
        let r = Robots::parse(&format!("User-agent: *\nCrawl-delay: {value}\n"));
        assert!(
            r.crawl_delay() > Duration::ZERO,
            "{value:?} must not produce a zero delay"
        );
    }
}

// ── Robustness ──────────────────────────────────────────────────────────────────────────────

#[test]
fn an_oversized_file_is_truncated_without_panicking() {
    // RFC 9309 §2.5 allows a 500 KiB cap. The cap has to be applied on a character boundary:
    // slicing a `String` mid-character panics, and this input is Arabic, so every character is
    // multi-byte. A site can serve whatever it likes here.
    // Swept across offsets rather than tried once. The first version of this test used a single
    // padding and passed — the cap happened to land on a boundary. A sweep of four offsets is
    // guaranteed to put the cut inside a two-byte character at least once, which is the case that
    // panics.
    for pad in 0..4 {
        let mut txt = String::from("User-agent: *\nDisallow: /admin\n");
        txt.push_str(&" ".repeat(pad));
        txt.push_str(&"# الجزائر\n".repeat(120_000));
        let r = Robots::parse(&txt);
        assert!(
            !r.allows("/admin/x"),
            "rules before the cap must survive (pad {pad})"
        );
    }
}

#[test]
fn deeply_pathological_patterns_terminate() {
    // A pattern of nothing but stars, against a long path. Backtracking implementations go
    // exponential here; this must not.
    let txt = format!("User-agent: *\nDisallow: /{}\n", "*".repeat(200));
    let path = format!("/{}", "a".repeat(2_000));
    let _ = allows(&txt, &path);
}

#[test]
fn no_input_makes_the_parser_panic() {
    for txt in [
        "",
        ":",
        "\n\n\n",
        "User-agent:",
        "Disallow: /",
        "user-agent: *\ndisallow",
        "\u{FEFF}",
        "Crawl-delay: 5",
        "Allow",
        "\0\0\0",
    ] {
        let r = Robots::parse(txt);
        let _ = r.allows("/");
        let _ = r.crawl_delay();
    }
}

// ── Sitemaps ────────────────────────────────────────────────────────────────────────────────

#[test]
fn sitemaps_are_collected_regardless_of_group() {
    // RFC 9309 §2.2.4: `Sitemap:` is not part of any group. A parser that only read it inside the
    // matching group would miss it on most real files, where it sits at the top or bottom.
    let txt = format!(
        "Sitemap: https://example.dz/sitemap.xml\n\
         User-agent: {UA_TOKEN}\nDisallow: /a\n\
         Sitemap: https://example.dz/news.xml\n"
    );
    let r = Robots::parse(&txt);
    assert_eq!(r.sitemaps.len(), 2, "got {:?}", r.sitemaps);
}

// ── The testing bypass ──────────────────────────────────────────────────────────────────────

#[test]
fn the_bypass_is_off_unless_asked_for() {
    let p = Politeness::new();
    assert!(!p.bypassed());
    // And with no rules cached, an unchecked host is refused rather than assumed open.
    assert!(!p.allows("example.dz", "/anything"));
}

#[test]
fn the_bypass_ignores_robots_delays_and_the_fetch_itself() {
    let mut p = Politeness::with_bypass(true);
    // A file that forbids everything, from a host that demands an hour between requests.
    p.set_rules(
        "example.dz",
        Robots::parse("User-agent: *\nDisallow: /\nCrawl-delay: 3600\n"),
    );

    assert!(
        p.allows("example.dz", "/admin/secret"),
        "the bypass must ignore Disallow"
    );
    assert_eq!(
        p.wait_for("example.dz"),
        Duration::ZERO,
        "and the crawl delay"
    );
    // And a host never checked at all is allowed, which is what removes the robots round trip.
    assert!(p.allows("never-seen.dz", "/x"));
    assert!(p.skip_robots_fetch());
}

#[test]
fn without_the_bypass_the_same_rules_are_obeyed() {
    // The other half of the previous test. A bypass that is indistinguishable from normal
    // operation is not a bypass, and one that leaks into normal operation is a liability.
    let mut p = Politeness::new();
    p.set_rules(
        "example.dz",
        Robots::parse("User-agent: *\nDisallow: /\nCrawl-delay: 3600\n"),
    );
    assert!(!p.allows("example.dz", "/admin/secret"));
    assert!(!p.skip_robots_fetch());
}
