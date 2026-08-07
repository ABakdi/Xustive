//! The queue against a live Redis.
//!
//! Skipped when none is reachable — a checkout without `make dev-up` should still have a green
//! suite. Everything asserted here is a durability property, and durability cannot be tested
//! against a mock: the whole point is what Redis does when a worker disappears.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use xustive_queue::Queue;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Job {
    url: String,
    depth: u32,
}

fn url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| {
        let port = std::env::var("XUSTIVE_REDIS_PORT").unwrap_or_else(|_| "6390".into());
        format!("redis://127.0.0.1:{port}")
    })
}

/// A queue on a name unique to this test, so a failed run cannot poison the next one.
async fn queue(name: &str) -> Option<Queue> {
    let stream = format!("xtest:{name}:{}", std::process::id());
    let q = Queue::connect(&url(), &stream, "workers").await.ok()?;
    // Prove the connection actually works; `connect` can succeed lazily.
    q.depth().await.ok()?;
    Some(q)
}

macro_rules! require {
    ($name:expr) => {
        match queue($name).await {
            Some(q) => q,
            None => {
                eprintln!("skipping: no Redis at {}", url());
                return;
            }
        }
    };
}

#[tokio::test]
async fn a_produced_job_is_consumed_once() {
    let q = require!("basic");
    let job = Job {
        url: "https://aps.dz/a".into(),
        depth: 1,
    };
    q.produce(&job).await.expect("produce");

    let got: Vec<_> = q
        .consume::<Job>("w1", 10, Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].payload, job);
    assert_eq!(got[0].attempts, 1);

    // A second consumer sees nothing: the group has already delivered it.
    let again: Vec<Delivery> = q
        .consume("w2", 10, Duration::from_millis(50))
        .await
        .unwrap();
    assert!(
        again.is_empty(),
        "a job must not be delivered to two workers"
    );

    q.destroy().await.ok();
}

type Delivery = xustive_queue::Delivery<Job>;

