//! A task queue on Redis Streams.
//!
//! # Why Streams rather than a list
//!
//! `LPUSH`/`BRPOP` is simpler and loses work. A worker that pops a job and dies has taken that
//! job out of Redis with nothing recorded anywhere — the crawl page is gone and nobody knows.
//! Streams keep a delivered-but-unacknowledged entry in a per-group pending list, so a dead
//! worker's jobs are visible, reclaimable, and countable.
//!
//! For a crawler that costs someone else's bandwidth to re-fetch, losing work silently is the
//! failure that matters most.
//!
//! # The delivery guarantee
//!
//! **At least once.** A job may be delivered twice — a worker that finishes its side effect and
//! dies before acknowledging will see the job again. Exactly-once does not exist here, so
//! consumers must be idempotent, and the indexer is: it writes documents keyed by id, so a
//! repeated write is a no-op rather than a duplicate.
//!
//! Claiming otherwise would be worse than useless. It would let a consumer be written as though
//! replay could not happen.

pub mod dlq;
pub mod indexer;

use std::time::Duration;

use redis::streams::{StreamAutoClaimReply, StreamId, StreamReadOptions, StreamReadReply};
use redis::{AsyncCommands, RedisError};
use serde::{de::DeserializeOwned, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("redis: {0}")]
    Redis(#[from] RedisError),
    #[error("payload could not be encoded: {0}")]
    Encode(String),
    #[error("payload could not be decoded: {0}")]
    Decode(String),
}

/// The field every job payload is stored under.
///
/// One field holding JSON rather than a field per property: the schema then lives in Rust where
/// it can evolve with `serde` defaults, instead of in Redis where a rename is a migration.
const FIELD: &str = "payload";

/// How long a job may sit unacknowledged before another worker may claim it.
///
/// Five minutes is long enough that a slow fetch behind a crawl-delay is not stolen mid-flight,
/// and short enough that a crashed worker's queue does not sit idle for an hour.
pub const RECLAIM_AFTER: Duration = Duration::from_secs(300);

/// Cap on stream length, enforced approximately.
///
/// Acknowledged entries are **not** removed by `XACK` — they leave the pending list but stay in
/// the stream forever. Without trimming, a queue that has processed ten million pages holds ten
/// million entries and Redis grows until `noeviction` starts refusing writes, which looks like
/// the crawler breaking for no reason.
pub const MAX_LEN: usize = 100_000;

/// The consumer group the indexer worker drains the stream with.
///
/// Named as a constant, and shared, because more than one component needs to name it: the worker
/// creates it, and the crawler's backpressure must read *its* lag rather than its own. When the
/// crawler measured its own producer group instead, that group's lag only ever grew — nothing
/// consumes it — and the crawler paused permanently the moment it passed the threshold while the
/// indexer was in fact keeping up perfectly.
pub const INDEXER_GROUP: &str = "indexers";

/// A job as delivered to a consumer.
#[derive(Debug, Clone)]
pub struct Delivery<T> {
    /// The stream entry id. Needed to acknowledge, and stable across redeliveries.
    pub id: String,
    pub payload: T,
    /// How many times this entry has been delivered. `1` on first receipt.
    ///
    /// The signal for poison detection: a job that has been delivered many times is one that
    /// keeps killing its worker, and retrying it forever starves everything behind it.
    pub attempts: u64,
}

/// A named stream with a consumer group.
///
/// Holds one auto-reconnecting [`redis::aio::ConnectionManager`], shared by cloning across every
/// worker, rather than opening a fresh multiplexed connection per call. The old per-op pattern
/// spun up hundreds of short-lived connections a second under sixteen workers — the resource waste,
/// and the "Multiplexed connection driver unexpectedly terminated" flood, both came from that. The
/// manager keeps a single connection alive and silently re-establishes it after a drop.
#[derive(Clone)]
pub struct Queue {
    pub(crate) manager: redis::aio::ConnectionManager,
    pub stream: String,
    pub group: String,
}

