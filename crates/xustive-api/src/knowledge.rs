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
        return Err(StatusCode::NO_CONTENT);
    };

    Ok((
        StatusCode::OK,
        Json(render(&resolution.entity, resolution.also.as_ref(), lang)),
    ))
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
