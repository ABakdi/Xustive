//! The indexer worker.
//!
//! Drains a queue of documents into Meilisearch in batches. The whole design is one property:
//! **nothing is acknowledged until it is durably in the index.**
//!
//! # The ordering that matters
//!
//! ```text
//!   consume  →  validate  →  submit  →  poll task to completion  →  ack
//! ```
//!
//! Acknowledging on submit would be faster and wrong. Meilisearch accepts a batch and returns a
//! task id immediately; the write happens afterwards and can fail. A worker that acknowledged on
//! submit and then saw the task fail has lost those documents with no record — the queue entry is
//! gone and the index never got them.
//!
//! The cost of the correct order is that a crash between the write landing and the acknowledgement
//! redelivers the batch. That is safe because indexing is keyed by document id, so a repeated
//! write is a no-op. **At-least-once plus idempotence**, which is achievable, rather than
//! exactly-once, which is not.
//!
//! # Split on failure
//!
//! Meilisearch fails a batch as a unit. One malformed document in five hundred rejects all five
//! hundred, and retrying the same batch fails identically forever.
//!
//! So a failed batch is bisected: split, retry each half, recurse. A single bad document is
//! isolated in about nine attempts for a batch of five hundred, and the other 499 land. Retrying
//! whole would have dropped all of them or blocked the queue.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::{Delivery, Queue, QueueError};

/// Documents per submission.
///
/// Every batch costs one `add_documents` + `wait_task` round-trip, so the drain rate under a
/// backlog is (batch size) / (per-task latency) — a bigger batch amortises that fixed latency
/// over more documents. Meilisearch indexes in a single writer, but it searches concurrently
/// with indexing, so a larger batch drains faster without making queries wait. The
/// `MAX_BATCH_BYTES` cap below is the real guard against an oversized hold: a batch of long
/// articles trips it well before this count. A thousand is the assert ceiling and the sweet
/// spot for short documents, where the byte cap never binds.
pub const MAX_BATCH: usize = 1000;

/// Bytes per submission. A batch of long articles hits this well before the count.
pub const MAX_BATCH_BYTES: usize = 4 * 1024 * 1024;

/// How long to wait for a partial batch before submitting anyway.
///
/// Without it, the last few documents of a crawl sit unindexed until the next crawl starts —
/// which on a small site is never.
pub const BATCH_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to wait for Meilisearch to finish a task before giving up.
pub const TASK_TIMEOUT: Duration = Duration::from_secs(120);

/// A document as it arrives on the queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexJob {
    pub document: serde_json::Value,
    /// Which index it belongs in. Carried per job so one queue can feed documents and comments.
    #[serde(default)]
    pub index: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Stats {
    pub indexed: usize,
    pub rejected: usize,
    pub dead_lettered: usize,
    pub batches: usize,
}

/// Why a document was refused before it ever reached the index.
///
/// Checked here rather than left to Meilisearch because a batch rejected by the engine takes 499
/// good documents with it, and because the engine's error message does not say which document
/// was at fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invalid {
    /// No `id`. The primary key; without it a write is not idempotent and a redelivery would
    /// duplicate rather than overwrite — which is the property the whole design rests on.
    MissingId,
    NotAnObject,
    /// Larger than Meilisearch will accept. Better refused with a reason than failing a batch.
    TooLarge,
    /// No `title` *and* no `body`. A document with neither is not searchable, so indexing it
    /// only inflates the count.
    Empty,
}

impl Invalid {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingId => "missing_id",
            Self::NotAnObject => "not_an_object",
            Self::TooLarge => "too_large",
            Self::Empty => "empty",
        }
    }
}

/// Per-document size ceiling.
const MAX_DOC_BYTES: usize = 1024 * 1024;

pub fn validate(document: &serde_json::Value) -> Result<(), Invalid> {
    let Some(object) = document.as_object() else {
        return Err(Invalid::NotAnObject);
    };
    let has_id = object
        .get("id")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    if !has_id {
        return Err(Invalid::MissingId);
    }

    let non_empty = |key: &str| {
        object
            .get(key)
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.trim().is_empty())
    };
    if !non_empty("title") && !non_empty("body") {
        return Err(Invalid::Empty);
    }

    if serde_json::to_string(document)
        .map(|s| s.len())
        .unwrap_or(0)
        > MAX_DOC_BYTES
    {
        return Err(Invalid::TooLarge);
    }
    Ok(())
}

/// What the worker needs from a search backend.
///
/// A trait so the worker's ordering and split-on-failure logic can be tested without a live
/// Meilisearch — those are the parts most worth testing and the hardest to provoke against a real
/// engine, since making it fail a specific batch on demand is not something it offers.
/// Why a submission failed.
///
/// Typed rather than a string, because the indexer has to tell one case from the other and
/// nothing else can: a **transient** failure is left for retry, a **permanent** one is
/// dead-lettered. When this was a `String` every failure looked permanent, so a slow index
/// discarded real documents.
#[derive(Debug, Clone)]
pub struct SubmitError {
    pub message: String,
    /// True when retrying could succeed — a timeout, an unreachable backend, a 5xx.
    pub retryable: bool,
}

