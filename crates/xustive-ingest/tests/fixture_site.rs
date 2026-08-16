//! The crawler against the offline fixture site.
//!
//! Everything here is a case that breaks crawlers on the real web: redirect chains, legacy
//! encodings, robots directives, rate limits, traps. Testing them live means testing slowly,
//! unreproducibly, and while being throttled by someone else's server — so `tests/fixtures/site/`
//! reproduces them locally and this exercises the real [`Fetcher`] against them.
//!
//! The server starts and stops with the test binary. If it cannot start, every test here skips
//! rather than failing: a machine without a spare port should not turn the suite red.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use xustive_core::{DatePrecision, SafeUrl, SourceType};
use xustive_ingest::{FetchConfig, FetchError, Fetcher, ParseConfig, Parser};

fn base() -> String {
    format!("http://127.0.0.1:{}", port())
}

fn port() -> u16 {
    server().unwrap_or(0)
}

/// Start the fixture server once for the whole binary.
///
/// Leaked deliberately: the child must outlive every test, and there is no teardown hook for a
/// test binary. It dies when the process does.
fn server() -> Option<u16> {
    static STARTED: OnceLock<Option<u16>> = OnceLock::new();
    *STARTED.get_or_init(|| {
        // The guard is process-wide, and this binary exists to talk to loopback. Enabling it
        // here rather than in the library keeps every other binary — including both servers —
        // at the default of refusing.
        SafeUrl::allow_loopback_for_testing();

        let script = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/site/serve.py"
        );
        // Port 0: the OS picks a free one and the server prints it. A fixed port collides with
        // an orphan from an earlier run, and the symptom is not "address in use" — it is this
        // suite silently testing against whatever code that orphan was built from.
        let mut child = match Command::new(script)
            .args(["--port", "0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("skipping: could not start the fixture site: {e}");
                return None;
            }
        };

        let stdout = child.stdout.take()?;
        let mut line = String::new();
        if BufReader::new(stdout).read_line(&mut line).is_err() {
            eprintln!("skipping: fixture site printed no port");
            return None;
        }
        let port: u16 = line.trim().parse().ok()?;

        // Leaked deliberately: it must outlive every test, and a test binary has no teardown
        // hook. It is orphaned when this process exits, which is why the port is ephemeral.
        std::mem::forget(child);

        for _ in 0..50 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Some(port);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("skipping: fixture site did not come up");
        None
    })
}

fn fetcher() -> Fetcher {
    Fetcher::new(FetchConfig {
        // Shorter than the fixture's 5-second /slow endpoint, which is the point of that endpoint.
        total_timeout: Duration::from_secs(2),
        connect_timeout: Duration::from_secs(2),
        // The fixture declares Crawl-delay: 1. Honouring it makes this suite slow but it is the
        // behaviour under test, so it stays.
        ..Default::default()
    })
    .expect("fetcher should build")
}

macro_rules! require_server {
    () => {
        if server().is_none() {
            return;
        }
    };
}

#[tokio::test]
async fn a_normal_article_parses_with_title_body_and_date() {
    require_server!();
    let url = format!("{}/articles/normal.html", base());
    let fetched = fetcher().get(&url).await.expect("should fetch");
    assert_eq!(fetched.status, 200);

    let parsed = Parser::new(ParseConfig::default())
        .parse(
            &fetched.body,
            &fetched.final_url,
            "fixture",
            SourceType::Web,
        )
        .expect("should parse");

    assert!(parsed.document.title.contains("استهلاك الكهرباء"));
    assert!(parsed.document.body.contains("سونلغاز"));
    assert!(
        !parsed.document.body.contains("جميع الحقوق محفوظة"),
        "footer boilerplate leaked into the body: {}",
        &parsed.document.body[..120.min(parsed.document.body.len())]
    );
}

#[tokio::test]
async fn a_redirect_chain_is_followed_to_the_end() {
    require_server!();
    let fetched = fetcher()
        .get(&format!("{}/redirect/1", base()))
        .await
        .expect("should follow the chain");
    assert_eq!(fetched.status, 200);
    assert!(
        fetched.final_url.ends_with("/articles/normal.html"),
        "landed on {}",
        fetched.final_url
    );
    assert_ne!(fetched.url, fetched.final_url, "the original URL is kept");
}

