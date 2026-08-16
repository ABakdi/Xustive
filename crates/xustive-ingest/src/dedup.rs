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
