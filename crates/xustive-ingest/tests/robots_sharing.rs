//! Does one worker's `robots.txt` fetch serve the others?
//!
//! In its own test binary, not alongside the rest of the fixture-site suite. Rust runs tests in a
//! binary in parallel, and this one asserts a **count of requests the server saw** — every other
//! test that fetches a page also fetches `robots.txt`, so sharing a process made the number depend
//! on scheduling. It passed alone and failed in the suite.
//!
//! A separate binary gets its own fixture server on its own port, and therefore its own counter
//! and its own cache keys.

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

/// Redis for the shared-cache tests. Absent means skip rather than fail — this suite must run on a
/// machine with no infrastructure.
fn shared_cache() -> Option<xustive_ingest::robots_cache::RobotsCache> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
    xustive_ingest::robots_cache::RobotsCache::connect(&url)
}

#[tokio::test]
async fn robots_txt_is_shared_between_workers() {
    // The behaviour the cache exists for. Two independent fetchers — which is what two workers
    // are — must not each request robots.txt from the same host.
    //
    // Asserted by counting requests at the server rather than by inspecting our own state: the
    // thing that matters is what the site sees in its access log, and an in-process cache would
    // pass a state-based check while still sending two requests.
    require_server!();
    let Some(cache) = shared_cache() else {
        eprintln!("skipping: no Redis");
        return;
    };

    let host = format!("127.0.0.1:{}", port());
    // Both sides of the state must be reset, not just the server's counter. Clearing only the
    // counter left the Redis entry from an earlier run in place, so *neither* fetcher requested
    // robots.txt and the assertion passed for entirely the wrong reason.
    cache.forget(&host).await;
    let _ = reqwest::Client::new()
        .get(format!("{}/robots-count/reset", base()))
        .send()
        .await;

    let a = Fetcher::new(FetchConfig::default())
        .expect("fetcher")
        .with_shared_cache(cache.clone());
    let b = Fetcher::new(FetchConfig::default())
        .expect("fetcher")
        .with_shared_cache(cache);

    let _ = a.get(&format!("{}/articles/normal.html", base())).await;
    let _ = b.get(&format!("{}/index.html", base())).await;

    let count: usize = reqwest::Client::new()
        .get(format!("{}/robots-count", base()))
        .send()
        .await
        .expect("count")
        .text()
        .await
        .expect("body")
        .trim()
        .parse()
        .unwrap_or(usize::MAX);

    assert_eq!(
        count, 1,
        "robots.txt was requested {count} times for {host}; the second worker should have used \
         the shared cache"
    );
}
