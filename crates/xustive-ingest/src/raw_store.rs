//! Raw fetched-body storage with a TTL (M2-T04.7).
//!
//! Keeps the raw HTML of a fetch for a while, so extraction can be re-run without re-fetching the
//! page. The point is **reindexing**: when the parser improves — a new per-domain rule, a fixed
//! date selector — the operator can re-run it over what was already fetched, instead of paying the
//! sites' bandwidth again to collect bytes we already had.
//!
//! # Off by default, and why
//!
//! Storing every fetched body is expensive: at a 10 MB cap over millions of pages it dwarfs the
//! index itself, and the development Redis is a 1 GB `noeviction` instance shared with the frontier
//! and the queue. Filling it would refuse the writes the crawl actually depends on. So this is
//! opt-in — the operator turns it on when there is somewhere to put it, and the real home is object
//! storage, a decision the docs already pre-identify ([[Task Queue]] §12). Until then this is the
//! interim store, bounded by a per-blob size cap and a TTL so it cannot grow without limit.
//!
//! Best-effort throughout: a store that cannot be written is a lost reindex convenience, never a
//! lost document, so every failure is silent.

use std::time::Duration;

/// A blob larger than this is not stored. A page past the fetch cap should not exist, but a cap
/// here too means a single pathological body cannot claim a slice of a small Redis on its own.
const MAX_BLOB_BYTES: usize = 5 * 1024 * 1024;

/// Raw bodies in Redis, each with an expiry.
#[derive(Clone)]
pub struct RawStore {
    client: redis::Client,
    namespace: String,
    ttl: Duration,
}

impl RawStore {
    /// Connect with a TTL for stored bodies. Lazy — no connection until first use.
    pub fn connect_in(url: &str, namespace: &str, ttl: Duration) -> Option<Self> {
        Some(Self {
            client: redis::Client::open(url).ok()?,
            namespace: namespace.to_string(),
            ttl,
        })
    }

    async fn conn(&self) -> Option<redis::aio::MultiplexedConnection> {
        self.client.get_multiplexed_async_connection().await.ok()
    }

    fn key(&self, url: &str) -> String {
        format!("{}:raw:{url}", self.namespace)
    }

    /// Store a fetched body under its URL, with the configured TTL. Oversized bodies are skipped.
    pub async fn put(&self, url: &str, body: &str) {
        if body.len() > MAX_BLOB_BYTES {
            return;
        }
        let Some(mut conn) = self.conn().await else {
            return;
        };
        // `SET key body EX ttl` — the expiry is set atomically with the write, so a blob can never
        // be stored without one and outlive its window.
        let _: Result<(), _> = redis::cmd("SET")
            .arg(self.key(url))
            .arg(body)
            .arg("EX")
            .arg(self.ttl.as_secs().max(1))
            .query_async::<()>(&mut conn)
            .await;
    }

    /// Retrieve a stored body, for a reindex. `None` once it has expired or was never stored.
    pub async fn get(&self, url: &str) -> Option<String> {
        let mut conn = self.conn().await?;
        redis::cmd("GET")
            .arg(self.key(url))
            .query_async(&mut conn)
            .await
            .ok()
            .flatten()
    }

    /// Drop a stored body — for a takedown, so a removed page's bytes do not linger.
    pub async fn forget(&self, url: &str) {
        let Some(mut conn) = self.conn().await else {
            return;
        };
        let _: Result<(), _> = redis::cmd("DEL")
            .arg(self.key(url))
            .query_async::<()>(&mut conn)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_oversized_body_is_not_stored_and_a_dead_redis_is_silent() {
        // Against a port nothing listens on: put and get must not panic, get returns None.
        let s =
            RawStore::connect_in("redis://127.0.0.1:1", "test", Duration::from_secs(60)).unwrap();
        // Oversized: returns early before any connection is attempted.
        let big = "x".repeat(MAX_BLOB_BYTES + 1);
        s.put("https://e.dz/a", &big).await;
        // Normal size against a dead Redis: silent, and get finds nothing.
        s.put("https://e.dz/b", "<html>small</html>").await;
        assert!(s.get("https://e.dz/b").await.is_none());
    }
}
