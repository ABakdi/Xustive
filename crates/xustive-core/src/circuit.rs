//! A circuit breaker: fail fast when a dependency is down, and probe for recovery ([[Error Handling
//! and Resilience]], M4-T02.2).
//!
//! When Meilisearch, Redis, the summariser, or a sidecar starts failing, hammering it with every
//! request makes things worse — each call waits for a timeout, ties up a worker, and delays the
//! error the caller was always going to get. A breaker turns that slow cascade into an immediate,
//! cheap failure, and periodically lets one request through to check whether the dependency is back.
//!
//! # States
//!
//! - **Closed** — normal. Requests pass; consecutive failures are counted. At the threshold, open.
//! - **Open** — fail fast. `allow` returns false without a call until the cooldown elapses.
//! - **Half-open** — one probe is allowed. If it succeeds, close (recovered); if it fails, open
//!   again with a **longer** cooldown (exponential backoff, capped), because a dependency that
//!   fails its probe is probably still broken and should be checked less often.
//!
//! # Testable time
//!
//! Every transition takes an explicit `now: Instant`, so tests drive the clock deterministically
//! rather than sleeping. [`SharedBreaker`] is the thread-safe wrapper real callers use; it supplies
//! `Instant::now()` and a mutex.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Breaker tuning.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// Consecutive failures in the Closed state that trip the breaker Open.
    pub failure_threshold: u32,
    /// Base Open duration. Doubles per consecutive Open (exponential backoff).
    pub cooldown: Duration,
    /// Ceiling on the backed-off cooldown, so it never grows without bound.
    pub max_cooldown: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            cooldown: Duration::from_secs(2),
            max_cooldown: Duration::from_secs(60),
        }
    }
}

/// The observable state, for metrics and the admin console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Closed,
    Open,
    HalfOpen,
}

/// The pure state machine. Not thread-safe on its own — see [`SharedBreaker`]. Every method that
/// depends on time takes `now`, so behaviour is fully determined by its inputs.
#[derive(Debug)]
pub struct Breaker {
    config: Config,
    state: State,
    consecutive_failures: u32,
    /// When Open, the instant at which a probe becomes allowed.
    open_until: Option<Instant>,
    /// How many times in a row it has opened, for the exponential backoff.
    consecutive_opens: u32,
    /// Whether a half-open probe is currently outstanding (only one at a time).
    probe_inflight: bool,
}

impl Breaker {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            state: State::Closed,
            consecutive_failures: 0,
            open_until: None,
            consecutive_opens: 0,
            probe_inflight: false,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// Whether a request may proceed. Advances Open→HalfOpen when the cooldown has elapsed, and
    /// hands out exactly one half-open probe.
    pub fn allow(&mut self, now: Instant) -> bool {
        match self.state {
            State::Closed => true,
            State::Open => {
                let ready = self.open_until.is_some_and(|t| now >= t);
                if ready {
                    // Move to half-open and let this one request be the probe.
                    self.state = State::HalfOpen;
                    self.probe_inflight = true;
                    true
                } else {
                    false
                }
            }
            State::HalfOpen => {
                // Only a single probe is in flight at once; everything else fails fast until it
                // resolves and decides whether to close or re-open.
                if self.probe_inflight {
                    false
                } else {
                    self.probe_inflight = true;
                    true
                }
            }
        }
    }

    /// Record a success. Any success closes the breaker and clears the backoff — the dependency is
    /// healthy, so the next failure starts counting from zero.
    pub fn on_success(&mut self) {
        self.state = State::Closed;
        self.consecutive_failures = 0;
        self.consecutive_opens = 0;
        self.open_until = None;
        self.probe_inflight = false;
    }

    /// Record a failure. In Closed, trips Open at the threshold; a failed half-open probe re-opens
    /// immediately with a longer cooldown.
    pub fn on_failure(&mut self, now: Instant) {
        match self.state {
            State::HalfOpen => self.open(now),
            State::Closed => {
                self.consecutive_failures += 1;
                if self.consecutive_failures >= self.config.failure_threshold {
                    self.open(now);
                }
            }
            // A failure reported while already Open (a request that was in flight when it tripped)
            // does not extend the cooldown — the timer is already running.
            State::Open => {}
        }
    }

    fn open(&mut self, now: Instant) {
        self.state = State::Open;
        self.probe_inflight = false;
        self.consecutive_opens = self.consecutive_opens.saturating_add(1);
        self.open_until = Some(now + self.backoff());
    }

    /// Cooldown for the current open streak: `cooldown * 2^(opens-1)`, capped at `max_cooldown`.
    fn backoff(&self) -> Duration {
        let shift = self.consecutive_opens.saturating_sub(1).min(20);
        let scaled = self.config.cooldown.saturating_mul(1u32 << shift);
        scaled.min(self.config.max_cooldown)
    }
}

/// A thread-safe, clock-supplying wrapper — what application code holds (behind an `Arc`).
///
/// Usage: `if !breaker.allow() { return fail_fast(); }` then `breaker.on_success()` /
/// `breaker.on_failure()` per the call's outcome.
#[derive(Clone)]
pub struct SharedBreaker {
    inner: Arc<Mutex<Breaker>>,
}

