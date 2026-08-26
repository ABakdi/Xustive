//! The entity panel endpoint (M8-T02).
//!
//! Reads the knowledge index and nothing else. The serving plane has no route to the internet
//! ([[ADR-0001 - Two-Plane Architecture]]) and this endpoint does not want one: everything it
//! serves was harvested on the ingestion plane, so an entity nobody has harvested has no panel,
//! which [[ADR-0019 - The Knowledge Layer]] argues is correct rather than unfortunate.
//!
//! Served out of band, like the summary and unlike the instant answers: the search path costs
//! nothing, the rail announces itself while this runs, and a slow or empty answer degrades to a
//! rail that is simply not there.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use xustive_knowledge::resolve::{self, Candidate};
use xustive_knowledge::{index, template, Entity};

use crate::state::AppState;

/// How many candidates to judge. The index ranks by its own relevance; the resolver re-ranks by
/// signals the index does not have. Ten is enough for the ambiguity that actually occurs — shared
/// city names, a film and the book it came from — without paying for a long tail nothing will pick.
const CANDIDATES: usize = 10;

/// How many corpus documents to count per candidate before the signal saturates. The resolver caps
/// its contribution anyway, so counting beyond this buys nothing and costs a wider search.
const CORPUS_PROBE: usize = 50;

#[derive(Debug, Deserialize)]
pub struct PanelQuery {
    pub q: String,
    #[serde(default)]
    pub lang: Option<String>,
}

/// `GET /api/v1/knowledge?q=&lang=` — the entity for this query, or nothing.
///
/// Answers `204 No Content` rather than an empty body with a 200. Most queries are not about an
/// entity, and "there is no panel" is a different thing from "here is an empty panel" for both the
/// client and the caches in between.
pub async fn panel(
    State(state): State<AppState>,
    Query(params): Query<PanelQuery>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let lang = params.lang.as_deref().unwrap_or("ar");
    let raw = params.q.trim();

    // The cheap gate first, before any index round trip. A question belongs to the summariser.
    if !resolve::is_panel_shaped(raw) {
        return Err(StatusCode::NO_CONTENT);
    }

    let candidates = candidates(&state, raw).await;
    let Some(resolution) = resolve::choose(raw, &candidates) else {
        // Nobody has harvested this. Record the gap so the store converges on what this audience
        // actually asks for rather than on a guess about it (M8-T09) — through the *same*
        // k-anonymous mechanism weak coverage already uses, under the same floor, in the same
        // ephemeral instance. An entity fewer than k people asked for is never written down.
        record_miss(&state, raw).await;
        return Err(StatusCode::NO_CONTENT);
    };

    // When the resolver left a near-tie, the model may break it (M8-T04.1). Live and bounded; if
    // it declines or is too slow, the deterministic leader ships — which is what happens today.
    let resolution = match resolution.also.as_ref() {
        Some(runner_up) => {
            let options = [&resolution.entity, runner_up];
            match crate::knowledge_model::disambiguate(&state, raw, &options, lang).await {
                // The model preferred the runner-up: swap them, and keep the leader as the
                // alternative so the ambiguity is still visible rather than merely re-hidden.
                Some(1) => resolve::Resolution {
                    entity: runner_up.clone(),
                    also: Some(resolution.entity.clone()),
                    ..resolution
                },
                _ => resolution,
            }
        }
        None => resolution,
    };

    // A written line, only for an entity that has facts and no encyclopedic paragraph, only when
    // the operator turned it on, and cached against the entity so the model runs once per entity
    // rather than once per search (M8-T04).
    let blurb = crate::knowledge_model::blurb(&state, &resolution.entity, lang).await;

    let mut body = render(&resolution.entity, resolution.also.as_ref(), lang);
    if let Some(text) = blurb {
        body["blurb"] = json!({
            "text": text,
            // Labelled, always. A reader is entitled to know which sentence a machine wrote.
            "generated": true,
        });
    }
    Ok((StatusCode::OK, Json(body)))
}

/// Fetch and score the candidate entities for a query.
async fn candidates(state: &AppState, q: &str) -> Vec<Candidate> {
    let query = xustive_search::Query::new(q).limit(CANDIDATES);
    let Ok(response) = state.search.search::<Value>(index::INDEX, &query).await else {
        // A missing or unreachable knowledge index means no panel, never a failed request. The
        // rail is additive; nothing on the page depends on it.
        return Vec::new();
    };

    let mut out = Vec::new();
    for hit in &response.hits {
        let Some(entity) = index::from_document(hit) else {
            continue;
        };
        let corpus_mentions = corpus_mentions(state, &entity).await;
        out.push(Candidate {
            entity,
            corpus_mentions,
        });
    }
    out
}

