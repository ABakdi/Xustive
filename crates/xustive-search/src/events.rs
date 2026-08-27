//! Housekeeping on the `events` index ([[ADR-0030]], M11) that both the API and the CLI need:
//! the retention sweep and the right to be forgotten. Here rather than in the API so the
//! operator command does not have to link the whole serving crate.

use serde_json::Value;

use crate::settings::EVENTS;
use crate::{MeiliClient, Query};

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Delete every event carrying `visitor`; returns how many there were.
pub async fn forget_visitor(client: &MeiliClient, visitor: &str) -> Result<u64, String> {
    let index = client.resolve(EVENTS).await.map_err(|e| e.to_string())?;
    let filter = format!("visitor = \"{visitor}\"");
    let q = Query::new("").filter(filter.clone()).limit(1);
    let n = client
        .search::<Value>(&index, &q)
        .await
        .map_err(|e| e.to_string())?
        .estimated_total_hits;
    client
        .delete_by_filter(&index, &filter)
        .await
        .map_err(|e| e.to_string())?;
    Ok(n as u64)
}

/// Delete events older than `retention_days`; returns how many there were.
pub async fn sweep(client: &MeiliClient, retention_days: u64) -> Result<u64, String> {
    let index = client.resolve(EVENTS).await.map_err(|e| e.to_string())?;
    let cutoff = now() - retention_days as i64 * 86_400;
    let filter = format!("at < {cutoff}");
    let q = Query::new("").filter(filter.clone()).limit(1);
    let n = client
        .search::<Value>(&index, &q)
        .await
        .map_err(|e| e.to_string())?
        .estimated_total_hits;
    client
        .delete_by_filter(&index, &filter)
        .await
        .map_err(|e| e.to_string())?;
    Ok(n as u64)
}
