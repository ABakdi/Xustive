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
    /// The dead-letter queue for this stream. Shares the same connection manager.
    pub fn dead_letter(&self) -> Queue {
        Queue {
            manager: self.manager.clone(),
            stream: format!("{}:dead", self.stream),
            group: self.group.clone(),
            max_len: self.max_len,
        }
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
        Ok(self
            .peek_dead_with_ids(count)
            .await?
            .into_iter()
            .map(|(_, letter)| letter)
            .collect())
    }

    /// Read dead letters with their stream entry ids — the handle the per-item actions
    /// ([`Queue::replay_dead_one`], [`Queue::drop_dead`]) need to address one letter.
    pub async fn peek_dead_with_ids(
        &self,
        count: usize,
    ) -> Result<Vec<(String, DeadLetter)>, QueueError> {
        use redis::AsyncCommands;
        let dead = self.dead_letter();
        let mut conn = self.conn().await?;
        let reply: redis::streams::StreamRangeReply =
            conn.xrevrange_count(&dead.stream, "+", "-", count).await?;

        Ok(reply
            .ids
            .into_iter()
            .filter_map(|entry| {
                let raw: String = entry.get("payload")?;
                let letter: DeadLetter = serde_json::from_str(&raw).ok()?;
                Some((entry.id, letter))
            })
            .collect())
    }

    /// Fetch one dead letter by its stream entry id.
    async fn dead_by_id(&self, entry_id: &str) -> Result<Option<DeadLetter>, QueueError> {
        use redis::AsyncCommands;
        let dead = self.dead_letter();
        let mut conn = self.conn().await?;
        let reply: redis::streams::StreamRangeReply =
            conn.xrange(&dead.stream, entry_id, entry_id).await?;
        Ok(reply.ids.into_iter().find_map(|entry| {
            let raw: String = entry.get("payload")?;
            serde_json::from_str(&raw).ok()
        }))
    }

    /// Put one dead letter back on the main queue and remove it from the dead stream. Returns
    /// `false` when no letter has that id (already replayed, dropped, or a stale row on the page).
    ///
    /// Re-enqueue first, delete second — the same ordering argument as [`Queue::dead_letter_job`]:
    /// this can duplicate a job on a crash between the two, which is recoverable; the other order
    /// can lose it, which is not.
    pub async fn replay_dead_one(&self, entry_id: &str) -> Result<bool, QueueError> {
        let Some(letter) = self.dead_by_id(entry_id).await? else {
            return Ok(false);
        };
        self.produce(&letter.payload).await?;
        use redis::AsyncCommands;
        let mut conn = self.conn().await?;
        let _: i64 = conn.xdel(&self.dead_letter().stream, &[entry_id]).await?;
        Ok(true)
    }

    /// Delete one dead letter without replaying it. Returns `false` when no letter has that id.
    ///
    /// This is the only place a dead letter is discarded on purpose — for a job whose cause is
    /// understood and whose payload is not wanted back (a permanently gone page, a malformed
    /// document). Everything else keeps the evidence.
    pub async fn drop_dead(&self, entry_id: &str) -> Result<bool, QueueError> {
        use redis::AsyncCommands;
        let mut conn = self.conn().await?;
        let removed: i64 = conn.xdel(&self.dead_letter().stream, &[entry_id]).await?;
        Ok(removed > 0)
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
            let mut conn = self.conn().await?;
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