/// How much our own crawled corpus talks about this entity.
///
/// The signal that makes an Algeria-first engine behave like one: two cities share a name, and the
/// one the Algerian web writes about is the one a reader here means. Counted against the entity's
/// preferred label rather than the raw query, so the count measures the *entity*, not the typing.
async fn corpus_mentions(state: &AppState, entity: &Entity) -> u32 {
    let Some(label) = entity
        .names
        .best_label("ar")
        .or_else(|| entity.names.best_label("en"))
    else {
        return 0;
    };
    let probe = xustive_search::Query::new(label).limit(CORPUS_PROBE);
    state
        .search
        .search::<Value>(xustive_search::settings::DOCUMENTS, &probe)
        .await
        .map(|r| r.hits.len() as u32)
        .unwrap_or(0)
}

/// Shape the panel for the client.
///
/// Facts stay typed and labels stay machine keys: the interface holds the translations, so the
/// same harvested entity renders correctly in four languages. Provenance rides along per fact,
/// because a licence that travels with its field cannot be forgotten by a renderer that assumed
/// one licence for the whole card.
fn render(entity: &Entity, also: Option<&Entity>, lang: &str) -> Value {
    json!({
        "id": entity.id,
        "kind": entity.kind.as_str(),
        "title": entity.names.best_label(lang),
        "description": entity
            .description(lang)
            .or_else(|| entity.description("en")),
        "extract": entity.extract(lang).map(|e| json!({
            "text": e.text,
            "lang": e.lang,
            "source": e.provenance.source,
            "licence": e.provenance.licence,
            "url": e.provenance.url,
        })),
        // The template decides what this kind shows and in what order, capped per key — the
        // parser stores every property it recognises, and a panel is not a data dump.
        "facts": template::select(entity),
        "authorities": entity.authorities,
        "images": entity.images,
        "also": also.map(|a| json!({
            "id": a.id,
            "title": a.names.best_label(lang),
            "description": a.description(lang).or_else(|| a.description("en")),
        })),
        "updated_at": entity.updated_at,
    })
}

/// Namespace for entity demand.
///
/// Separate from `discovery` so the two never mix: a weak *search* wants a crawl source, a weak
/// *entity* wants a harvest, and an operator reading either list should not have to guess which
/// kind of gap a row is.
pub const DEMAND_NAMESPACE: &str = "entity";

/// Record that a panel-shaped query resolved to nothing.
///
/// Governed by the same switch as weak coverage: with the flag clear, nothing about this query is
/// recorded anywhere. Best-effort — a lost increment is a slightly undercounted gap, never a
/// correctness problem.
async fn record_miss(state: &AppState, query: &str) {
    let disc = &state.config.discovery;
    if !disc.weak_coverage_enabled {
        return;
    }
    let normalised = query.trim().to_lowercase();
    if normalised.is_empty() {
        return;
    }
    if let Some(w) = xustive_ingest::weak_coverage::WeakCoverage::connect_in(
        state.config.queue.signals_url(),
        DEMAND_NAMESPACE,
        disc.effective_k(),
        std::time::Duration::from_secs(disc.weak_coverage_window_days * 86_400),
    ) {
        w.record(&normalised).await;
    }
}

