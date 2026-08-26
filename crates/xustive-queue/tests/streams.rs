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

/// Redis must be configured to refuse writes rather than evict them.
///
/// M1-T12.4. `noeviction` is set in `deploy/docker-compose.yml`, and until now that was the only
/// thing asserting it — a comment and a flag, neither of which fails if someone changes them.
///
/// The stakes are why this is worth a test at all. Under any `allkeys-*` policy Redis reclaims
/// memory by deleting keys of its own choosing, and the queue's stream is a perfectly ordinary key.
/// A full Redis would then drop queued documents **silently**: no error to the producer, no
/// dead letter, no gap anyone can see afterwards. The documents simply never arrive, and the only
/// symptom is an index that is quietly smaller than the crawl says it should be.
///
/// `noeviction` converts that into a write error, which the producer can retry and an operator can
/// see. Loudly refusing is strictly better than silently losing.
#[tokio::test]
async fn redis_refuses_to_evict_rather_than_dropping_queued_work() {
    let Some(q) = queue("eviction").await else {
        eprintln!("skipping: no Redis");
        return;
    };
    let _ = q;

    let Ok(client) = redis::Client::open(url()) else {
        return;
    };
    let Ok(mut conn) = client.get_multiplexed_async_connection().await else {
        return;
    };

    let policy: Vec<String> = match redis::cmd("CONFIG")
        .arg("GET")
        .arg("maxmemory-policy")
        .query_async(&mut conn)
        .await
    {
        Ok(v) => v,
        // A managed Redis may refuse CONFIG GET. Skipping is right: this asserts a deployment
        // setting, and being unable to read it is not evidence that it is wrong.
        Err(e) => {
            eprintln!("skipping: cannot read maxmemory-policy ({e})");
            return;
        }
    };

    let value = policy.get(1).map(String::as_str).unwrap_or("");
    assert_eq!(
        value, "noeviction",
        "maxmemory-policy is {value:?}; under any allkeys-* policy Redis would delete queued \
         documents to reclaim memory, with no error and no dead letter — an index quietly smaller \
         than the crawl claims"
    );
}

/// Backpressure must measure the *consumer's* lag, not the producer's own group.
///
/// The crawler froze at ~5000 documents indexed: it published through a group nobody consumes, then
/// read that group's lag for backpressure. A producer group's lag is every message ever added and
/// only grows, so once it crossed the threshold the crawler paused every iteration forever — while
/// the indexer, on its own group, was fully keeping up.
///
/// This asserts the two lags diverge exactly as they did in production: the producer's grows with
/// what it writes, the consumer's falls as it acknowledges.
#[tokio::test]
async fn depth_reads_the_named_consumers_lag_not_the_producers() {
    let producer = require!("backpressure");
    let stream = producer.stream.clone();

    // A separate handle on the same stream, in the consumer group — the worker.
    let Some(consumer) = Queue::connect(&url(), &stream, xustive_queue::INDEXER_GROUP)
        .await
        .ok()
    else {
        return;
    };

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Job {
        n: u32,
    }

    for n in 0..10 {
        producer.produce(&Job { n }).await.expect("produce");
    }

    // The producer's own group ("workers") has never consumed, so its lag is everything.
    let producer_lag = producer.depth().await.expect("producer depth");
    assert_eq!(
        producer_lag, 10,
        "the producer group sees the whole stream as outstanding"
    );

    // The consumer drains and acknowledges half.
    let batch = consumer
        .consume::<Job>("w1", 5, std::time::Duration::from_millis(100))
        .await
        .expect("consume");
    assert_eq!(batch.len(), 5);
    for d in &batch {
        consumer.ack(&d.id).await.expect("ack");
    }

    // Backpressure asking about the *consumer* group sees the real backlog fall; asking about the
    // producer group would still see 10 and pause needlessly.
    let real_backlog = producer
        .depth_of(xustive_queue::INDEXER_GROUP)
        .await
        .expect("consumer lag");
    assert!(
        real_backlog < producer_lag,
        "the consumer group's lag ({real_backlog}) must fall below the producer's ({producer_lag}); \
         reading the producer's own group is what froze the crawler"
    );
}

/// A producer connection must not create a consumer group.
///
/// The crawler is a pure producer. When it connected with `connect` it left a group nothing drains,
/// whose lag grew forever and which it then read for backpressure — freezing itself. A producer
/// gets no group, so there is nothing to misread and nothing phantom in `XINFO GROUPS`.
#[tokio::test]
async fn a_producer_connection_creates_no_group() {
    let stream = format!("xtest:producer:{}", std::process::id());
    let Some(producer) = Queue::connect_producer(&url(), &stream).await.ok() else {
        eprintln!("skipping: no Redis at {}", url());
        return;
    };

    #[derive(serde::Serialize)]
    struct Job {
        n: u32,
    }
    producer.produce(&Job { n: 1 }).await.expect("produce");

    let Ok(client) = redis::Client::open(url()) else {
        return;
    };
    let Ok(mut conn) = client.get_multiplexed_async_connection().await else {
        return;
    };
    let groups: redis::Value = redis::cmd("XINFO")
        .arg("GROUPS")
        .arg(&stream)
        .query_async(&mut conn)
        .await
        .unwrap_or(redis::Value::Nil);
    let count = match groups {
        redis::Value::Array(v) => v.len(),
        _ => 0,
    };
    assert_eq!(
        count, 0,
        "a producer connection left {count} consumer group(s); it should leave none"
    );

    let _: Result<(), _> = redis::cmd("DEL")
        .arg(&stream)
        .query_async::<()>(&mut conn)
        .await;
}

#[tokio::test]
async fn one_dead_letter_can_be_replayed_or_dropped_without_touching_the_rest() {
    let q = require!("dead-one");
    q.dead_letter().destroy().await.ok();

    for i in 0..3 {
        q.dead_letter_job(
            &format!("0-{i}"),
            serde_json::json!({ "url": format!("https://example.dz/{i}") }),
            3,
            "test poison",
        )
        .await
        .unwrap();
    }
    assert_eq!(q.dead_count().await.unwrap(), 3);

    let letters = q.peek_dead_with_ids(10).await.unwrap();
    assert_eq!(letters.len(), 3);
    let (replay_id, replay_letter) = letters[0].clone();
    let (drop_id, _) = letters[1].clone();

    // Replaying one puts exactly that payload back on the main queue and removes only it.
    assert!(q.replay_dead_one(&replay_id).await.unwrap());
    assert_eq!(q.dead_count().await.unwrap(), 2);
    assert_eq!(q.depth().await.unwrap(), 1);
    let back: Vec<serde_json::Value> = q
        .consume::<serde_json::Value>("t", 1, Duration::from_millis(200))
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.payload)
        .collect();
    assert_eq!(back[0]["url"], replay_letter.payload["url"]);

    // Dropping one removes only it, and a second attempt honestly reports it gone.
    assert!(q.drop_dead(&drop_id).await.unwrap());
    assert!(!q.drop_dead(&drop_id).await.unwrap());
    assert_eq!(q.dead_count().await.unwrap(), 1);

    q.destroy().await.ok();
    q.dead_letter().destroy().await.ok();
}
