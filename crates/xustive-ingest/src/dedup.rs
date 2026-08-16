//! Cross-run content deduplication (M2-T05.2, M2-T05.7).
//!
//! The crawler already drops duplicates *within* a run with an in-memory set. This is the part that
//! survives a restart: a persistent record of which `content_hash` values have been indexed, so the
//! same article reached from a homepage today and a sitemap tomorrow is indexed once.
//!
//! `content_hash` is BLAKE3 over the extracted, normalised body ([[crate::revisit]]), so this
//! catches the same article at two URLs — a syndicated wire story, an article and its
//! print-friendly twin — which URL canonicalisation cannot, because the URLs genuinely differ.
//!
//! # Fail-open, deliberately (M2-T05.7)
//!
//! When Redis is unreachable, `is_new` returns `true` — treat the document as new and index it.
//! The alternative, treating an unknown as a duplicate, would make a Redis outage silently drop
//! real documents, and a dropped document is gone: the site was crawled, politely, and the result
//! thrown away. An accidental duplicate costs nothing, because the indexer writes keyed by id and a
//! repeated write is a no-op. So the failure mode is a few extra writes, never a hole in the index.
//!
//! This is the same fail-open discipline the robots cache uses, for the same reason: an
//! infrastructure wobble must degrade the crawl, not corrupt it.

/// One member of a duplicate set, with the facts winner selection needs (M2-T05.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    /// Publication time, unix seconds. Earlier is better — it is likely the original.
    pub published_at: i64,
    /// Whether the date is trusted. A guessed date must never beat a real one on "earliest",
    /// because the guess is usually the crawl time, which makes a copy look older than its source.
    pub date_trusted: bool,
    /// Source trust, 0–100. Breaks ties between same-dated copies.
    pub trust: u8,
    /// Extracted body length. The last tiebreak — a fuller version is the better record.
    pub body_len: usize,
    /// Engagement on this copy, summed across the set onto the winner.
    pub engagement: u64,
}

/// Pick which of a set of duplicates to keep, and the engagement to carry onto it (M2-T05.4).
///
/// The order is deliberate and each step earns its place:
///
/// 1. **A trusted date beats an untrusted one.** A copy whose date we guessed (usually the crawl
///    time) must not win "earliest" over the source whose real publication date we read — that
///    would crown the copy as the original.
/// 2. **Earliest published wins.** Among copies with comparably trustworthy dates, the first to
///    publish is the source; the rest are syndications of it.
/// 3. **Then highest trust**, then **longest body**, as tiebreaks — a fuller version from a source
///    we rate more is the better record of the same story.
///
/// Engagement is *aggregated*: the winner should reflect all the attention the story got, not just
/// the attention on the one copy that happened to win. Returns `None` for an empty set.
pub fn select_winner(candidates: &[Candidate]) -> Option<Winner> {
    let total_engagement: u64 = candidates.iter().map(|c| c.engagement).sum();
    let winner = candidates.iter().min_by(|a, b| {
        // `min_by`, so "less" means "better". Compare on each key in order, flipping the sense so
        // the better value sorts first.
        b.date_trusted
            .cmp(&a.date_trusted) // trusted (true) sorts before untrusted
            .then(a.published_at.cmp(&b.published_at)) // earlier sorts before later
            .then(b.trust.cmp(&a.trust)) // higher trust sorts before lower
            .then(b.body_len.cmp(&a.body_len)) // longer body sorts before shorter
            .then(a.id.cmp(&b.id)) // stable final tiebreak, so the result is deterministic
    })?;
    Some(Winner {
        id: winner.id.clone(),
        engagement: total_engagement,
    })
}

/// The chosen document and the engagement summed across its duplicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Winner {
    pub id: String,
    pub engagement: u64,
}

/// A persistent set of indexed content hashes, in Redis.
#[derive(Clone)]
pub struct Dedup {
    client: redis::Client,
    key: String,
}

impl Dedup {
    /// Connect. Lazy — the connection is not opened until first use, so construction cannot fail on
    /// a Redis that is merely slow to start.
    pub fn connect_in(url: &str, namespace: &str) -> Option<Self> {
        Some(Self {
            client: redis::Client::open(url).ok()?,
            key: format!("{namespace}:seen_hashes"),
        })
    }

    async fn conn(&self) -> Option<redis::aio::MultiplexedConnection> {
        self.client.get_multiplexed_async_connection().await.ok()
    }