impl SubmitError {
    pub fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }

    pub fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }
}

impl From<&str> for SubmitError {
    /// Defaults to **permanent**, matching the old `String` behaviour so an implementor that has
    /// not thought about it keeps the semantics it had — dead-lettering a document that should
    /// have been retried is visible in the DLQ, where retrying one that never can would loop.
    fn from(message: &str) -> Self {
        Self::permanent(message)
    }
}

impl std::fmt::Display for SubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub trait Sink: Send + Sync {
    /// Submit and wait for the write to be durable. An error means nothing landed.
    fn submit(
        &self,
        index: &str,
        documents: &[serde_json::Value],
    ) -> impl std::future::Future<Output = Result<(), SubmitError>> + Send;
}

pub struct Indexer<S: Sink> {
    queue: Queue,
    sink: S,
    consumer: String,
    default_index: String,
}

impl<S: Sink> Indexer<S> {
    pub fn new(queue: Queue, sink: S, consumer: &str, default_index: &str) -> Self {
        Self {
            queue,
            sink,
            consumer: consumer.to_string(),
            default_index: default_index.to_string(),
        }
    }

    /// Drain everything currently queued, then return.
    ///
    /// One pass rather than a loop, so the caller owns the scheduling. A worker that owns its own
    /// forever-loop cannot be shut down cleanly or driven from a test.
    pub async fn run_once(&self) -> Result<Stats, QueueError> {
        let mut stats = Stats::default();

        // Reclaim before consuming. A crashed worker's jobs are older than anything new and
        // should not wait behind a fresh backlog.
        let reclaimed: Vec<Delivery<IndexJob>> =
            self.queue.reclaim(&self.consumer, MAX_BATCH).await?;
        if !reclaimed.is_empty() {
            tracing::info!(
                count = reclaimed.len(),
                "reclaimed jobs from a stalled worker"
            );
            self.process(reclaimed, &mut stats).await?;
        }

        loop {
            let batch: Vec<Delivery<IndexJob>> = self
                .queue
                .consume(&self.consumer, MAX_BATCH, BATCH_TIMEOUT)
                .await?;
            if batch.is_empty() {
                return Ok(stats);
            }
            self.process(batch, &mut stats).await?;
        }
    }

    async fn process(
        &self,
        batch: Vec<Delivery<IndexJob>>,
        stats: &mut Stats,
    ) -> Result<(), QueueError> {
        let mut valid: Vec<(String, serde_json::Value)> = Vec::with_capacity(batch.len());

        for delivery in batch {
            // A job redelivered past the limit is poison: it has killed a worker repeatedly and
            // retrying it forever starves everything behind it.
            if delivery.attempts > crate::dlq::MAX_ATTEMPTS {
                self.queue
                    .dead_letter_job(
                        &delivery.id,
                        serde_json::to_value(&delivery.payload).unwrap_or_default(),
                        delivery.attempts,
                        "exceeded delivery attempts",
                    )
                    .await?;
                stats.dead_lettered += 1;
                continue;
            }

            match validate(&delivery.payload.document) {
                Ok(()) => valid.push((delivery.id, delivery.payload.document)),
                Err(reason) => {
                    // Refused documents are dead-lettered, not dropped. A document the crawler
                    // produced and the indexer refused is a bug in one of them, and silently
                    // discarding it means nobody ever finds out which.
                    self.queue
                        .dead_letter_job(
                            &delivery.id,
                            delivery.payload.document,
                            delivery.attempts,
                            reason.as_str(),
                        )
                        .await?;
                    stats.rejected += 1;
                }
            }
        }

        if valid.is_empty() {
            return Ok(());
        }

        for chunk in split_by_bytes(valid, MAX_BATCH_BYTES) {
            self.submit_chunk(chunk, stats).await?;
        }
        Ok(())
    }

