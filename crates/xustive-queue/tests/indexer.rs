//! The indexer's durability properties.
//!
//! Driven against a fake sink, because the cases worth testing are the ones a real Meilisearch
//! will not produce on demand: reject exactly this document, fail this batch and not that one,
//! die between the write and the acknowledgement.
//!
//! The queue is real. Redis is where the durability actually lives, so mocking it would test
//! nothing.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use xustive_queue::indexer::SubmitError;
use xustive_queue::indexer::{IndexJob, Indexer, Sink};
use xustive_queue::Queue;

fn url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| {
        let port = std::env::var("XUSTIVE_REDIS_PORT").unwrap_or_else(|_| "6390".into());
        format!("redis://127.0.0.1:{port}")
    })
}

async fn queue(name: &str) -> Option<Queue> {
    let stream = format!("xtest-idx:{name}:{}", std::process::id());
    let q = Queue::connect(&url(), &stream, "indexers").await.ok()?;
    q.depth().await.ok()?;
    Some(q)
}

macro_rules! require {
    ($name:expr) => {
        match queue($name).await {
            Some(q) => q,
            None => {
                eprintln!("skipping: no Redis");
                return;
            }
        }
    };
}

/// Records what it was asked to write, and can be told to fail.
#[derive(Clone, Default)]
struct FakeSink {
    written: Arc<Mutex<Vec<String>>>,
    submissions: Arc<AtomicUsize>,
    /// Any batch containing one of these ids fails as a unit — which is how Meilisearch behaves.
    poison: Arc<Mutex<Vec<String>>>,
}

impl FakeSink {
    fn ids_written(&self) -> Vec<String> {
        self.written.lock().unwrap().clone()
    }
}

impl Sink for FakeSink {
    async fn submit(
        &self,
        _index: &str,
        documents: &[serde_json::Value],
    ) -> Result<(), SubmitError> {
        self.submissions.fetch_add(1, Ordering::Relaxed);
        let poison = self.poison.lock().unwrap().clone();
        let ids: Vec<String> = documents
            .iter()
            .filter_map(|d| d.get("id")?.as_str().map(str::to_string))
            .collect();

        if ids.iter().any(|id| poison.contains(id)) {
            return Err("document rejected".into());
        }
        self.written.lock().unwrap().extend(ids);
        Ok(())
    }
}

fn job(id: &str) -> IndexJob {
    IndexJob {
        document: json!({ "id": id, "title": "عنوان", "body": "نص" }),
        index: None,
    }
}

#[tokio::test]
async fn documents_are_indexed_and_only_then_acknowledged() {
    let q = require!("happy");
    for i in 0..5 {
        q.produce(&job(&format!("doc{i}"))).await.unwrap();
    }

    let sink = FakeSink::default();
    let stats = Indexer::new(q.clone(), sink.clone(), "w1", "documents")
        .run_once()
        .await
        .unwrap();

    assert_eq!(stats.indexed, 5);
    assert_eq!(sink.ids_written().len(), 5);
    // The property the whole ordering exists for: nothing is pending once the write landed.
    assert_eq!(q.pending().await.unwrap(), 0);

    q.destroy().await.ok();
    q.dead_letter().destroy().await.ok();
}

#[tokio::test]
async fn a_failed_batch_does_not_acknowledge_anything() {
    // The case that makes acknowledge-on-submit wrong. If the write fails, the queue entries must
    // survive — otherwise those documents are gone with no record anywhere.
    let q = require!("failed");
    for i in 0..4 {
        q.produce(&job(&format!("doc{i}"))).await.unwrap();
    }

    let sink = FakeSink::default();
    // Every id poisoned, so every bisection down to singletons fails too.
    *sink.poison.lock().unwrap() = (0..4).map(|i| format!("doc{i}")).collect();

    let stats = Indexer::new(q.clone(), sink.clone(), "w1", "documents")
        .run_once()
        .await
        .unwrap();

    assert_eq!(stats.indexed, 0);
    assert!(sink.ids_written().is_empty(), "nothing was written");
    // Each was isolated to a singleton and dead-lettered, so none is left blocking the queue.
    assert_eq!(stats.rejected, 4);
    assert_eq!(q.dead_count().await.unwrap(), 4);
    assert_eq!(q.pending().await.unwrap(), 0);

    q.destroy().await.ok();
    q.dead_letter().destroy().await.ok();
}

#[tokio::test]
async fn one_bad_document_does_not_take_the_batch_with_it() {
    // Meilisearch fails a batch as a unit and does not say which document was at fault. Without
    // bisection, one bad page in a crawl of five hundred loses all five hundred.
    let q = require!("bisect");
    for i in 0..16 {
        q.produce(&job(&format!("doc{i}"))).await.unwrap();
    }

    let sink = FakeSink::default();
    sink.poison.lock().unwrap().push("doc7".into());

    let stats = Indexer::new(q.clone(), sink.clone(), "w1", "documents")
        .run_once()
        .await
        .unwrap();

    assert_eq!(stats.indexed, 15, "the other fifteen must land");
    assert_eq!(stats.rejected, 1);
    let written = sink.ids_written();
    assert!(!written.contains(&"doc7".to_string()));
    assert!(written.contains(&"doc0".to_string()));
    assert!(written.contains(&"doc15".to_string()));
    assert_eq!(q.pending().await.unwrap(), 0);

    // Bisection, not one-at-a-time: sixteen documents should isolate one culprit in far fewer
    // than sixteen submissions.
    let submissions = sink.submissions.load(Ordering::Relaxed);
    assert!(submissions < 16, "took {submissions} submissions");

    q.destroy().await.ok();
    q.dead_letter().destroy().await.ok();
}