    /// Record the hash and report whether it was **new** — never seen before this call.
    ///
    /// Atomic: `SADD` returns 1 when the member was added and 0 when it was already present, so the
    /// check and the record are one operation and two workers racing on the same hash cannot both
    /// see it as new.
    ///
    /// An empty hash is always new — it means the body was never hashed, which is not a duplicate of
    /// anything and must not collapse every unhashed document into one.
    ///
    /// Fails **open**: any Redis error yields `true`. See the module note — indexing a duplicate is
    /// free, dropping a document is not.
    pub async fn is_new(&self, content_hash: &str) -> bool {
        if content_hash.is_empty() {
            return true;
        }
        let Some(mut conn) = self.conn().await else {
            return true;
        };
        let added: Result<i64, _> = redis::cmd("SADD")
            .arg(&self.key)
            .arg(content_hash)
            .query_async(&mut conn)
            .await;
        match added {
            Ok(n) => n == 1,
            Err(_) => true,
        }
    }

    /// Forget a hash, so its document can be re-indexed. For a takedown or a forced reindex.
    pub async fn forget(&self, content_hash: &str) {
        let Some(mut conn) = self.conn().await else {
            return;
        };
        let _: Result<(), _> = redis::cmd("SREM")
            .arg(&self.key)
            .arg(content_hash)
            .query_async::<()>(&mut conn)
            .await;
    }

    /// How many distinct hashes are recorded.
    pub async fn len(&self) -> usize {
        let Some(mut conn) = self.conn().await else {
            return 0;
        };
        redis::cmd("SCARD")
            .arg(&self.key)
            .query_async(&mut conn)
            .await
            .unwrap_or(0)
    }

    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}

#[cfg(test)]
mod tests {
    // The Redis-backed behaviour is exercised in tests/dedup_redis.rs, which skips without Redis.
    // Here we only assert the one branch that needs no connection: an empty hash is always new,
    // because it means "not hashed", not "a duplicate of the last unhashed thing".
    use super::*;

    fn c(id: &str, published: i64, trusted: bool, trust: u8, len: usize, eng: u64) -> Candidate {
        Candidate {
            id: id.into(),
            published_at: published,
            date_trusted: trusted,
            trust,
            body_len: len,
            engagement: eng,
        }
    }

    #[test]
    fn the_earliest_trusted_date_wins() {
        let set = [
            c("late", 2000, true, 60, 100, 5),
            c("early", 1000, true, 60, 100, 3),
        ];
        assert_eq!(select_winner(&set).unwrap().id, "early");
    }

    #[test]
    fn a_trusted_date_beats_an_earlier_untrusted_one() {
        // The untrusted one looks earlier, but its date is a guess (the crawl time) — the copy must
        // not be crowned the original over the source whose real date we read.
        let set = [
            c("guessed", 500, false, 60, 100, 0),
            c("real", 1000, true, 60, 100, 0),
        ];
        assert_eq!(select_winner(&set).unwrap().id, "real");
    }

    #[test]
    fn trust_then_length_break_a_date_tie() {
        let same = 1000;
        assert_eq!(
            select_winner(&[
                c("a", same, true, 30, 100, 0),
                c("b", same, true, 90, 100, 0)
            ])
            .unwrap()
            .id,
            "b"
        );
        assert_eq!(
            select_winner(&[
                c("a", same, true, 60, 80, 0),
                c("b", same, true, 60, 200, 0)
            ])
            .unwrap()
            .id,
            "b"
        );
    }

    #[test]
    fn engagement_is_summed_onto_the_winner() {
        let set = [
            c("early", 1000, true, 60, 100, 4),
            c("copy1", 2000, true, 60, 100, 10),
            c("copy2", 3000, true, 60, 100, 6),
        ];
        let w = select_winner(&set).unwrap();
        assert_eq!(w.id, "early");
        assert_eq!(
            w.engagement, 20,
            "the winner carries all the attention the story got, not just its own"
        );
    }

    #[test]
    fn selection_is_deterministic_and_empty_is_none() {
        assert!(select_winner(&[]).is_none());
        let set = [
            c("a", 1000, true, 60, 100, 0),
            c("b", 1000, true, 60, 100, 0),
        ];
        assert_eq!(select_winner(&set), select_winner(&set));
    }

    #[tokio::test]
    async fn an_empty_hash_is_always_new_even_without_redis() {
        // A connection to a port nothing listens on: every Redis call fails, exercising fail-open.
        let d = Dedup::connect_in("redis://127.0.0.1:1", "test").unwrap();
        assert!(
            d.is_new("").await,
            "an empty hash is not a duplicate of anything"
        );
        // And a real hash against a dead Redis fails open to new, so the crawl is never blocked.
        assert!(
            d.is_new("b3:whatever").await,
            "an unreachable Redis must not make everything look like a duplicate"
        );
    }
}
