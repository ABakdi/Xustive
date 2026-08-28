//! The endorse sink: the web's verdict, written onto our documents ([[ADR-0031]], M13-T01.3).
//!
//! Every federated response passes through here. For each hit the sink folds one sighting —
//! rank, SearXNG score, engines — into the document's `web` record and rewrites the flat
//! `endorsement` signal, as a *partial* update so the rest of the document is untouched. It
//! is the shape of the events writer: a bounded channel, one task, batches, one filtered read
//! per batch for the current values.
//!
//! Which ids it writes depends on whether the URL is already ours. An existing document —
//! crawled on our own, or eager-indexed earlier — is always updated: that is the whole point
//! (a page we hold that the web keeps returning should rank higher). A URL not yet in the
//! index is written only when eager indexing is on, because then a thin document for the same
//! id is on its way through the index queue and the two merge in either order; with eager
//! indexing off, a partial update would create an empty stub, so the sighting is skipped and
//! the crawl-feed carries the URL instead.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::mpsc;

use xustive_core::model::WebEndorsement;
use xustive_search::MeiliClient;

const QUEUE: usize = 4096;
const FLUSH_EVERY: Duration = Duration::from_secs(2);

/// One sighting of a URL in a federated response.
#[derive(Debug, Clone)]
pub struct Sighting {
    /// The document id (`id_for_url` of the canonical URL) — the same id the eager document
    /// and the crawl-feed use, so all three converge on one document.
    pub id: String,
    pub rank: u32,
    pub score: f32,
    pub engines: Vec<String>,
    /// Whether a missing id may be created (eager indexing on).
    pub create: bool,
}

#[derive(Clone)]
pub struct EndorseSink {
    tx: mpsc::Sender<Sighting>,
    written: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
}

