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

    loop {
        let stats = indexer.run_once().await?;
        if stats.indexed > 0 || stats.rejected > 0 {
            println!(
                "  indexed {} · rejected {} · dead-lettered {} · {} batches",
                stats.indexed, stats.rejected, stats.dead_lettered, stats.batches
            );
        }
        // Trimming here rather than only on write: a queue that stops receiving work stops
        // trimming itself, and a stream that stopped growing is the one nobody is watching.
        queue.trim().await.ok();

        if once {
            return Ok(());
        }
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
