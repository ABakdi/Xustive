//! The dead-letter queue.
//!
//! A job that keeps killing its worker is worse than a job that fails: it is redelivered
//! forever, and everything behind it waits. After [`MAX_ATTEMPTS`] deliveries it moves here,
//! where it stops blocking the queue and starts being evidence.
//!
//! Dead-lettering is **not** discarding. The entry keeps its payload, its attempt count and the
//! reason, because the thing that killed a worker three times is usually a bug worth reading —
//! a page shape the parser cannot handle, a document the index rejects.

use serde::{Deserialize, Serialize};

use crate::{Queue, QueueError};

/// Deliveries before a job is considered poison.
///
/// Three. Two would dead-letter jobs that hit one transient failure and one unlucky restart;
/// ten would mean a genuinely poisonous job blocks its queue for ten cycles first.
pub const MAX_ATTEMPTS: u64 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadLetter {
    /// The original stream id, so the entry can be traced back.
    pub original_id: String,
    /// The payload exactly as it was, so a replay does not have to reconstruct it.
    pub payload: serde_json::Value,
    pub attempts: u64,
    pub reason: String,
    pub failed_at: i64,
}

impl Queue {
    /// The dead-letter queue for this stream.
    pub fn dead_letter(&self) -> Queue {
        Queue {
            client: self.client_handle(),
            stream: format!("{}:dead", self.stream),
            group: self.group.clone(),
        }
    }

    pub(crate) fn client_handle(&self) -> redis::Client {
        self.client.clone()
    }

    /// Move a job to the dead-letter queue and acknowledge the original.
    ///
    /// Written before the acknowledgement on purpose. Acknowledging first and then failing to
    /// write leaves no record anywhere — the job is simply gone. This ordering can duplicate a
    /// dead letter, which is recoverable; the other cannot.
    pub async fn dead_letter_job(
        &self,
        id: &str,
        payload: serde_json::Value,
        attempts: u64,
        reason: &str,
    ) -> Result<(), QueueError> {
        let entry = DeadLetter {
            original_id: id.to_string(),
            payload,
            attempts,
            reason: reason.to_string(),
            failed_at: xustive_core::now_unix(),
        };
        self.dead_letter().produce(&entry).await?;
        self.ack(id).await?;
        tracing::warn!(stream = %self.stream, id, attempts, reason, "job dead-lettered");
        Ok(())
    }

    /// Read dead letters without consuming them.
    pub async fn peek_dead(&self, count: usize) -> Result<Vec<DeadLetter>, QueueError> {
        use redis::AsyncCommands;
        let dead = self.dead_letter();
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let reply: redis::streams::StreamRangeReply =
            conn.xrevrange_count(&dead.stream, "+", "-", count).await?;

        Ok(reply
            .ids
            .into_iter()
            .filter_map(|entry| {
                let raw: String = entry.get("payload")?;
                serde_json::from_str(&raw).ok()
            })
            .collect())
    }

    /// Put dead letters back on the main queue.
    ///
    /// Replay is always a deliberate act, never automatic. A queue that retries its own poison on
    /// a timer is a queue that will do it at three in the morning after somebody has fixed the
    /// bug and gone to bed.
    pub async fn replay_dead(&self, limit: usize) -> Result<usize, QueueError> {
        let letters = self.peek_dead(limit).await?;
        let payloads: Vec<serde_json::Value> = letters.iter().map(|l| l.payload.clone()).collect();
        let replayed = self.produce_many(&payloads).await?;

        if replayed > 0 {
            use redis::AsyncCommands;
            let mut conn = self.client.get_multiplexed_async_connection().await?;
            let _: Result<i64, redis::RedisError> = conn.del(&self.dead_letter().stream).await;
        }
        Ok(replayed)
    }

    pub async fn dead_count(&self) -> Result<usize, QueueError> {
        self.dead_letter().depth().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_attempt_limit_distinguishes_bad_luck_from_poison() {
        // Two would dead-letter a job that hit one transient failure and one unlucky restart.
        // Ten would let genuine poison block its queue for ten cycles first.
        const { assert!(MAX_ATTEMPTS >= 3) };
        const { assert!(MAX_ATTEMPTS <= 5) };
    }
}
