//! Polling a host's sitemap to drive freshness (M2-T15.6).
//!
//! The parser ([`crate::sitemap`]) and the decision logic ([`crate::revisit::sitemap_verdict`]) are
//! the pieces; this joins them to a fetcher and the frontier. One pass over one host's sitemap:
//!
//! - fetch the sitemap and read its `<loc>`/`<lastmod>` pairs;
//! - for each URL we already hold, ask what the `lastmod` says;
//! - a page the sitemap reports **changed** is deferred into the frontier as due now, so the
//!   ordinary fetch path picks it up promptly instead of waiting out its interval;
//! - a page reported **unchanged** takes a free "unchanged" observation, growing its interval with
//!   no request at all — which is the whole economy of this: one sitemap fetch stands in for
//!   hundreds of revisits that would each have cost a request to learn nothing.
//!
//! Pages the sitemap lists that we have never crawled are ignored here. Discovering new URLs from a
//! sitemap is [`crate::sitemap::extract_urls`]'s job on the crawl path; this is strictly about
//! keeping what we already hold fresh.

use crate::fetch::Fetcher;
use crate::frontier::{self, Frontier, Pending};
use crate::revisit::{SitemapVerdict, Visits};

/// What one poll of one sitemap did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PollOutcome {
    /// Entries the sitemap reported changed, deferred for a prompt refetch.
    pub changed: usize,
    /// Entries reported unchanged, whose interval was extended without a fetch.
    pub unchanged: usize,
    /// Entries the sitemap tells us nothing actionable about (no `lastmod`, or never crawled).
    pub unknown: usize,
}

/// Poll one sitemap URL and apply its `lastmod` values to the schedule.
///
/// `trust` is the source's trust tier, used both to bound the interval growth and as the priority
/// of any page promoted for refetch. `now` is unix seconds, passed in rather than read so the
/// function is deterministic and testable.
///
/// Best-effort throughout: a sitemap that will not fetch or parse yields an empty outcome rather
/// than an error, because a freshness optimisation must never be able to stop the crawl.
pub async fn poll_sitemap(
    fetcher: &Fetcher,
    visits: &Visits,
    frontier: &Frontier,
    sitemap_url: &str,
    trust: u8,
    now: i64,
    max_entries: usize,
) -> PollOutcome {
    let Ok(fetched) = fetcher.get(sitemap_url).await else {
        return PollOutcome::default();
    };
    let entries = crate::sitemap::extract_entries(&fetched.body, max_entries);

    let mut out = PollOutcome::default();
    for entry in entries {
        let verdict = visits
            .apply_sitemap(&entry.url, entry.lastmod, now, trust)
            .await;
        match verdict {
            SitemapVerdict::Changed => {
                out.changed += 1;
                // Reset means nothing unless the page is actually scheduled somewhere. Put it into
                // the due set at now, reconstructing the minimal Pending a revisit needs. `depth`
                // is not recoverable from a sitemap, so 1 — a revisit of a known page is not a new
                // discovery and its depth no longer gates anything.
                let Ok(u) = url::Url::parse(&entry.url) else {
                    continue;
                };
                let host = u.host_str().unwrap_or_default().to_string();
                if host.is_empty() {
                    continue;
                }
                let pending = Pending {
                    url: frontier::canonical(&u),
                    host,
                    source_id: "sitemap".into(),
                    depth: 1,
                    trust,
                    channel: xustive_core::DiscoveryChannel::Sitemap,
                    priority: frontier::priority_for(1, trust, true),
                };
                frontier.defer(&pending, now.saturating_mul(1000)).await;
            }
            SitemapVerdict::Unchanged => out.unchanged += 1,
            SitemapVerdict::Unknown => out.unknown += 1,
        }
    }
    out
}