#[tokio::test]
async fn a_redirect_loop_terminates() {
    require_server!();
    // The property is that this returns at all. A crawler that follows a cycle does not fail
    // loudly — it just never finishes, which is much harder to notice.
    let result = fetcher().get(&format!("{}/redirect/loop", base())).await;
    assert!(result.is_err(), "a cycle must not resolve: {result:?}");
}

#[tokio::test]
async fn a_disallowed_path_is_never_fetched() {
    require_server!();
    // The single most important assertion in this file. A crawler that ignores robots.txt is not
    // merely buggy; it is the kind of bug that gets an IP range blocked.
    let result = fetcher()
        .get(&format!("{}/private/secret.html", base()))
        .await;
    assert!(
        matches!(result, Err(FetchError::RobotsDisallowed)),
        "expected a robots refusal, got {result:?}"
    );
}

#[tokio::test]
async fn a_legacy_encoding_is_decoded_from_the_meta_tag() {
    require_server!();
    // windows-1256 declared only in the document, with no charset on the response. A fetcher
    // that assumes UTF-8 gets mojibake and indexes it as though it were Arabic — searchable by
    // nobody, and invisible in review because it still looks like text.
    let fetched = fetcher()
        .get(&format!("{}/articles/windows-1256.html", base()))
        .await
        .expect("should fetch");

    assert!(
        fetched.body.contains("الدخول المدرسي"),
        "decoded as: {}",
        &fetched.body[..200.min(fetched.body.len())]
    );
    assert!(
        !fetched.body.contains('\u{fffd}'),
        "replacement characters mean the decode failed"
    );
}