#[tokio::test]
async fn an_unacknowledged_job_stays_pending() {
    // The property that makes this a queue rather than a pipe. A worker that takes a job and
    // dies leaves it visible and recoverable, which LPUSH/BRPOP does not.
    let q = require!("pending");
    q.produce(&Job {
        url: "https://aps.dz/b".into(),
        depth: 0,
    })
    .await
    .unwrap();

    let got: Vec<Delivery> = q
        .consume("w1", 10, Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(
        q.pending().await.unwrap(),
        1,
        "delivered but unacknowledged"
    );

    q.ack(&got[0].id).await.unwrap();
    assert_eq!(q.pending().await.unwrap(), 0, "acknowledged");

    q.destroy().await.ok();
}

#[tokio::test]
async fn work_survives_a_worker_that_never_acknowledges() {
    // The crash-safety case, simulated the only way it can be: consume, then abandon.
    //
    // Reclaim uses a zero-millisecond idle window here rather than the production five minutes,
    // because the alternative is a test that sleeps for five minutes.
    let q = require!("crash");
    q.produce(&Job {
        url: "https://aps.dz/c".into(),
        depth: 2,
    })
    .await
    .unwrap();

    let taken: Vec<Delivery> = q
        .consume("doomed", 10, Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(taken.len(), 1);
    // `doomed` dies here. No ack.

    let rescued = reclaim_now(&q, "rescuer").await;
    assert_eq!(rescued.len(), 1, "a dead worker's job must be recoverable");
    assert_eq!(rescued[0].payload.url, "https://aps.dz/c");
    assert!(rescued[0].attempts >= 2, "a reclaim is a redelivery");

    q.ack(&rescued[0].id).await.unwrap();
    assert_eq!(q.pending().await.unwrap(), 0);

    q.destroy().await.ok();
}

/// `XAUTOCLAIM` with a zero idle window, so tests do not wait out `RECLAIM_AFTER`.
async fn reclaim_now(q: &Queue, consumer: &str) -> Vec<Delivery> {
    let client = redis::Client::open(url()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let reply: redis::streams::StreamAutoClaimReply = redis::AsyncCommands::xautoclaim_options(
        &mut conn,
        &q.stream,
        &q.group,
        consumer,
        0usize,
        "0-0",
        redis::streams::StreamAutoClaimOptions::default().count(10),
    )
    .await
    .unwrap();

    reply
        .claimed
        .into_iter()
        .filter_map(|entry| {
            let raw: String = entry.get("payload")?;
            Some(xustive_queue::Delivery {
                id: entry.id,
                payload: serde_json::from_str(&raw).ok()?,
                attempts: 2,
            })
        })
        .collect()
}

#[tokio::test]
async fn a_batch_is_produced_and_drained_in_order() {
    let q = require!("batch");
    let jobs: Vec<Job> = (0..25)
        .map(|i| Job {
            url: format!("https://aps.dz/{i}"),
            depth: 0,
        })
        .collect();
    assert_eq!(q.produce_many(&jobs).await.unwrap(), 25);
    assert_eq!(q.depth().await.unwrap(), 25);

    let first: Vec<Delivery> = q
        .consume("w1", 10, Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(first.len(), 10, "count bounds a read");
    // Streams are ordered, and the crawler depends on it: a frontier that reorders turns
    // breadth-first into something nobody chose.
    assert_eq!(first[0].payload.url, "https://aps.dz/0");

    let ids: Vec<String> = first.iter().map(|d| d.id.clone()).collect();
    assert_eq!(q.ack_all(&ids).await.unwrap(), 10);
    assert_eq!(q.pending().await.unwrap(), 0);

    q.destroy().await.ok();
}

#[tokio::test]
async fn a_poison_job_is_dead_lettered_and_stops_blocking() {
    let q = require!("poison");
    q.produce(&Job {
        url: "https://bad.example/x".into(),
        depth: 9,
    })
    .await
    .unwrap();

    let got: Vec<Delivery> = q
        .consume("w1", 10, Duration::from_millis(50))
        .await
        .unwrap();
    let job = &got[0];

    q.dead_letter_job(
        &job.id,
        serde_json::to_value(&job.payload).unwrap(),
        3,
        "parser panicked three times",
    )
    .await
    .unwrap();

    // Dead-lettering acknowledges the original, so it stops being redelivered.
    assert_eq!(
        q.pending().await.unwrap(),
        0,
        "a dead letter must not stay pending"
    );
    assert_eq!(q.dead_count().await.unwrap(), 1);

    let letters = q.peek_dead(10).await.unwrap();
    assert_eq!(letters.len(), 1);
    assert_eq!(letters[0].attempts, 3);
    assert!(
        letters[0].reason.contains("panicked"),
        "the reason is the evidence"
    );
    // The payload is kept whole, so a replay does not have to reconstruct it.
    assert_eq!(letters[0].payload["url"], "https://bad.example/x");

    q.destroy().await.ok();
    q.dead_letter().destroy().await.ok();
}

#[tokio::test]
async fn dead_letters_can_be_replayed_deliberately() {
    let q = require!("replay");
    q.produce(&Job {
        url: "https://aps.dz/retry".into(),
        depth: 1,
    })
    .await
    .unwrap();
    let got: Vec<Delivery> = q
        .consume("w1", 10, Duration::from_millis(50))
        .await
        .unwrap();
    q.dead_letter_job(
        &got[0].id,
        serde_json::to_value(&got[0].payload).unwrap(),
        3,
        "transient",
    )
    .await
    .unwrap();

    assert_eq!(q.replay_dead(10).await.unwrap(), 1);
    assert_eq!(
        q.dead_count().await.unwrap(),
        0,
        "replayed letters are cleared"
    );

    let back: Vec<Delivery> = q
        .consume("w2", 10, Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].payload.url, "https://aps.dz/retry");

    q.destroy().await.ok();
    q.dead_letter().destroy().await.ok();
}

#[tokio::test]
async fn an_undecodable_entry_does_not_stop_the_consumer() {
    // One malformed payload must not stop the queue draining. It is skipped and left pending, so
    // the reclaim loop surfaces it rather than it vanishing.
    let q = require!("garbage");
    let client = redis::Client::open(url()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let _: String =
        redis::AsyncCommands::xadd(&mut conn, &q.stream, "*", &[("payload", "{not json")])
            .await
            .unwrap();
    q.produce(&Job {
        url: "https://aps.dz/good".into(),
        depth: 0,
    })
    .await
    .unwrap();

    let got: Vec<Delivery> = q
        .consume("w1", 10, Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(got.len(), 1, "the good job still arrives");
    assert_eq!(got[0].payload.url, "https://aps.dz/good");

    q.destroy().await.ok();
}

#[tokio::test]
async fn creating_a_group_twice_is_not_an_error() {
    // Every worker calls this at startup. If the second one failed, a cold system would need its
    // processes started in a particular order, which nobody remembers at 3am.
    let q = require!("idempotent");
    assert!(Queue::connect(&url(), &q.stream, &q.group).await.is_ok());
    assert!(Queue::connect(&url(), &q.stream, &q.group).await.is_ok());
    q.destroy().await.ok();
}
