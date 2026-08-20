//! Domain authority — the "famous websites" ranking signal.
//!
//! A per-domain prior for how well-known a site is, independent of any single query. It is what lets
//! a search for a film surface `imdb.com` above a forum thread that mentions the same title: both
//! match the words, but one is the authoritative home of the answer.
//!
//! The map is curated (`data/sources/authority.tsv`), not computed from a link graph — a link-graph
//! PageRank is the eventual upgrade, but a hand-picked prior for the domains people actually mean is
//! both cheaper and, at this corpus size, more accurate. It is compiled in for the same reason the
//! trust tiers are: a missing file must not silently flatten the signal.
//!
//! # Algeria-first
//!
//! This is where "Algeria-first" lives in the ranker. Any `.dz` host gets [`HOME_FLOOR`] even when it
//! is not listed, so an unlisted Algerian site still outranks an unlisted global one on this signal;
//! an unlisted non-`.dz` host gets [`BASELINE`]. Listed domains use their own score, which for a few
//! global institutions rises above the home floor — correct, because for a plainly global query the
//! global authority *should* win, and the weight on this signal is small enough that it only ever
//! breaks ties among documents that already match.

use std::collections::HashMap;

/// Authority (0–1) given to an unlisted `.dz` host — the Algeria-first home floor.
pub const HOME_FLOOR: f32 = 0.62;

/// Authority (0–1) given to an unlisted non-`.dz` host: present, but unproven.
pub const BASELINE: f32 = 0.35;

/// Load the curated domain→authority map from `data/sources/authority.tsv`.
///
/// Compiled in rather than read at runtime: the list is small and a missing file would quietly drop
/// the whole signal to the floor — a ranking regression nobody would notice. Scores are scaled from
/// the file's 0–100 to 0–1; malformed lines are skipped.
pub fn load() -> HashMap<String, f32> {
    const TSV: &str = include_str!("../../../data/sources/authority.tsv");
    parse(TSV)
}

fn parse(tsv: &str) -> HashMap<String, f32> {
    let mut out = HashMap::new();
    for line in tsv.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
        if cols.len() < 3 {
            continue;
        }
        let Ok(score) = cols[2].parse::<f32>() else {
            continue;
        };
        let domain = cols[0].trim_start_matches("www.").to_ascii_lowercase();
        if domain.is_empty() {
            continue;
        }
        out.insert(domain, (score / 100.0).clamp(0.0, 1.0));
    }
    out
}

/// The authority score (0–1) for a document's domain.
///
/// Resolution order: an exact listed domain, then any listed parent domain (so `ocw.mit.edu` finds
/// `mit.edu` if only the parent is listed), then the `.dz` home floor, then the baseline. `domain` is
/// a registrable host such as `en.wikipedia.org`; a `www.` prefix and case are ignored.
pub fn score_for(map: &HashMap<String, f32>, domain: &str) -> f32 {
    let host = domain
        .trim()
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    if host.is_empty() {
        return BASELINE;
    }
    if let Some(s) = map.get(&host) {
        return *s;
    }
    // Walk up the parent domains: en.wikipedia.org → wikipedia.org → org.
    let mut rest = host.as_str();
    while let Some((_, parent)) = rest.split_once('.') {
        if let Some(s) = map.get(parent) {
            return *s;
        }
        rest = parent;
    }
    if host == "dz" || host.ends_with(".dz") {
        return HOME_FLOOR;
    }
    BASELINE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_list_loads_and_covers_the_expected_names() {
        let m = load();
        assert!(m.len() >= 60, "only {} domains loaded", m.len());
        // A few anchors across categories.
        assert!(m.get("wikipedia.org").copied().unwrap_or(0.0) > 0.9);
        assert!(m.contains_key("imdb.com"));
        assert!(m.contains_key("bbc.com"));
        assert!(m.contains_key("aps.dz"));
    }

    #[test]
    fn a_listed_domain_gets_its_score_and_a_subdomain_inherits_the_parent() {
        let m = load();
        let wiki = score_for(&m, "wikipedia.org");
        assert_eq!(
            score_for(&m, "en.wikipedia.org"),
            wiki,
            "subdomain inherits"
        );
        assert_eq!(score_for(&m, "www.wikipedia.org"), wiki, "www stripped");
    }

    #[test]
    fn algeria_is_first_unlisted_dz_beats_unlisted_global() {
        let m = load();
        let dz = score_for(&m, "some-random-shop.dz");
        let global = score_for(&m, "some-random-blog.com");
        assert_eq!(dz, HOME_FLOOR);
        assert_eq!(global, BASELINE);
        assert!(
            dz > global,
            "an unlisted .dz host must take the tie over a global one"
        );
    }

    #[test]
    fn a_famous_global_institution_can_exceed_the_home_floor() {
        // Deliberate: for a plainly global query, global authority should be allowed to win. The
        // small weight on this signal keeps it a tie-breaker, not a takeover.
        let m = load();
        assert!(score_for(&m, "wikipedia.org") > HOME_FLOOR);
    }

    #[test]
    fn empty_or_unknown_hosts_are_the_baseline_and_do_not_panic() {
        let m = load();
        assert_eq!(score_for(&m, ""), BASELINE);
        assert_eq!(score_for(&m, "example.com"), BASELINE);
    }
}