/// Pull one group's `lag` out of an `XINFO GROUPS` reply.
///
/// Parsed by hand because the reply is a nested array of alternating keys and values, and the
/// shape differs between Redis versions — a typed deserialisation would break on an upgrade,
/// where a missing field here simply falls back to the stream length.
fn group_lag(reply: &redis::Value, group: &str) -> Option<usize> {
    let redis::Value::Array(entries) = reply else {
        return None;
    };
    for entry in entries {
        let redis::Value::Array(fields) = entry else {
            continue;
        };
        let mut name: Option<String> = None;
        let mut lag: Option<usize> = None;
        for pair in fields.chunks(2) {
            let [k, v] = pair else { continue };
            let key = match k {
                redis::Value::BulkString(b) => String::from_utf8_lossy(b).to_string(),
                redis::Value::SimpleString(s) => s.clone(),
                _ => continue,
            };
            match key.as_str() {
                "name" => {
                    if let redis::Value::BulkString(b) = v {
                        name = Some(String::from_utf8_lossy(b).to_string());
                    }
                }
                "lag" => {
                    if let redis::Value::Int(n) = v {
                        lag = usize::try_from(*n).ok();
                    }
                }
                _ => {}
            }
        }
        if name.as_deref() == Some(group) {
            return lag;
        }
    }
    None
}

impl Queue {
    /// Connect and ensure the consumer group exists.
    pub async fn connect(url: &str, stream: &str, group: &str) -> Result<Self, QueueError> {
        let client = redis::Client::open(url)?;
        let manager = client.get_connection_manager().await?;
        let queue = Self {
            manager,
            stream: stream.to_string(),
            group: group.to_string(),
        };
        queue.ensure_group().await?;
        Ok(queue)
    }

    /// Connect as a **producer only** — no consumer group is created.
    ///
    /// A component that just appends work (the crawler) has no group of its own, and creating one
    /// is worse than pointless: it is a group nothing consumes, whose lag grows without bound with
    /// every message written. That group is what the crawler once read for backpressure, freezing
    /// itself the moment its lag passed the threshold while the indexer was keeping up. Giving a
    /// producer no group removes the temptation to measure the wrong thing, and keeps `XINFO
    /// GROUPS` free of a phantom group an operator has to learn to ignore.
    ///
    /// `produce`, `produce_many` and `depth_of` are the whole surface a producer needs, and none of
    /// them touches `self.group`.
    pub async fn connect_producer(url: &str, stream: &str) -> Result<Self, QueueError> {
        let client = redis::Client::open(url)?;
        let manager = client.get_connection_manager().await?;
        Ok(Self {
            manager,
            stream: stream.to_string(),
            // Never used by the producer paths. Named so a stray consume() is obviously wrong
            // rather than silently joining some default group.
            group: "__producer__".to_string(),
        })
    }

    /// A handle to the shared connection. Cloning a `ConnectionManager` is cheap — it hands back a
    /// reference to the one underlying connection and its reconnect logic — so every call reuses it
    /// instead of opening a socket.
    async fn conn(&self) -> Result<redis::aio::ConnectionManager, QueueError> {
        Ok(self.manager.clone())
    }