    /// Submit, then acknowledge — never the other way round. Bisect on failure.
    async fn submit_chunk(
        &self,
        chunk: Vec<(String, serde_json::Value)>,
        stats: &mut Stats,
    ) -> Result<(), QueueError> {
        if chunk.is_empty() {
            return Ok(());
        }
        let documents: Vec<serde_json::Value> = chunk.iter().map(|(_, d)| d.clone()).collect();
        let started = Instant::now();

        match self.sink.submit(&self.default_index, &documents).await {
            Ok(()) => {
                // Only now. The write is durable, so the queue entries can go.
                let ids: Vec<String> = chunk.into_iter().map(|(id, _)| id).collect();
                stats.indexed += ids.len();
                stats.batches += 1;
                self.queue.ack_all(&ids).await?;
                tracing::debug!(
                    count = ids.len(),
                    ms = started.elapsed().as_millis() as u64,
                    "batch indexed"
                );
                Ok(())
            }
            Err(error) => {
                // A transient failure is not the document's fault.
                //
                // Left unacknowledged rather than dead-lettered, so it is reclaimed and retried.
                // Bisecting is pointless too — halving a batch does not make a busy index answer
                // faster, it just produces more timeouts.
                //
                // This was losing documents. A slow Meilisearch timed out, the batch bisected all
                // the way down to single documents, and each one was dead-lettered as "the
                // culprit" — 125 real documents discarded with `attempts 1`, at exactly the moment
                // the system was under load and least able to afford it.
                if error.retryable {
                    tracing::warn!(
                        count = chunk.len(),
                        %error,
                        "transient index failure; leaving for retry"
                    );
                    return Ok(());
                }

                // A single document cannot be bisected further, so it is the culprit.
                if chunk.len() == 1 {
                    let (id, document) = chunk.into_iter().next().expect("length checked");
                    self.queue
                        .dead_letter_job(&id, document, 1, &format!("rejected by index: {error}"))
                        .await?;
                    stats.rejected += 1;
                    return Ok(());
                }

                // Bisect. Meilisearch fails a batch as a unit and gives no indication which
                // document was at fault, so the only way to save the rest is to halve and retry.
                tracing::warn!(count = chunk.len(), %error, "batch failed; splitting");
                let mid = chunk.len() / 2;
                let mut left = chunk;
                let right = left.split_off(mid);
                Box::pin(self.submit_chunk(left, stats)).await?;
                Box::pin(self.submit_chunk(right, stats)).await?;
                Ok(())
            }
        }
    }
}

/// Split into chunks under a byte ceiling.
///
/// By serialised size, not document count. A batch of five hundred long articles is many times
/// the size of five hundred short ones, and the engine's limit is bytes.
fn split_by_bytes(
    items: Vec<(String, serde_json::Value)>,
    max_bytes: usize,
) -> Vec<Vec<(String, serde_json::Value)>> {
    let mut chunks = Vec::new();
    let mut current: Vec<(String, serde_json::Value)> = Vec::new();
    let mut bytes = 0usize;

    for (id, document) in items {
        let size = serde_json::to_string(&document)
            .map(|s| s.len())
            .unwrap_or(0);
        if !current.is_empty() && bytes + size > max_bytes {
            chunks.push(std::mem::take(&mut current));
            bytes = 0;
        }
        bytes += size;
        current.push((id, document));
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(id: &str) -> serde_json::Value {
        json!({ "id": id, "title": "عنوان", "body": "نص المقال" })
    }

    #[test]
    fn a_document_without_an_id_is_refused() {
        // The primary key. Without it a write is not idempotent, and a redelivery would duplicate
        // rather than overwrite — which is the property the whole at-least-once design rests on.
        assert_eq!(validate(&json!({"title": "x"})), Err(Invalid::MissingId));
        assert_eq!(
            validate(&json!({"id": "  ", "title": "x"})),
            Err(Invalid::MissingId)
        );
    }

    #[test]
    fn a_document_with_no_text_is_refused() {
        // Indexing it inflates the document count and makes the index look healthier than it is.
        assert_eq!(validate(&json!({"id": "a"})), Err(Invalid::Empty));
        assert_eq!(
            validate(&json!({"id": "a", "title": "", "body": "  "})),
            Err(Invalid::Empty)
        );
        assert!(
            validate(&json!({"id": "a", "body": "نص"})).is_ok(),
            "body alone is enough"
        );
    }

    #[test]
    fn an_oversized_document_is_refused_rather_than_failing_its_batch() {
        let big = json!({ "id": "a", "title": "t", "body": "x".repeat(MAX_DOC_BYTES + 10) });
        assert_eq!(validate(&big), Err(Invalid::TooLarge));
    }

    #[test]
    fn batches_are_split_by_serialised_size() {
        // A batch of long articles hits the byte ceiling long before the count, and the engine's
        // limit is bytes.
        let items: Vec<(String, serde_json::Value)> = (0..10)
            .map(|i| {
                (
                    i.to_string(),
                    json!({ "id": i.to_string(), "body": "x".repeat(1000) }),
                )
            })
            .collect();
        let chunks = split_by_bytes(items, 3000);
        assert!(chunks.len() > 1, "should have split");
        for chunk in &chunks {
            let bytes: usize = chunk
                .iter()
                .map(|(_, d)| serde_json::to_string(d).unwrap().len())
                .sum();
            // A single oversized document still forms a chunk of one — refusing to emit it would
            // silently drop it.
            assert!(bytes <= 3000 || chunk.len() == 1, "chunk of {bytes} bytes");
        }
    }

    #[test]
    fn splitting_preserves_every_item() {
        let items: Vec<(String, serde_json::Value)> = (0..25)
            .map(|i| (i.to_string(), doc(&i.to_string())))
            .collect();
        let total: usize = split_by_bytes(items, 200).iter().map(Vec::len).sum();
        assert_eq!(total, 25, "no document may be lost in splitting");
    }

    #[test]
    fn the_batch_size_keeps_the_index_responsive() {
        // Meilisearch indexes with a single writer. An oversized batch holds it and stalls
        // search for everyone.
        const { assert!(MAX_BATCH <= 1000) };
        const { assert!(MAX_BATCH >= 50) };
    }
}
