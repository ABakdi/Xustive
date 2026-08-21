//! `xustive-cli worker` — drain the index queue into Meilisearch.
//!
//! The [`Sink`] implementation is the only place the queue meets the search engine, and it is
//! deliberately thin: submit, wait for the task to finish, report success only when it did.
//! Everything interesting — batching, ordering, bisection — lives in [`xustive_queue::indexer`]
//! where it is testable without a live engine.

use anyhow::{Context, Result};
use xustive_core::Config;
use xustive_queue::indexer::SubmitError;
use xustive_queue::indexer::{Indexer, Sink};
use xustive_queue::Queue;
use xustive_search::MeiliClient;

/// Writes to Meilisearch and **waits for the write to be durable**.
///
/// The waiting is the whole point. Meilisearch returns a task id immediately and performs the
/// write afterwards; returning success on submission would let the worker acknowledge documents
/// that never landed.
/// Map a search error to the indexer's retry decision.
///
/// This is the classification the indexer depends on: a timeout or an unreachable backend is left
/// for retry, anything else is the document's fault and dead-lettered. Getting it wrong in the
/// permanent direction discards real documents, which is what happened before it existed.
fn classify(e: xustive_search::SearchError) -> SubmitError {
    use xustive_core::Classify;
    if e.is_retryable() {
        SubmitError::transient(e.to_string())
    } else {
        SubmitError::permanent(e.to_string())
    }
}

struct MeiliSink {
    client: MeiliClient,
}

impl Sink for MeiliSink {
    async fn submit(
        &self,
        index: &str,
        documents: &[serde_json::Value],
    ) -> Result<(), SubmitError> {
        let task = self
            .client
            .add_documents(index, documents)
            .await
            .map_err(classify)?;

        let status = self.client.wait_task(task).await.map_err(classify)?;

        if status.is_success() {
            Ok(())
        } else {
            // The engine's message names the failure but not the document. Bisection in the
            // indexer is what turns this into a usable diagnosis.
            // A task that ran and failed is the document's problem, not the backend's — retrying
            // it unchanged would fail identically.
            Err(SubmitError::permanent(status.error_message()))
        }
    }
}

pub async fn run(config: &Config, client: &MeiliClient, once: bool) -> Result<()> {
    let queue = Queue::connect(
        &config.queue.url,
        &config.queue.index_stream,
        xustive_queue::INDEXER_GROUP,
    )
    .await
    .with_context(|| format!("connecting to {}", config.queue.url))?;

    let consumer = format!("{}-{}", hostname(), std::process::id());
    let index = client.resolve(&config.search.documents_index).await?;
    // Ensure the write index is configured before indexing into it. Meilisearch auto-creates an
    // index on first write with **no** settings, and `resolve` then prefers that bare index over
    // the configured one — so a single premature write leaves search unable to filter, and every
    // search 500s. Applying the settings here is idempotent and runs once per start, so it costs
    // nothing on a healthy index and closes the door on the auto-created-unconfigured index for
    // good (the recurrence this has bitten us with before).
    client.ensure_index(&index, "id").await?;
    client
        .apply_settings(&index, &xustive_search::settings::documents_settings())
        .await
        .with_context(|| format!("configuring the write index {index}"))?;
    let indexer = Indexer::new(
        queue.clone(),
        MeiliSink {
            client: client.clone(),
        },
        &consumer,
        &index,
    );

    println!(
        "worker {consumer} draining {} → {index}",
        config.queue.index_stream
    );

    if once {
        let stats = indexer.run_once().await?;
        report(&stats);
        queue.trim().await.ok();
        return Ok(());
    }

    // Graceful shutdown (M4-T02.7): on SIGTERM/Ctrl-C stop taking new batches. A batch that is
    // mid-flight when the signal lands is abandoned *unacked*, so it is redelivered and reprocessed
    // on the next start (the indexer's reclaim path) — at-least-once, and `add_documents` is
    // idempotent by id, so a reprocess overwrites rather than duplicates. Every batch that finished
    // before the signal is already acked.
    let mut shutdown = std::pin::pin!(crate::shutdown::signal());
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                println!("worker: shutdown signal received; exiting");
                return Ok(());
            }
            result = indexer.run_once() => {
                let stats = result?;
                report(&stats);
                queue.trim().await.ok();
            }
        }
    }
}

fn report(stats: &xustive_queue::indexer::Stats) {
    if stats.indexed > 0 || stats.rejected > 0 {
        println!(
            "  indexed {} · rejected {} · dead-lettered {} · {} batches",
            stats.indexed, stats.rejected, stats.dead_lettered, stats.batches
        );
    }
}

/// Dead-letter inspection and replay.
pub async fn dlq(config: &Config, action: &str, limit: usize) -> Result<()> {
    let queue = Queue::connect(
        &config.queue.url,
        &config.queue.index_stream,
        xustive_queue::INDEXER_GROUP,
    )
    .await?;

    match action {
        "stats" => {
            println!("  queue depth    {}", queue.depth().await?);
            println!("  pending        {}", queue.pending().await?);
            println!("  dead letters   {}", queue.dead_count().await?);
        }
        "peek" => {
            let letters = queue.peek_dead(limit).await?;
            if letters.is_empty() {
                println!("  no dead letters");
            }
            for letter in letters {
                let id = letter
                    .payload
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("—");
                println!(
                    "  {id:26}  {:20}  attempts {}",
                    letter.reason, letter.attempts
                );
            }
        }
        "replay" => {
            // Always deliberate, never on a timer. A queue that retries its own poison
            // automatically will do it at 3am after someone fixed the bug and went to bed.
            let count = queue.replay_dead(limit).await?;
            println!("  replayed {count} dead letter(s) onto {}", queue.stream);
        }
        other => anyhow::bail!("unknown dlq action {other:?}; use stats, peek or replay"),
    }
    Ok(())
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "worker".into())
}