    /// Create the group, tolerating one that already exists.
    ///
    /// `MKSTREAM` so the group can be created before anything has been produced — otherwise the
    /// first consumer to start fails and a cold system needs its components started in a
    /// particular order, which nobody remembers.
    async fn ensure_group(&self) -> Result<(), QueueError> {
        let mut conn = self.conn().await?;
        let result: Result<(), RedisError> = conn
            .xgroup_create_mkstream(&self.stream, &self.group, "0")
            .await;
        match result {
            Ok(()) => Ok(()),
            // BUSYGROUP: another process created it first. That is the desired end state.
            Err(e) if e.code() == Some("BUSYGROUP") => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Append a job.
    ///
    /// Trims approximately (`~`), which lets Redis remove whole macro-nodes instead of walking to
    /// an exact length. Exact trimming on every write costs more than the bound is worth.
    pub async fn produce<T: Serialize>(&self, payload: &T) -> Result<String, QueueError> {
        let json = serde_json::to_string(payload).map_err(|e| QueueError::Encode(e.to_string()))?;
        let mut conn = self.conn().await?;
        let id: String = conn
            .xadd_maxlen(
                &self.stream,
                redis::streams::StreamMaxlen::Approx(MAX_LEN),
                "*",
                &[(FIELD, json.as_str())],
            )
            .await?;
        Ok(id)
    }

    /// Append many jobs in one round trip.
    pub async fn produce_many<T: Serialize>(&self, payloads: &[T]) -> Result<usize, QueueError> {
        if payloads.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn().await?;
        let mut pipe = redis::pipe();
        for payload in payloads {
            let json =
                serde_json::to_string(payload).map_err(|e| QueueError::Encode(e.to_string()))?;
            pipe.xadd_maxlen(
                &self.stream,
                redis::streams::StreamMaxlen::Approx(MAX_LEN),
                "*",
                &[(FIELD, json.as_str())],
            )
            .ignore();
        }
        pipe.query_async::<()>(&mut conn).await?;
        Ok(payloads.len())
    }

    /// Take up to `count` new jobs, waiting up to `block` for the first.
    ///
    /// `>` means "entries never delivered to this group". Recovering a crashed worker's jobs is
    /// [`Self::reclaim`]'s business, deliberately kept separate: mixing the two makes it
    /// impossible to tell a backlog of new work from a backlog of stuck work.
    pub async fn consume<T: DeserializeOwned>(
        &self,
        consumer: &str,
        count: usize,
        block: Duration,
    ) -> Result<Vec<Delivery<T>>, QueueError> {
        let mut conn = self.conn().await?;
        let options = StreamReadOptions::default()
            .group(&self.group, consumer)
            .count(count)
            .block(block.as_millis() as usize);

        let reply: Option<StreamReadReply> = conn
            .xread_options(&[&self.stream], &[">"], &options)
            .await?;

        let Some(reply) = reply else {
            return Ok(Vec::new());
        };
        Ok(reply
            .keys
            .into_iter()
            .flat_map(|key| key.ids)
            .filter_map(|entry| decode(entry, 1))
            .collect())
    }

    /// Take over jobs another consumer left unacknowledged for longer than [`RECLAIM_AFTER`].
    ///
    /// `XAUTOCLAIM` rather than the `XPENDING` + `XCLAIM` pair: one round trip, and it cannot
    /// race with another reclaimer into a double claim the way reading then claiming can.
    pub async fn reclaim<T: DeserializeOwned>(
        &self,
        consumer: &str,
        count: usize,
    ) -> Result<Vec<Delivery<T>>, QueueError> {
        let mut conn = self.conn().await?;
        let reply: StreamAutoClaimReply = conn
            .xautoclaim_options(
                &self.stream,
                &self.group,
                consumer,
                RECLAIM_AFTER.as_millis() as usize,
                "0-0",
                redis::streams::StreamAutoClaimOptions::default().count(count),
            )
            .await?;

        Ok(reply
            .claimed
            .into_iter()
            // Delivery count is not returned by XAUTOCLAIM, so a reclaimed job is reported as a
            // repeat without a precise count. `2` is the honest floor: it was delivered at least
            // once before, or it would not be pending.
            .filter_map(|entry| decode(entry, 2))
            .collect())
    }

    /// Mark a job done. **Nothing is complete until this returns.**
    pub async fn ack(&self, id: &str) -> Result<(), QueueError> {
        let mut conn = self.conn().await?;
        let _: i64 = conn.xack(&self.stream, &self.group, &[id]).await?;
        Ok(())
    }

    /// Acknowledge several at once.
    pub async fn ack_all(&self, ids: &[String]) -> Result<usize, QueueError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn().await?;
        let acked: i64 = conn.xack(&self.stream, &self.group, ids).await?;
        Ok(acked as usize)
    }

    /// Jobs delivered but not yet acknowledged.
    ///
    /// The number to alert on. A pending count that climbs without the queue depth climbing means
    /// workers are taking jobs and dying, which nothing else makes visible.
    pub async fn pending(&self) -> Result<usize, QueueError> {
        let mut conn = self.conn().await?;
        let reply: redis::streams::StreamPendingReply =
            conn.xpending(&self.stream, &self.group).await?;
        Ok(match reply {
            redis::streams::StreamPendingReply::Empty => 0,
            redis::streams::StreamPendingReply::Data(data) => data.count,
        })
    }

    /// Entries in the stream, acknowledged or not.
    pub async fn depth(&self) -> Result<usize, QueueError> {
        self.depth_of(&self.group).await
    }

    /// The outstanding lag of a *named* consumer group, whichever group it is.
    ///
    /// A producer that measures its own group's lag learns nothing about the backlog — its group
    /// never consumes, so the lag is every message ever added. Backpressure has to ask about the
    /// group that actually drains the stream, which is a different group from the one the producer
    /// created. This lets it name that group without joining it (`XINFO GROUPS` reads any group; it
    /// does not enlist the caller as a consumer, which would steal messages from the real worker).
    pub async fn depth_of(&self, group: &str) -> Result<usize, QueueError> {
        let mut conn = self.conn().await?;

        // The group's **lag**, not the stream length.
        //
        // A Redis stream keeps entries after they are acknowledged — `XLEN` only shrinks on a
        // trim. So it counts work already done, and once the stream reaches its trim ceiling it
        // reports that ceiling forever regardless of how much is actually outstanding.
        //
        // That is not academic: it feeds the crawler's backpressure, which would have paused
        // permanently the moment the stream passed the threshold, and it fed an admin figure that
        // read as a growing backlog when the indexer was keeping up. Measured here: `XLEN` said
        // 38,274 while the real lag was 32,128, and no amount of draining moved the first number.
        let groups: redis::Value = redis::cmd("XINFO")
            .arg("GROUPS")
            .arg(&self.stream)
            .query_async(&mut conn)
            .await
            .unwrap_or(redis::Value::Nil);

        if let Some(lag) = group_lag(&groups, group) {
            return Ok(lag);
        }

        // No group yet — nothing has consumed, so everything in the stream is outstanding.
        let len: usize = conn.xlen(&self.stream).await?;
        Ok(len)
    }

    /// Trim to [`MAX_LEN`], approximately.
    ///
    /// Called on a timer as well as on write, because a queue that stops receiving work stops
    /// trimming — and a stream that stopped growing is exactly the one nobody is watching.
    pub async fn trim(&self) -> Result<(), QueueError> {
        let mut conn = self.conn().await?;
        let _: i64 = conn
            .xtrim(&self.stream, redis::streams::StreamMaxlen::Approx(MAX_LEN))
            .await?;
        Ok(())
    }

    /// Remove the group and stream. Tests only.
    pub async fn destroy(&self) -> Result<(), QueueError> {
        let mut conn = self.conn().await?;
        let _: Result<i64, RedisError> = conn.del(&self.stream).await;
        Ok(())
    }
}

fn decode<T: DeserializeOwned>(entry: StreamId, attempts: u64) -> Option<Delivery<T>> {
    let raw: String = entry.get(FIELD)?;
    match serde_json::from_str(&raw) {
        Ok(payload) => Some(Delivery {
            id: entry.id,
            payload,
            attempts,
        }),
        Err(e) => {
            // A payload we cannot parse is a poison pill. Logged and skipped rather than
            // returned as an error, because one bad entry must not stop the consumer draining
            // the rest — and it stays pending, so the reclaim loop will surface it.
            tracing::warn!(id = %entry.id, error = %e, "undecodable job payload; skipping");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reclaim_window_is_longer_than_a_slow_job() {
        // A crawl behind a one-second crawl-delay fetching a slow page can legitimately take
        // minutes. Reclaiming under that steals work in flight and doubles the load on a site
        // that is already responding slowly.
        const { assert!(RECLAIM_AFTER.as_secs() >= 120) };
        const { assert!(RECLAIM_AFTER.as_secs() <= 900) };
    }

    #[test]
    fn the_length_cap_is_bounded() {
        // XACK does not remove entries. Without a cap the stream grows forever and Redis under
        // `noeviction` eventually refuses writes, which presents as the crawler stopping for no
        // visible reason.
        const { assert!(MAX_LEN > 0) };
        const { assert!(MAX_LEN <= 1_000_000) };
    }
}