#[tokio::test]
async fn a_slow_response_times_out_rather_than_hanging() {
    require_server!();
    let started = std::time::Instant::now();
    let result = fetcher().get(&format!("{}/slow", base())).await;
    assert!(result.is_err(), "expected a timeout, got {result:?}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "waited {:?}, which is the server's full delay — the budget did not apply",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_server_error_is_reported_not_indexed() {
    require_server!();
    let result = fetcher().get(&format!("{}/500", base())).await;
    assert!(result.is_err(), "a 500 must not become a document");
}

#[tokio::test]
async fn a_javascript_only_page_yields_nothing_rather_than_its_loading_message() {
    require_server!();
    // We do not run JavaScript. The failure mode worth guarding is not "we miss the content" —
    // it is indexing "جارٍ التحميل…" as the article body, which pollutes results with pages
    // that appear to match and say nothing.
    let fetched = fetcher()
        .get(&format!("{}/articles/spa.html", base()))
        .await
        .expect("should fetch");

    match Parser::new(ParseConfig::default()).parse(
        &fetched.body,
        &fetched.final_url,
        "fixture",
        SourceType::Web,
    ) {
        Err(_) => {}
        Ok(parsed) => assert!(
            !parsed.document.body.contains("جارٍ التحميل"),
            "indexed the loading placeholder as content"
        ),
    }
}

#[tokio::test]
async fn malformed_markup_still_yields_the_text_it_contains() {
    require_server!();
    // Unclosed tags, a stray close, nested forms. An HTML5 parser recovers; the assertion is
    // that we do not throw the page away over markup no browser objects to.
    let fetched = fetcher()
        .get(&format!("{}/articles/malformed.html", base()))
        .await
        .expect("should fetch");

    let parsed = Parser::new(ParseConfig::default())
        .parse(
            &fetched.body,
            &fetched.final_url,
            "fixture",
            SourceType::Web,
        )
        .expect("malformed markup must still parse");
    assert!(
        parsed.document.body.contains("ميزانية الولاية"),
        "lost the content: {}",
        parsed.document.body
    );
}

#[tokio::test]
async fn maghrebi_dates_are_recognised() {
    require_server!();
    // أوت and جويلية, not أغسطس and يوليو. A date parser built for Mashriqi Arabic silently
    // fails on every Algerian publication date and leaves freshness ranking with nothing to use.
    let fetched = fetcher()
        .get(&format!("{}/articles/dates.html", base()))
        .await
        .expect("should fetch");

    let parsed = Parser::new(ParseConfig::default())
        .parse(
            &fetched.body,
            &fetched.final_url,
            "fixture",
            SourceType::Web,
        )
        .expect("should parse");
    assert_ne!(
        parsed.document.published_at_precision,
        DatePrecision::Unknown,
        "no date extracted from a page full of Algerian dates"
    );
}

#[tokio::test]
async fn the_sitemap_is_discovered_from_robots_txt() {
    require_server!();
    // Sitemap locations come from the cached robots rules, which are only populated once this
    // fetcher has actually been to the host. Asking a fresh fetcher returns nothing, which is
    // correct and was briefly mistaken for a bug.
    let f = fetcher();
    f.get(&format!("{}/articles/normal.html", base()))
        .await
        .expect("should fetch");

    // Keyed by **authority**, host and port. robots.txt is per origin — `example.dz:8080` serves
    // a different file from `example.dz` and usually a different application — so keying on the
    // bare host would let one inherit rules it never published.
    //
    // This returns the sitemap *locations* declared in robots.txt, not the URLs inside them: one
    // is a discovery step and the other is the crawl frontier.
    let locations = f.sitemaps_for(&format!("127.0.0.1:{}", port())).await;
    assert_eq!(locations.len(), 1, "got {locations:?}");
    assert!(
        locations[0].contains(&port().to_string()),
        "robots.txt must name the host it was served from, got {}",
        locations[0]
    );

    // Now the contents. The fixture sitemap deliberately mixes articles with navigational URLs,
    // which is what aps.dz actually publishes — a crawler that trusts a sitemap to contain only
    // articles indexes the front page several times over.
    let xml = f.get(&locations[0]).await.expect("sitemap should fetch");
    let urls = xustive_ingest::sitemap::extract_urls(&xml.body, 100);
    assert!(
        urls.iter().any(|u| u.ends_with("/articles/normal.html")),
        "articles missing from the sitemap: {urls:?}"
    );
    assert!(
        urls.iter().any(|u| u.ends_with("/category/nation")),
        "the navigational URLs should still be present for the caller to filter: {urls:?}"
    );
}

#[tokio::test]
async fn an_x_robots_tag_header_is_honoured() {
    // The header is the only way a document without a `<head>` can refuse indexing, so honouring
    // the meta tag alone means honouring the request exactly where it is easy.
    //
    // Served as two separate header lines, which is what real servers emit — a parser reading only
    // the first would see `noindex` and miss `nofollow`.
    require_server!();
    let fetched = fetcher()
        .get(&format!("{}/noindex-header.html", base()))
        .await
        .expect("the page is crawlable; only indexing is refused");

    let exclusion = fetched.exclusion.expect("the header should have been read");
    assert!(exclusion.blocks_indexing(), "got {exclusion:?}");
    assert!(
        exclusion.blocks_links(),
        "the second header line was dropped: {exclusion:?}"
    );
    // Still fetched and still parseable. Refusing indexing is not refusing the request.
    assert!(!fetched.body.is_empty());
}

#[tokio::test]
async fn an_x_robots_tag_for_another_crawler_is_ignored() {
    // Obeying `googlebot: noindex` would silently drop documents the site was happy for us to
    // keep, and nothing in the index would show why.
    require_server!();
    let fetched = fetcher()
        .get(&format!("{}/noindex-other-agent.html", base()))
        .await
        .expect("fetch");
    assert!(
        fetched.exclusion.is_none(),
        "another crawler's directive was applied to us: {:?}",
        fetched.exclusion
    );
}

#[tokio::test]
async fn the_politeness_bypass_reaches_a_disallowed_path() {
    // The flag has to change what actually gets fetched, not merely what a helper reports. This
    // path is refused by the fixture's robots.txt and is fetched here — which is the entire point
    // of the flag and the reason it must never be on outside testing.
    require_server!();
    let fetcher = Fetcher::new(FetchConfig {
        ignore_politeness: true,
        ..FetchConfig::default()
    })
    .expect("fetcher");

    let url = format!("{}/private/secret.html", base());
    let fetched = fetcher.get(&url).await.expect("the bypass should reach it");
    assert_eq!(fetched.status, 200);
}

#[tokio::test]
async fn without_the_bypass_the_same_path_is_refused() {
    // The other half. A bypass indistinguishable from normal operation is not a bypass, and one
    // that leaks into normal operation is the reason a crawler gets banned.
    require_server!();
    let url = format!("{}/private/secret.html", base());
    let result = fetcher().get(&url).await;
    assert!(
        result.is_err(),
        "robots.txt disallows /private/ — it must not be fetched: {result:?}"
    );
}

#[tokio::test]
async fn an_unreachable_robots_txt_is_not_permission() {
    // Over real HTTP, against a server that actually returns each status. A 403 or a 5xx is the
    // site refusing or failing, and reading either as "no restrictions" is the mistake that gets a
    // crawler named in an abuse report — while looking perfectly well-behaved in testing.
    require_server!();
    for status in [401u16, 403, 500, 503] {
        let url = format!("{}/robots-status/{status}", base());
        let fetched = fetcher().get(&url).await;
        // The status route itself returns a non-200, so the fetch fails; what matters is that it
        // fails rather than being treated as a crawlable page.
        assert!(
            fetched.is_err(),
            "a {status} response was accepted as a page"
        );
    }
}

// --- crawl depth -------------------------------------------------------------------------------

/// A frontier in its own namespace, or `None` without Redis.
fn depth_frontier(namespace: &str) -> Option<xustive_ingest::frontier::Frontier> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
    xustive_ingest::frontier::Frontier::connect_in(&url, &format!("test:{namespace}")).ok()
}