impl SharedBreaker {
    pub fn new(config: Config) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Breaker::new(config))),
        }
    }

    pub fn allow(&self) -> bool {
        self.with(|b| b.allow(Instant::now()))
    }

    pub fn on_success(&self) {
        self.with(|b| b.on_success());
    }

    pub fn on_failure(&self) {
        self.with(|b| b.on_failure(Instant::now()));
    }

    pub fn state(&self) -> State {
        self.with(|b| b.state())
    }

    fn with<R>(&self, f: impl FnOnce(&mut Breaker) -> R) -> R {
        // A poisoned lock means a panic while a breaker was held; recover the guard rather than
        // propagate, so one panic cannot wedge the breaker for every subsequent request.
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut guard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config {
            failure_threshold: 3,
            cooldown: Duration::from_secs(2),
            max_cooldown: Duration::from_secs(60),
        }
    }

    #[test]
    fn closed_allows_and_stays_closed_on_success() {
        let mut b = Breaker::new(cfg());
        let now = Instant::now();
        assert!(b.allow(now));
        b.on_success();
        assert_eq!(b.state(), State::Closed);
    }

    #[test]
    fn trips_open_at_the_failure_threshold() {
        let mut b = Breaker::new(cfg());
        let now = Instant::now();
        b.on_failure(now);
        b.on_failure(now);
        assert_eq!(b.state(), State::Closed, "two failures below threshold");
        b.on_failure(now);
        assert_eq!(b.state(), State::Open, "third failure trips it");
        assert!(!b.allow(now), "open breaker fails fast");
    }

    #[test]
    fn a_success_resets_the_failure_count() {
        let mut b = Breaker::new(cfg());
        let now = Instant::now();
        b.on_failure(now);
        b.on_failure(now);
        b.on_success(); // clears the count
        b.on_failure(now);
        b.on_failure(now);
        assert_eq!(b.state(), State::Closed, "count restarted after success");
    }

    #[test]
    fn open_transitions_to_half_open_after_cooldown() {
        let mut b = Breaker::new(cfg());
        let t0 = Instant::now();
        for _ in 0..3 {
            b.on_failure(t0);
        }
        assert!(!b.allow(t0), "still open during cooldown");
        assert!(
            !b.allow(t0 + Duration::from_secs(1)),
            "half a cooldown, still open"
        );
        // After the 2 s cooldown, exactly one probe is allowed.
        let after = t0 + Duration::from_secs(2);
        assert!(b.allow(after), "first request after cooldown probes");
        assert_eq!(b.state(), State::HalfOpen);
        assert!(
            !b.allow(after),
            "no second probe while the first is in flight"
        );
    }

    #[test]
    fn a_successful_probe_closes_the_breaker() {
        let mut b = Breaker::new(cfg());
        let t0 = Instant::now();
        for _ in 0..3 {
            b.on_failure(t0);
        }
        let after = t0 + Duration::from_secs(2);
        assert!(b.allow(after)); // probe
        b.on_success();
        assert_eq!(b.state(), State::Closed);
        assert!(b.allow(after), "closed again, requests pass");
    }

    #[test]
    fn a_failed_probe_reopens_with_exponential_backoff() {
        let mut b = Breaker::new(cfg());
        let t0 = Instant::now();
        for _ in 0..3 {
            b.on_failure(t0);
        }
        // First open: 2 s cooldown.
        let t1 = t0 + Duration::from_secs(2);
        assert!(b.allow(t1)); // probe
        b.on_failure(t1); // probe fails → reopen, cooldown doubles to 4 s
        assert_eq!(b.state(), State::Open);
        assert!(
            !b.allow(t1 + Duration::from_secs(3)),
            "3 s < doubled 4 s cooldown"
        );
        assert!(b.allow(t1 + Duration::from_secs(4)), "4 s cooldown elapsed");
    }

    #[test]
    fn backoff_is_capped() {
        let mut b = Breaker::new(Config {
            failure_threshold: 1,
            cooldown: Duration::from_secs(10),
            max_cooldown: Duration::from_secs(30),
        });
        let mut now = Instant::now();
        // Open, probe, fail — many times. Cooldown would be 10,20,40,80… but must cap at 30.
        for _ in 0..8 {
            b.on_failure(now); // opens (threshold 1)
            let open_for = b.open_until.unwrap() - now;
            assert!(
                open_for <= Duration::from_secs(30),
                "cooldown exceeded the cap"
            );
            now += open_for;
            assert!(b.allow(now)); // probe
        }
    }

    #[test]
    fn shared_breaker_is_usable_across_clones() {
        let a = SharedBreaker::new(cfg());
        let b = a.clone();
        assert_eq!(a.state(), State::Closed);
        b.on_failure();
        b.on_failure();
        b.on_failure();
        // Both handles see the same tripped state.
        assert_eq!(a.state(), State::Open);
        assert!(!a.allow());
    }
}