impl EndorseSink {
    pub fn start(client: Arc<MeiliClient>) -> Self {
        let (tx, rx) = mpsc::channel(QUEUE);
        let written = Arc::new(AtomicU64::new(0));
        tokio::spawn(writer(client, rx, written.clone()));
        Self {
            tx,
            written,
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn record(&self, sightings: Vec<Sighting>) {
        for s in sightings {
            if self.tx.try_send(s).is_err() {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// How many flushes a batch survives a failed read before it is dropped. The read is a
/// filtered search, and the engine that is too busy to answer it is usually busy indexing a
/// crawl batch — a condition that clears in minutes, which is what the retries wait out.
const MAX_ATTEMPTS: u32 = 30;

async fn writer(
    client: Arc<MeiliClient>,
    mut rx: mpsc::Receiver<Sighting>,
    written: Arc<AtomicU64>,
) {
    let mut batch: Vec<Sighting> = Vec::new();
    let mut attempts: u32 = 0;
    let mut tick = tokio::time::interval(FLUSH_EVERY);
    loop {
        // Sightings accumulate; the tick writes them. One federated response is twenty-odd
        // sightings arriving together, and one read + one write per tick is the point.
        tokio::select! {
            job = rx.recv() => match job {
                Some(s) => { batch.push(s); continue; }
                None => break,
            },
            _ = tick.tick() => {}
        }
        if batch.is_empty() {
            continue;
        }
        // A failed read keeps the batch for a later tick, with a widening pause: the ticks
        // keep coming every two seconds, so the wait is counted in ticks skipped.
        if attempts > 0 && !tick_due(attempts) {
            attempts += 1;
            continue;
        }
        match flush(&client, &mut batch, &written).await {
            Ok(()) => attempts = 0,
            Err(()) => {
                attempts += 1;
                if attempts >= MAX_ATTEMPTS {
                    tracing::warn!(n = batch.len(), "endorse: batch dropped after retries");
                    batch.clear();
                    attempts = 0;
                }
            }
        }
    }
    let _ = flush(&client, &mut batch, &written).await;
}

/// Retry on the 2nd, 4th, 8th, 16th tick after a failure, then every sixteenth — a long
/// outage is polled every half minute, not hammered.
fn tick_due(attempts: u32) -> bool {
    attempts.is_power_of_two() || attempts % 16 == 0
}

/// Fold a batch of sightings into the documents they name and write the updates. `Err` means
/// the batch is intact and should be retried; `Ok` means it was written or had nothing to write.
async fn flush(
    client: &MeiliClient,
    batch: &mut Vec<Sighting>,
    written: &AtomicU64,
) -> Result<(), ()> {
    if batch.is_empty() {
        return Ok(());
    }
    let Ok(index) = client.resolve(xustive_search::settings::DOCUMENTS).await else {
        tracing::warn!("endorse: documents index unresolved; will retry");
        return Err(());
    };
    let sightings = std::mem::take(batch);
    // The current records, one filtered read for the batch.
    let mut ids: Vec<&str> = sightings.iter().map(|s| s.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    let quoted: Vec<String> = ids.iter().map(|i| format!("\"{i}\"")).collect();
    let q = xustive_search::Query::new("")
        .filter(format!("id IN [{}]", quoted.join(", ")))
        .limit(ids.len().max(1));
    let current: HashMap<String, WebEndorsement> = match client.search::<Value>(&index, &q).await {
        Ok(r) => r
            .hits
            .iter()
            .filter_map(|h| {
                let id = h.get("id")?.as_str()?.to_string();
                let web = h
                    .get("web")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                Some((id, web))
            })
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, n = sightings.len(), "endorse: read failed; will retry");
            *batch = sightings;
            return Err(());
        }
    };
    let now = xustive_core::now_unix();
    let n = sightings.len();
    let updates = fold(sightings, &current, now);
    if updates.is_empty() {
        return Ok(());
    }
    match client.update_documents(&index, &updates).await {
        Ok(_) => {
            written.fetch_add(updates.len() as u64, Ordering::Relaxed);
            tracing::info!(sightings = n, documents = updates.len(), "endorse: written");
            Ok(())
        }
        Err(e) => {
            // The engine took the read but not the write: nothing to retry from — the folded
            // values would double-count on a second pass — so this one is lost and said so.
            tracing::warn!(error = %e, "endorse: write failed; batch lost");
            Ok(())
        }
    }
}

/// Pure: the partial updates for a batch given the current records. Sightings of an id absent
/// from `current` are kept only when one of them may create it.
pub fn fold(
    sightings: Vec<Sighting>,
    current: &HashMap<String, WebEndorsement>,
    now: i64,
) -> Vec<Value> {
    let mut folded: HashMap<String, (WebEndorsement, bool)> = HashMap::new();
    for s in sightings {
        let entry = folded
            .entry(s.id.clone())
            .or_insert_with(|| (current.get(&s.id).cloned().unwrap_or_default(), false));
        entry.0.fold(s.rank, s.score, &s.engines, now);
        entry.1 |= s.create;
    }
    let mut out: Vec<Value> = folded
        .into_iter()
        .filter(|(id, (_, create))| *create || current.contains_key(id))
        .map(|(id, (web, _))| json!({ "id": id, "endorsement": web.signal(), "web": web }))
        .collect();
    out.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sighting(id: &str, rank: u32, create: bool) -> Sighting {
        Sighting {
            id: id.into(),
            rank,
            score: 1.0 / rank as f32,
            engines: vec!["duckduckgo".into()],
            create,
        }
    }

    #[test]
    fn an_existing_document_is_endorsed_and_a_missing_one_only_when_eager() {
        let mut current = HashMap::new();
        let mut seen_before = WebEndorsement::default();
        seen_before.fold(5, 0.2, &["bing".into()], 1_000);
        current.insert("ours".to_string(), seen_before);

        let out = fold(
            vec![
                sighting("ours", 2, false),
                sighting("new-no-eager", 1, false),
                sighting("new-eager", 3, true),
            ],
            &current,
            2_000,
        );
        let ids: Vec<&str> = out.iter().map(|u| u["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["new-eager", "ours"]);

        let ours = &out[1];
        assert_eq!(ours["web"]["seen"], 2);
        assert_eq!(ours["web"]["best_rank"], 2);
        assert_eq!(ours["web"]["first_seen_at"], 1_000);
        assert_eq!(ours["web"]["last_seen_at"], 2_000);
        let engines = ours["web"]["engines"].as_array().unwrap();
        assert_eq!(engines.len(), 2, "engines are a union");
        let signal = ours["endorsement"].as_f64().unwrap();
        assert!(
            signal > 0.45 && signal < 0.5,
            "seen twice, best rank 2: {signal}"
        );
    }

    #[test]
    fn two_sightings_of_one_id_in_a_batch_fold_once() {
        let mut current = HashMap::new();
        current.insert("a".to_string(), WebEndorsement::default());
        let out = fold(
            vec![sighting("a", 4, false), sighting("a", 1, false)],
            &current,
            1,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["web"]["seen"], 2);
        assert_eq!(out[0]["web"]["best_rank"], 1);
    }
}