/// Discovered links must be one hop deeper than the page they came from.
///
/// The regression this exists for: `enqueue_outlinks` hardcoded `let depth = 1`, so every
/// discovery was recorded at depth 1 no matter how far in it actually was. The check immediately
/// below it compared that constant against `max_depth` (3), which is never true — so the depth
/// limit could not fire and every URL in the frontier scored identically, leaving the crawl with
/// no breadth-first ordering at all.
///
/// Seeded at depth 2 rather than 0 on purpose. From a depth-0 seed the old constant and the
/// correct arithmetic agree — both give 1 — so a test starting at the root passes either way and
/// proves nothing.
#[tokio::test]
async fn discovered_links_are_one_hop_deeper_than_their_parent() {
    let Some(f) = depth_frontier("crawl_depth") else {
        eprintln!("skipping: no Redis");
        return;
    };
    if port() == 0 {
        return;
    }
    f.clear().await;

    let Ok(fetcher) = Fetcher::new(FetchConfig {
        ignore_politeness: true,
        ..FetchConfig::default()
    }) else {
        return;
    };
    let mut orch = xustive_ingest::orchestrator::Orchestrator::new(
        fetcher,
        f.clone(),
        xustive_ingest::orchestrator::OrchestratorConfig {
            max_depth: 5,
            default_delay: Duration::from_millis(0),
            ..Default::default()
        },
    );

    let index = format!("{}/index.html", base());
    let parsed = url::Url::parse(&index).unwrap();
    let pending = xustive_ingest::frontier::Pending {
        url: index.clone(),
        host: parsed.host_str().unwrap().to_string(),
        source_id: "fixture".into(),
        depth: 2,
        trust: 77,
        channel: xustive_core::DiscoveryChannel::Seed,
        priority: 0,
    };
    assert_eq!(f.add(&pending).await, Ok(true));

    // One turn: claims the seed, fetches it, queues what it links to.
    let _ = orch.step(1_000).await;

    // The articles the index links to must now be claimable at depth 3, not depth 1.
    let claim = f
        .claim(10_000, Duration::from_millis(0))
        .await
        .expect("the index links to articles, so something should be queued");
    assert_ne!(claim.url, index, "the seed was already consumed");
    assert_eq!(
        claim.depth, 3,
        "a link found on a depth-2 page is at depth 3, not the old constant 1"
    );
    assert_eq!(claim.trust, 77, "trust is inherited from the linking page");
}

