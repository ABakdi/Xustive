//! The evaluation section of the admin console (PROB-003 item 4).
//!
//! The eval harness, A/B runner, weight calibrator, and SERP yardstick all write dated JSON files
//! under `eval/reports/`, and the synonym miner writes review sheets under `data/expansion/` — a
//! quality trail an operator could only read by shelling into the repo. This endpoint surfaces
//! those files as they are: each report's headline numbers, the regression-gate verdict computed
//! exactly as `xustive eval --baseline` computes it, and the miner sheets awaiting review.
//! Read-only by design — re-baselining and applying calibrations stay deliberate CLI acts, because
//! both change what "regression" means.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde_json::{json, Value};

use crate::admin::Peer;
use crate::state::AppState;

/// Same tolerance as `xustive-cli`'s gate (`eval.rs::NDCG_TOLERANCE`): the console must render the
/// verdict CI would give, not a second opinion.
const NDCG_TOLERANCE: f64 = 0.01;

const REPORTS_DIR: &str = "eval/reports";
const CANDIDATES_DIR: &str = "data/expansion";

/// What kind of report a filename announces. The writers use fixed prefixes, so the name is the
/// contract — `ab-*` from the A/B runner, `serp-*` from the SERP yardstick, `calibration-*` from
/// the weight calibrator, `baseline.json` as the gate reference, and bare dates from `eval`.
fn kind_of(name: &str) -> &'static str {
    if name == "baseline.json" {
        "baseline"
    } else if name.starts_with("ab-") {
        "ab"
    } else if name.starts_with("serp-") {
        "serp"
    } else if name.starts_with("calibration-") {
        "calibration"
    } else {
        "eval"
    }
}

/// Pull the headline numbers out of one parsed report, tolerant of the shapes the different
/// writers produce. Unknown fields stay absent rather than defaulting — a missing score is
/// information (BUG-019 taught us a defaulted 0.0 turns a gate permanently green).
fn summarise(name: &str, raw: &Value) -> Value {
    let mut row = json!({
        "file": name,
        "kind": kind_of(name),
        "generated_at": raw.get("generated_at").cloned().unwrap_or(Value::Null),
        "queries": raw.get("queries").cloned().unwrap_or(Value::Null),
    });
    let obj = row.as_object_mut().expect("row is an object");
    for key in [
        "ndcg_at_10",
        "mrr_at_10",
        "recall_at_50",
        "zero_result_rate",
    ] {
        if let Some(v) = raw.get(key) {
            obj.insert(key.into(), v.clone());
        }
    }
    // Per-language nDCG, flattened to one number per language — the trend the console charts.
    if let Some(langs) = raw.get("by_language").and_then(Value::as_object) {
        let per_lang: serde_json::Map<String, Value> = langs
            .iter()
            .filter_map(|(lang, v)| Some((lang.clone(), v.get("ndcg_at_10")?.clone())))
            .collect();
        obj.insert("by_language".into(), Value::Object(per_lang));
    }
    // A/B variants and calibration rankings carry their own rows; pass them through untouched so
    // the page can render whatever the runner scored.
    for key in ["variants", "ranked", "engine"] {
        if let Some(v) = raw.get(key) {
            obj.insert(key.into(), v.clone());
        }
    }
    row
}

/// `GET /admin/eval` — every report under `eval/reports/`, the gate verdict, and the miner's
/// candidate sheets awaiting review.
pub async fn status(
    State(state): State<AppState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    crate::admin::authorise(&state, peer, &headers).map_err(|d| d.json())?;

    let mut reports: Vec<Value> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(REPORTS_DIR) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".json") {
                continue;
            }
            match std::fs::read_to_string(entry.path())
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            {
                Some(raw) => reports.push(summarise(&name, &raw)),
                // A file that exists but does not parse is worth a loud line, not silence.
                None => unreadable.push(name),
            }
        }
    }
    // Newest first by filename — every writer embeds the date in the name, so the lexical order of
    // same-kind files is the chronological order.
    reports.sort_by(|a, b| b["file"].as_str().cmp(&a["file"].as_str()));

    // The gate verdict: latest dated eval report against baseline.json, relative tolerance,
    // mirroring `eval.rs::gate`. No baseline or no eval run yet → no verdict, honestly.
    let baseline_ndcg = reports
        .iter()
        .find(|r| r["kind"] == "baseline")
        .and_then(|r| r["ndcg_at_10"].as_f64());
    let latest = reports.iter().find(|r| r["kind"] == "eval");
    let gate = match (baseline_ndcg, latest) {
        (Some(previous), Some(current)) => match current["ndcg_at_10"].as_f64() {
            Some(now) => {
                let allowed = previous * NDCG_TOLERANCE;
                json!({
                    "baseline_ndcg": previous,
                    "latest_ndcg": now,
                    "latest_file": current["file"],
                    "delta": now - previous,
                    "tolerance_pct": NDCG_TOLERANCE * 100.0,
                    "pass": (now - previous) >= -allowed,
                })
            }
            None => Value::Null,
        },
        _ => Value::Null,
    };

    // Miner review sheets: name and line count only — the sheet itself is reviewed in an editor,
    // where accepting a row means editing synonyms.tsv, and that act should stay hands-on.
    let mut candidates: Vec<Value> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(CANDIDATES_DIR) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("candidates-") && name.ends_with(".tsv")) {
                continue;
            }
            let rows = std::fs::read_to_string(entry.path())
                .map(|t| {
                    t.lines()
                        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
                        .count()
                })
                .unwrap_or(0);
            candidates.push(json!({ "file": name, "rows": rows }));
        }
    }
    candidates.sort_by(|a, b| b["file"].as_str().cmp(&a["file"].as_str()));

    Ok(Json(json!({
        "reports": reports,
        "unreadable": unreadable,
        "gate": gate,
        "candidates": candidates,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_prefixes_classify_reports() {
        assert_eq!(kind_of("baseline.json"), "baseline");
        assert_eq!(kind_of("ab-2026-08-25.json"), "ab");
        assert_eq!(kind_of("serp-google-2026-08-20.json"), "serp");
        assert_eq!(kind_of("calibration-2026-08-23.json"), "calibration");
        assert_eq!(kind_of("2026-08-24.json"), "eval");
    }

    #[test]
    fn summary_keeps_scores_and_flattens_languages() {
        let raw = json!({
            "generated_at": "2026-08-24",
            "queries": 200,
            "ndcg_at_10": 0.62,
            "mrr_at_10": 0.89,
            "by_language": { "ar": { "queries": 80, "ndcg_at_10": 0.69 } },
            "per_query": [ { "id": "x" } ],
        });
        let row = summarise("2026-08-24.json", &raw);
        assert_eq!(row["kind"], "eval");
        assert_eq!(row["ndcg_at_10"], 0.62);
        assert_eq!(row["by_language"]["ar"], 0.69);
        // The bulky per-query detail stays on disk — the console gets the headline.
        assert!(row.get("per_query").is_none());
    }

    #[test]
    fn missing_scores_stay_absent_not_zero() {
        let row = summarise("ab-2026-08-25.json", &json!({ "variants": [] }));
        assert!(row.get("ndcg_at_10").is_none());
        assert_eq!(row["kind"], "ab");
    }
}
