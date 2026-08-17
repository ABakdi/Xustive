//! Bandwidth accounting and cost ([[Proxy Manager]] §8, §5).
//!
//! Residential and mobile bandwidth is the largest variable cost in the system, and the number that
//! decides whether a source is worth collecting is its **cost per 1 000 documents**: a source that
//! fetches full pages with images through a residential proxy can cost a hundred times one served
//! by a JSON endpoint. So bytes are metered per pool and per source, and the monthly spend is
//! watched against a budget with an alert at 80 % — the point at which there is still time to turn
//! off the expensive pools before the budget is gone rather than after.
//!
//! The cost *math* is pure and lives here so it is testable; the running totals live in Redis, keyed
//! by month, so every crawler replica accumulates into the same figure.

/// Bytes in a gigabyte, for the per-GB cost model.
const BYTES_PER_GB: f64 = 1_073_741_824.0;

/// Cost per 1 000 documents, in the price unit of `price_per_gb`. `None` until the source has
/// produced a document — a divide-by-zero dressed up as "free" would be a lie the dashboard should
/// not tell. The figure that decides whether a source earns its bandwidth.
pub fn cost_per_1k_docs(bytes: u64, docs: u64, price_per_gb: f64) -> Option<f64> {
    (docs > 0).then(|| {
        let gb = bytes as f64 / BYTES_PER_GB;
        (gb * price_per_gb / docs as f64) * 1_000.0
    })
}

/// Average bytes per document — the lever behind residential spend (§8). `None` with no documents.
pub fn bytes_per_doc(bytes: u64, docs: u64) -> Option<f64> {
    (docs > 0).then(|| bytes as f64 / docs as f64)
}

/// Fraction of the monthly budget used, in `0.0..`. A `budget_gb` of zero means "no budget set",
/// reported as `0.0` rather than infinity so an unconfigured pool does not perpetually alert.
pub fn budget_fraction(bytes: u64, budget_gb: f64) -> f64 {
    if budget_gb <= 0.0 {
        return 0.0;
    }
    (bytes as f64 / BYTES_PER_GB) / budget_gb
}

/// The 80 % alert threshold (§5, §9). True once the pool has used at least this share of its budget.
pub const ALERT_AT: f64 = 0.80;

/// Whether a pool at `fraction` of its budget should raise the `BandwidthBudget80` alert.
pub fn over_budget_alert(fraction: f64) -> bool {
    fraction >= ALERT_AT
}

/// Per-month bandwidth counters in Redis. One hash per month, so a month rolls over into a fresh
/// figure and reset is a single expiry.
#[derive(Clone)]
pub struct BandwidthMeter {
    client: redis::Client,
    namespace: String,
}

/// What a source has cost this month.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SourceUsage {
    pub bytes: u64,
    pub docs: u64,
}

impl BandwidthMeter {
    pub fn connect_in(url: &str, namespace: &str) -> Option<Self> {
        Some(Self {
            client: redis::Client::open(url).ok()?,
            namespace: namespace.to_string(),
        })
    }

    async fn conn(&self) -> Option<redis::aio::MultiplexedConnection> {
        self.client.get_multiplexed_async_connection().await.ok()
    }

    fn key(&self, month: &str) -> String {
        format!("{}:bandwidth:{month}", self.namespace)
    }

    /// Record `bytes` transferred and `docs` produced by `source` over `pool`, in `month`
    /// (`"YYYY-MM"`). Best-effort: a lost increment is a slightly low cost figure, never a blocked
    /// crawl.
    pub async fn record(&self, month: &str, pool: &str, source: &str, bytes: u64, docs: u64) {
        let Some(mut conn) = self.conn().await else {
            return;
        };
        let key = self.key(month);
        let mut pipe = redis::pipe();
        pipe.cmd("HINCRBY")
            .arg(&key)
            .arg(format!("pool:{pool}:bytes"))
            .arg(bytes as i64)
            .ignore()
            .cmd("HINCRBY")
            .arg(&key)
            .arg(format!("src:{source}:bytes"))
            .arg(bytes as i64)
            .ignore()
            .cmd("HINCRBY")
            .arg(&key)
            .arg(format!("src:{source}:docs"))
            .arg(docs as i64)
            .ignore();
        let _: Result<(), _> = pipe.query_async::<()>(&mut conn).await;
    }

    /// Total bytes a pool transferred this month.
    pub async fn pool_bytes(&self, month: &str, pool: &str) -> u64 {
        self.field(month, &format!("pool:{pool}:bytes")).await
    }

    /// A source's bytes and documents this month, for its cost-per-1k figure.
    pub async fn source_usage(&self, month: &str, source: &str) -> SourceUsage {
        SourceUsage {
            bytes: self.field(month, &format!("src:{source}:bytes")).await,
            docs: self.field(month, &format!("src:{source}:docs")).await,
        }
    }

    async fn field(&self, month: &str, field: &str) -> u64 {
        let Some(mut conn) = self.conn().await else {
            return 0;
        };
        redis::cmd("HGET")
            .arg(self.key(month))
            .arg(field)
            .query_async::<Option<u64>>(&mut conn)
            .await
            .ok()
            .flatten()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_per_1k_docs_scales_with_bytes_and_price() {
        // 2 GB over 1 000 docs at $8/GB → $16 of bandwidth per 1 000 docs.
        let two_gb = (2.0 * BYTES_PER_GB) as u64;
        let cost = cost_per_1k_docs(two_gb, 1_000, 8.0).unwrap();
        assert!((cost - 16.0).abs() < 1e-6, "got {cost}");
        // Half the price, half the cost.
        assert!((cost_per_1k_docs(two_gb, 1_000, 4.0).unwrap() - 8.0).abs() < 1e-6);
        // No documents → no figure, not a division by zero.
        assert!(cost_per_1k_docs(two_gb, 0, 8.0).is_none());
    }

    #[test]
    fn bytes_per_doc_is_the_residential_lever() {
        assert_eq!(bytes_per_doc(1_000_000, 4), Some(250_000.0));
        assert!(bytes_per_doc(1_000, 0).is_none());
    }

    #[test]
    fn the_budget_alert_fires_at_eighty_percent() {
        let budget_gb = 100.0;
        let seventy = (70.0 * BYTES_PER_GB) as u64;
        let eighty = (80.0 * BYTES_PER_GB) as u64;
        assert!(!over_budget_alert(budget_fraction(seventy, budget_gb)));
        assert!(over_budget_alert(budget_fraction(eighty, budget_gb)));
        assert!(
            over_budget_alert(budget_fraction(u64::MAX, budget_gb)),
            "way over"
        );
    }

    #[test]
    fn an_unset_budget_never_alerts() {
        // budget_gb == 0 means unconfigured; it must read as 0 %, not infinity.
        assert_eq!(budget_fraction(1_000_000_000, 0.0), 0.0);
        assert!(!over_budget_alert(budget_fraction(u64::MAX, 0.0)));
    }
}