/// `max_depth` must actually stop the crawl.
///
/// With the depth constant in place this could only ever fire at `max_depth = 0`, which made the
/// setting look functional while doing nothing at any realistic value.
#[tokio::test]
async fn max_depth_stops_link_following() {
    let Some(f) = depth_frontier("crawl_max_depth") else {
        eprintln!("skipping: no Redis");
        return;
    };
    if port() == 0 {
        return;
    }
    f.clear().await;

    let Ok(fetcher) = Fetcher::new(FetchConfig {
        ignore_politeness: true,
        ..FetchConfig::default()
    }) else {
        return;
    };
    let mut orch = xustive_ingest::orchestrator::Orchestrator::new(
        fetcher,
        f.clone(),
        xustive_ingest::orchestrator::OrchestratorConfig {
            // The seed sits at depth 2, so its links are depth 3 — past the limit.
            max_depth: 2,
            default_delay: Duration::from_millis(0),
            ..Default::default()
        },
    );

    let index = format!("{}/index.html", base());
    let parsed = url::Url::parse(&index).unwrap();
    assert_eq!(
        f.add(&xustive_ingest::frontier::Pending {
            url: index.clone(),
            host: parsed.host_str().unwrap().to_string(),
            source_id: "fixture".into(),
            depth: 2,
            trust: 50,
            channel: xustive_core::DiscoveryChannel::Seed,
            priority: 0,
        })
        .await,
        Ok(true)
    );

    let _ = orch.step(1_000).await;

    assert!(
        f.claim(10_000, Duration::from_millis(0)).await.is_none(),
        "links from a page at max_depth must not be queued"
    );
}

/// A revisit carrying validators gets a 304 instead of the page (M2-T04.2).
///
/// The fixture server is Python's `SimpleHTTPRequestHandler`, which answers `If-Modified-Since`
/// with 304 for static files — so this exercises the real header round trip, not a mock.
#[tokio::test]
async fn a_conditional_refetch_of_an_unchanged_page_costs_no_body() {
    if port() == 0 {
        return;
    }
    let Ok(fetcher) = Fetcher::new(FetchConfig {
        ignore_politeness: true,
        ..FetchConfig::default()
    }) else {
        return;
    };

    let url = format!("{}/articles/normal.html", base());
    let first = fetcher.get(&url).await.expect("plain fetch");
    assert_eq!(first.status, 200);
    let Some(lm) = first.last_modified.clone() else {
        eprintln!("skipping: server sent no Last-Modified");
        return;
    };
    assert!(!first.body.is_empty());

    let second = fetcher
        .get_conditional(
            &url,
            xustive_ingest::fetch::Conditional {
                etag: first.etag.as_deref(),
                last_modified: Some(&lm),
            },
        )
        .await
        .expect("conditional fetch");
    assert_eq!(second.status, 304, "unchanged page should answer 304");
    assert!(second.body.is_empty(), "a 304 carries no body");

    // And without validators the same page still comes back whole — the discovery path is
    // untouched by any of this.
    let third = fetcher.get(&url).await.expect("unconditional refetch");
    assert_eq!(third.status, 200);
    assert!(!third.body.is_empty());
}

/// The fetched split: a first fetch is discovery, a re-fetch of the same page is a revisit (M2-T15.8).
///
/// Freshness (revisits) and coverage (discovery) share one fetch budget, and a single `fetched`
/// total hides which is happening. This proves the two are counted apart: the same URL fetched
/// twice registers as one discovery and one revisit, because the second fetch finds prior visit
/// state and the first does not.
#[tokio::test]
async fn a_refetch_counts_as_a_revisit_not_a_discovery() {
    let Some(f) = depth_frontier("split") else {
        eprintln!("skipping: no Redis");
        return;
    };
    if port() == 0 {
        return;
    }
    f.clear().await;
    let visits = xustive_ingest::revisit::Visits::connect_in(
        &std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into()),
        "test:split",
    )
    .unwrap();

    let Ok(fetcher) = Fetcher::new(FetchConfig {
        ignore_politeness: true,
        ..FetchConfig::default()
    }) else {
        return;
    };
    let mut orch = xustive_ingest::orchestrator::Orchestrator::new(
        fetcher,
        f.clone(),
        xustive_ingest::orchestrator::OrchestratorConfig {
            revisit: true,
            default_delay: Duration::from_millis(0),
            ..Default::default()
        },
    )
    .with_visits(visits);

    let url = format!("{}/articles/normal.html", base());
    let host = url::Url::parse(&url)
        .unwrap()
        .host_str()
        .unwrap()
        .to_string();
    let pending = xustive_ingest::frontier::Pending {
        url: url.clone(),
        host: host.clone(),
        source_id: "fixture".into(),
        depth: 0,
        trust: 60,
        channel: xustive_core::DiscoveryChannel::Seed,
        priority: 0,
    };

    // First fetch: fresh discovery.
    assert_eq!(f.add(&pending).await, Ok(true));
    let _ = orch.step(1_000).await;
    assert_eq!(orch.stats().fetched, 1);
    assert_eq!(
        orch.stats().revisited,
        0,
        "a first fetch is discovery, not a revisit"
    );

    // Second fetch of the same URL: the page is now in `seen`, so re-queue it the way a revisit
    // does — through `defer`, which bypasses `seen` — then promote it due and step again.
    f.defer(&pending, 2_000).await;
    assert_eq!(f.promote_due(2_000, 10).await, 1);
    let _ = orch.step(3_000).await;

    assert_eq!(orch.stats().fetched, 2);
    assert_eq!(
        orch.stats().revisited,
        1,
        "the second fetch of a known page is a revisit; fetched - revisited is discovery"
    );
}