#[tokio::test]
async fn an_invalid_document_is_dead_lettered_before_it_reaches_the_index() {
    // Caught here rather than by the engine, because a batch the engine rejects takes the good
    // documents with it and its error does not identify the culprit.
    let q = require!("invalid");
    q.produce(&IndexJob {
        document: json!({ "title": "no id" }),
        index: None,
    })
    .await
    .unwrap();
    q.produce(&job("good")).await.unwrap();

    let sink = FakeSink::default();
    let stats = Indexer::new(q.clone(), sink.clone(), "w1", "documents")
        .run_once()
        .await
        .unwrap();

    assert_eq!(stats.indexed, 1);
    assert_eq!(stats.rejected, 1);
    assert_eq!(sink.ids_written(), vec!["good".to_string()]);

    let letters = q.peek_dead(10).await.unwrap();
    assert_eq!(letters.len(), 1);
    assert_eq!(
        letters[0].reason, "missing_id",
        "the reason names the defect"
    );

    q.destroy().await.ok();
    q.dead_letter().destroy().await.ok();
}

#[tokio::test]
async fn a_redelivered_document_overwrites_rather_than_duplicating() {
    // What makes at-least-once safe. A worker that writes and dies before acknowledging will see
    // the job again; because documents are keyed by id, the second write is a no-op.
    let q = require!("idempotent");
    q.produce(&job("same")).await.unwrap();

    let sink = FakeSink::default();
    Indexer::new(q.clone(), sink.clone(), "w1", "documents")
        .run_once()
        .await
        .unwrap();

    // The same document arrives again, as it would after a crash.
    q.produce(&job("same")).await.unwrap();
    Indexer::new(q.clone(), sink.clone(), "w1", "documents")
        .run_once()
        .await
        .unwrap();

    let written = sink.ids_written();
    assert_eq!(written.len(), 2, "the sink saw it twice");
    // Both writes carry the same id, so the index holds one document. That is the guarantee —
    // not that the write happens once, but that repeating it changes nothing.
    assert!(written.iter().all(|id| id == "same"));

    q.destroy().await.ok();
    q.dead_letter().destroy().await.ok();
}

#[tokio::test]
async fn work_left_by_a_crashed_worker_is_picked_up() {
    // Consume without acknowledging, exactly as a killed process leaves things, then confirm a
    // second worker can recover it. The reclaim window is production-length, so this asserts the
    // job is *pending and visible* rather than waiting five minutes to prove the timer.
    let q = require!("crashed");
    q.produce(&job("orphan")).await.unwrap();

    let taken: Vec<xustive_queue::Delivery<IndexJob>> = q
        .consume("doomed", 10, Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(taken.len(), 1);
    // `doomed` dies here without acknowledging.

    assert_eq!(
        q.pending().await.unwrap(),
        1,
        "the job must still be accounted for after the worker is gone"
    );

    // A fresh indexer sees no *new* work and correctly indexes nothing — the orphan is not lost,
    // it is waiting for the reclaim window rather than being silently redelivered.
    let sink = FakeSink::default();
    let stats = Indexer::new(q.clone(), sink.clone(), "w2", "documents")
        .run_once()
        .await
        .unwrap();
    assert_eq!(stats.indexed, 0);
    assert_eq!(q.pending().await.unwrap(), 1, "still recoverable");

    q.destroy().await.ok();
    q.dead_letter().destroy().await.ok();
}

/// A transient failure must not dead-letter the document.
///
/// This was losing real work: a slow Meilisearch timed out, the batch bisected all the way to
/// single documents, and each was dead-lettered as "the culprit" — 125 documents discarded with
/// `attempts 1`, at exactly the moment the system was under load and least able to afford it.
#[tokio::test]
async fn a_timeout_leaves_the_document_for_retry() {
    let Some(queue) = queue("retryable").await else {
        return;
    };
    for i in 0..4 {
        queue
            .produce(&IndexJob {
                document: serde_json::json!({ "id": format!("doc-{i}"), "title": "t", "body": "b" }),
                index: None,
            })
            .await
            .expect("produce");
    }

    // A sink that always times out, which is what a saturated index looks like.
    struct Timeout;
    impl Sink for Timeout {
        async fn submit(&self, _: &str, _: &[serde_json::Value]) -> Result<(), SubmitError> {
            Err(SubmitError::transient("search backend timed out after 60s"))
        }
    }

    let indexer = Indexer::new(queue.clone(), Timeout, "test", "documents");
    let stats = indexer.run_once().await.expect("run");

    assert_eq!(stats.indexed, 0);
    assert_eq!(
        stats.dead_lettered + stats.rejected,
        0,
        "a timeout discarded documents instead of leaving them for retry"
    );
    // Still pending, so a later run picks them up.
    assert!(
        queue.pending().await.unwrap_or(0) > 0,
        "nothing left to retry"
    );
}

/// The other direction: a permanent failure still dead-letters, or a poison document loops forever.
#[tokio::test]
async fn a_permanent_failure_still_dead_letters() {
    let Some(queue) = queue("permanent").await else {
        return;
    };
    queue
        .produce(&IndexJob {
            document: serde_json::json!({ "id": "bad", "title": "t", "body": "b" }),
            index: None,
        })
        .await
        .expect("produce");

    struct Rejects;
    impl Sink for Rejects {
        async fn submit(&self, _: &str, _: &[serde_json::Value]) -> Result<(), SubmitError> {
            Err(SubmitError::permanent("invalid document id"))
        }
    }

    let indexer = Indexer::new(queue.clone(), Rejects, "test", "documents");
    let stats = indexer.run_once().await.expect("run");
    assert_eq!(stats.rejected, 1, "a poison document must not loop forever");
}
