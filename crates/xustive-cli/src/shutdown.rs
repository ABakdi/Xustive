//! Shared graceful-shutdown helpers ([[Error Handling and Resilience]], M4-T02.7).
//!
//! The contract every long-running binary honours: on `SIGTERM` (what a container runtime sends) or
//! Ctrl-C, stop taking new work, let in-flight work finish and acknowledge, and **exit within a
//! bounded grace period** — never hang. The grace bound is the part that is easy to forget: a drain
//! with no timeout turns one stuck request or fetch into a process that ignores `SIGTERM` and gets
//! `SIGKILL`ed, which is exactly the unclean stop graceful shutdown exists to avoid.

use std::future::Future;
use std::time::Duration;

/// How long a drain may take before the process forces itself down. Comfortably inside the typical
/// orchestrator `SIGTERM`→`SIGKILL` window (often 30 s), so a clean exit wins the race.
pub const GRACE: Duration = Duration::from_secs(25);

/// Resolve on `SIGTERM` or Ctrl-C. On non-unix, Ctrl-C only.
pub async fn signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Run `drain` to completion, but never longer than [`GRACE`]. Logs whether it finished cleanly or
/// the grace period elapsed with work still in flight.
pub async fn with_grace(what: &str, drain: impl Future<Output = ()>) {
    match tokio::time::timeout(GRACE, drain).await {
        Ok(()) => tracing::info!("{what}: drained cleanly"),
        Err(_) => tracing::warn!(
            grace_secs = GRACE.as_secs(),
            "{what}: grace period elapsed with work still draining; forcing exit"
        ),
    }
}