/// Polling a host's sitemap keeps held pages fresh: changed pages are deferred for refetch,
/// unchanged ones extend without a request (M2-T15.6).
#[tokio::test]
async fn a_sitemap_poll_refreshes_changed_pages_and_skips_unchanged() {
    let Some(f) = depth_frontier("sitemap_poll") else {
        eprintln!("skipping: no Redis");
        return;
    };
    if port() == 0 {
        return;
    }
    f.clear().await;
    let redis = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
    let visits = xustive_ingest::revisit::Visits::connect_in(&redis, "test:sitemap_poll").unwrap();
    if visits.get("__probe__").await.is_none() {
        // Confirm Redis actually answers before asserting.
        visits
            .put("__probe__", &xustive_ingest::revisit::Visit::default())
            .await;
        if visits.get("__probe__").await.is_none() {
            eprintln!("skipping: Redis not reachable");
            return;
        }
        visits.forget("__probe__").await;
    }

    let Ok(fetcher) = Fetcher::new(FetchConfig {
        ignore_politeness: true,
        ..FetchConfig::default()
    }) else {
        return;
    };

    // The fixture sitemap dates normal.html at 2026-08-05. Seed a prior fetch of it: one *before*
    // that date (so the sitemap reports it changed) and, for a second URL, one *after*.
    let base = base();
    // The fixture sitemap hardcodes localhost:8099 URLs, so seed the held pages under those, not
    // the test server's dynamic port. We are testing the lastmod verdict against stored state, not
    // fetching the entry pages themselves.
    let changed_url = "http://localhost:8099/articles/normal.html".to_string();
    let unchanged_url = "http://localhost:8099/articles/malformed.html".to_string();

    // 2026-08-05 is 1786060800; 2026-08-04 is 1785974400.
    let mut before = xustive_ingest::revisit::Visit {
        last_fetched: 1_785_000_000, // well before both lastmods → both look changed unless...
        interval_secs: 7200,
        content_hash: "b3:x".into(),
        ..Default::default()
    };
    visits.put(&changed_url, &before).await;

    // For the "unchanged" URL, fetch it *after* its lastmod so the sitemap adds nothing.
    before.last_fetched = 1_790_000_000; // after 2026-08-04
    visits.put(&unchanged_url, &before).await;

    let sitemap = format!("{base}/sitemap.xml");
    let out = xustive_ingest::sitemap_poll::poll_sitemap(
        &fetcher,
        &visits,
        &f,
        &sitemap,
        60,
        1_791_000_000,
        100,
    )
    .await;

    assert!(
        out.changed >= 1,
        "normal.html was fetched before its lastmod, so it changed"
    );
    assert!(
        out.unchanged >= 1,
        "malformed.html was fetched after its lastmod, so it did not"
    );

    // The changed page must be in the due set, ready for a prompt refetch.
    assert!(
        f.deferred().await >= 1,
        "a changed page should be deferred for refetch"
    );
}