/// `POST /api/v1/knowledge/render` — render a Wikidata document the **web tier** fetched (M8-T03
/// live fallback).
///
/// The serving plane has no egress, so it cannot look an entity up when the store lacks one. The
/// web tier can, exactly as ADR-0014's Wikipedia panel does — and rather than teach it a second,
/// weaker parser, it hands the raw document here, where the store's own parser and templates turn
/// it into the same panel a harvested entity gets. Nothing is fetched and nothing is written: the
/// miss was already recorded k-anonymously by [`panel`], so the harvester will hold the entity
/// next time and this path stops being needed for it.
///
/// Two-round shape: the first call returns `unresolved` — the ids of directors, units and
/// reviewers whose labels the document does not carry — and the web tier fetches those labels and
/// calls again with them. Two cheap local calls instead of one that needs the internet.
pub async fn render_document(
    State(_state): State<AppState>,
    Json(req): Json<RenderRequest>,
) -> Result<Json<Value>, StatusCode> {
    let lang = req.lang.as_deref().unwrap_or("ar");
    let now = xustive_core::now_unix();
    let Some(mut entity) = xustive_knowledge::wikidata::parse(&req.doc, now) else {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    };
    if !req.labels.is_empty() {
        xustive_knowledge::wikidata::fill_entity_labels(&mut entity, &req.labels);
    }
    if let Some(x) = req.extract {
        if !x.text.trim().is_empty() {
            xustive_knowledge::wikidata::attach_extract(&mut entity, &x.lang, x.text, x.url);
        }
    }
    if !entity.is_renderable() {
        return Err(StatusCode::NO_CONTENT);
    }

    // What the second round needs: every reference the templates would show whose label is still
    // an id. Only the *shown* facts, so the web tier does not fetch labels for a cast of forty.
    let mut unresolved: Vec<String> = Vec::new();
    for fact in template::select(&entity) {
        match &fact.value {
            xustive_knowledge::Value::Entity { id, label } if label.is_empty() => {
                unresolved.push(id.clone())
            }
            xustive_knowledge::Value::Score { reviewer, .. }
                if xustive_knowledge::wikidata::is_qid(reviewer) =>
            {
                unresolved.push(reviewer.clone())
            }
            xustive_knowledge::Value::Quantity { unit, .. }
                if xustive_knowledge::wikidata::is_qid(unit) =>
            {
                unresolved.push(unit.clone())
            }
            _ => {}
        }
    }
    unresolved.sort();
    unresolved.dedup();

    let mut body = render(&entity, None, lang);
    body["unresolved"] = json!(unresolved);
    body["live"] = json!(true);
    Ok(Json(body))
}

#[derive(Debug, serde::Deserialize)]
pub struct RenderRequest {
    /// One entity from `wbgetentities`, as Wikidata returned it.
    pub doc: Value,
    #[serde(default)]
    pub lang: Option<String>,
    /// `(id, label)` pairs from the second round.
    #[serde(default)]
    pub labels: Vec<(String, String)>,
    #[serde(default)]
    pub extract: Option<RenderExtract>,
}

#[derive(Debug, serde::Deserialize)]
pub struct RenderExtract {
    pub lang: String,
    pub text: String,
    #[serde(default)]
    pub url: String,
}

/// `POST /api/v1/knowledge/resolve-live` — choose among Wikidata candidates the web tier fetched.
///
/// The first version of the live fallback ranked candidates in the web tier by sitelink count
/// alone, and on the French page "messi" resolved to Jesus Christ: Wikidata's French search
/// matched *Messie*, and prominence did the rest. Prominence without a name match is the wrong
/// rule, and the right rule already exists — [`resolve::choose`], with its exact-name-first
/// scoring, its corpus-agreement signal and its precision floor. So the web tier sends the raw
/// candidate documents and this endpoint judges them the way the store's own hits are judged.
/// Answers `204` when nothing clears the floor, which is the honest answer for a query that is
/// not a name.
pub async fn resolve_live(
    State(state): State<AppState>,
    Json(req): Json<ResolveLiveRequest>,
) -> Result<Json<Value>, StatusCode> {
    let query = req.query.trim();
    if !resolve::is_panel_shaped(query) {
        return Err(StatusCode::NO_CONTENT);
    }
    let now = xustive_core::now_unix();
    let mut candidates = Vec::new();
    for doc in &req.docs {
        let Some(entity) = xustive_knowledge::wikidata::parse(doc, now) else {
            continue;
        };
        let corpus_mentions = corpus_mentions(&state, &entity).await;
        candidates.push(Candidate {
            entity,
            corpus_mentions,
        });
    }
    // The kinds the caller expects — a relation query knows what its subject must be.
    let prefer: Vec<xustive_knowledge::Kind> = req
        .prefer_kinds
        .iter()
        .filter_map(|k| serde_json::from_value(json!(k)).ok())
        .collect();
    let Some(resolution) = resolve::choose_preferring(query, &candidates, &prefer) else {
        return Err(StatusCode::NO_CONTENT);
    };
    Ok(Json(json!({
        "id": resolution.entity.id,
        "kind": resolution.entity.kind.as_str(),
        "confidence": resolution.confidence,
        "also": resolution.also.map(|a| a.id),
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct ResolveLiveRequest {
    pub query: String,
    /// Entities from `wbgetentities`, as Wikidata returned them.
    #[serde(default)]
    pub docs: Vec<Value>,
    /// Kinds to lift — `person`, `film`, … — when the caller knows what the subject must be.
    #[serde(default)]
    pub prefer_kinds: Vec<String>,
}
