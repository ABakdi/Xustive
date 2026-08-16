//! The graded response-to-blocking ladder ([[Proxy Manager]] §4.6).
//!
//! `on_blocked` replaces the old flat `halt_and_flag`. The response now depends on the pool: a
//! platform block is a routine event handled by rotating identities, while an open-web block is a
//! site telling us something and is still met with a halt-and-flag. The open-web column is
//! deliberately unchanged from before [[ADR-0009]] — that decision altered the platform stance, not
//! the commitment to well-behaved crawling of ordinary websites.

use super::PoolKind;

/// What a site or platform did that we have to respond to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockSignal {
    /// `robots.txt` disallows the path. Open-web only.
    RobotsDisallow,
    /// 429 / `Retry-After`.
    RateLimited,
    /// 403 or an anti-bot challenge page.
    AntiBot,
    /// A captcha or account checkpoint.
    Captcha,
    /// A 200 with suspiciously empty content — possible silent soft-ban. Platform only.
    SilentEmpty,
    /// Challenge rate spiking across the whole platform — a defence rollout.
    PlatformWideSpike,
}

/// The action to take. The caller executes it; this module only decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Do not fetch — `robots.txt` said no.
    DoNotFetch,
    /// Honour the delay exactly and open the host breaker (open web).
    HonourAndBreak,
    /// Honour the delay, halve the identity's budget, back off ≥ 15 min (platform 429).
    HonourHalveBudgetBackoff,
    /// Halt this fetch and flag it for a human. **Never retried through another pool.**
    HaltAndFlag,
    /// Quarantine the identity and resume the work on a different one (platform block/challenge).
    QuarantineIdentity,
    /// Compare against canary results to tell a real empty page from a soft-ban.
    CanaryCheck,
    /// Halt the whole platform and page an operator.
    HaltPlatformAndPage,
}

/// Decide the response to `signal` for a request that went out through `pool`.
///
/// The invariant the guard test pins: for `direct` (open web), an anti-bot block or a captcha is
/// **always** `HaltAndFlag` — we do not retry a refusing site through another pool, because a small
/// Algerian news site that blocks us is to be respected, not evaded.
pub fn on_blocked(pool: PoolKind, signal: BlockSignal) -> Action {
    if pool.is_platform() {
        platform(signal)
    } else {
        open_web(signal)
    }
}

fn open_web(signal: BlockSignal) -> Action {
    match signal {
        BlockSignal::RobotsDisallow => Action::DoNotFetch,
        BlockSignal::RateLimited => Action::HonourAndBreak,
        // A site refusing us is obeyed, never routed around.
        BlockSignal::AntiBot | BlockSignal::Captcha => Action::HaltAndFlag,
        // These are platform concepts; on the open web the safe reading is still halt-and-flag.
        BlockSignal::SilentEmpty | BlockSignal::PlatformWideSpike => Action::HaltAndFlag,
    }
}

fn platform(signal: BlockSignal) -> Action {
    match signal {
        // Robots is not a platform concept — a platform request never consults it — but if it ever
        // arrives here, obeying is the safe answer.
        BlockSignal::RobotsDisallow => Action::DoNotFetch,
        BlockSignal::RateLimited => Action::HonourHalveBudgetBackoff,
        BlockSignal::AntiBot | BlockSignal::Captcha => Action::QuarantineIdentity,
        BlockSignal::SilentEmpty => Action::CanaryCheck,
        BlockSignal::PlatformWideSpike => Action::HaltPlatformAndPage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_web_never_evades_a_refusing_site() {
        // The load-bearing guarantee: on the open web, a block halts and flags — it is never retried
        // through another pool, whatever the signal.
        for signal in [BlockSignal::AntiBot, BlockSignal::Captcha] {
            assert_eq!(on_blocked(PoolKind::Direct, signal), Action::HaltAndFlag);
        }
        assert_eq!(
            on_blocked(PoolKind::Direct, BlockSignal::RobotsDisallow),
            Action::DoNotFetch
        );
        assert_eq!(
            on_blocked(PoolKind::Direct, BlockSignal::RateLimited),
            Action::HonourAndBreak
        );
    }

    #[test]
    fn a_platform_block_rotates_identities_rather_than_halting() {
        assert_eq!(
            on_blocked(PoolKind::Residential, BlockSignal::AntiBot),
            Action::QuarantineIdentity
        );
        assert_eq!(
            on_blocked(PoolKind::Residential, BlockSignal::Captcha),
            Action::QuarantineIdentity
        );
        assert_eq!(
            on_blocked(PoolKind::Residential, BlockSignal::RateLimited),
            Action::HonourHalveBudgetBackoff
        );
        assert_eq!(
            on_blocked(PoolKind::Mobile, BlockSignal::SilentEmpty),
            Action::CanaryCheck
        );
    }

    #[test]
    fn a_platform_wide_spike_halts_the_platform() {
        assert_eq!(
            on_blocked(PoolKind::Residential, BlockSignal::PlatformWideSpike),
            Action::HaltPlatformAndPage
        );
    }
}
