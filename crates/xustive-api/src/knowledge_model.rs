//! The two jobs the model is actually better at than a table (M8-T04).
//!
//! [[ADR-0019 - The Knowledge Layer]] argued that choosing which authorities describe a film is a
//! lookup, not a judgement, and that spending a language model on it would occupy a GPU slot to
//! reproduce a `match` statement. That leaves two places where a model genuinely earns its keep:
//!
//! 1. **A blurb**, for an entity that has facts but no encyclopedic paragraph. Composed *only*
//!    from the stored claims and validated against them, so it can state nothing we did not
//!    harvest. Cached against the entity id, so the model runs once per entity in its lifetime
//!    rather than once per search — which is what makes it affordable on a 4 GB card.
//! 2. **Disambiguation**, when the resolver leaves two candidates genuinely close.
//!
//! **Off by default** (`ml.knowledge_assist`). Everything here is additive: with it clear, or with
//! no model loaded, the panel is exactly what it was — which is why every path fails open rather
//! than erroring.
//!
//! ## An honest correction to the milestone
//!
//! M8-T04.3 said both results would be cached against the entity id. That is true of the blurb and
//! **cannot** be true of disambiguation: which entity a query means is a property of the *query*,
//! and a cache keyed by that is a query log with extra steps ([[ADR-0008 - No Query Logging]]). So
//! disambiguation is live, uncached, and strictly bounded, and it only runs on the rare searches
//! where the resolver already admits it is unsure. When it cannot answer in time, the
//! deterministic leader ships — as it does today.

use std::time::Duration;

use xustive_knowledge::entity::Value;
use xustive_knowledge::Entity;

use crate::state::AppState;

/// How long the model gets. Short on purpose: this is additive, and a panel that arrives late is
/// worse than one that arrives without a sentence it never promised.
const BUDGET: Duration = Duration::from_secs(8);

/// How long a generated blurb is kept. Long — the facts behind it change on the scale of weeks,
/// and the whole point is that the model runs once per entity rather than once per reader.
const BLURB_TTL: u64 = 30 * 24 * 3600;

/// Cache key. The entity id, never the query — enumerable, identical for everyone who asks, and
/// silent about who asked.
fn blurb_key(id: &str) -> String {
    format!("knowledge:blurb:v1:{id}")
}

/// A one-line description composed from an entity's own claims, or nothing.
///
/// Reads the cache first and only ever generates on a miss. Returns `None` — never an error — for
/// every failure mode there is: assist off, no model, no facts worth describing, generation
/// refused, model too slow.
pub async fn blurb(state: &AppState, entity: &Entity, lang: &str) -> Option<String> {
    if !state.config.ml.knowledge_assist {
        return None;
    }
    // An entity that already has an encyclopedic paragraph does not need a written one, and
    // writing one anyway would put a model's prose next to a human's for no gain.
    if entity.extract(lang).is_some() {
        return None;
    }
    let cache = state.tool_cache.as_ref()?;
    let key = blurb_key(&entity.id);

    if let Ok(Some(cached)) = cache.get::<String>(&key).await {
        return Some(cached.payload);
    }

    let facts = describable(entity, lang);
    // Below this there is nothing to say that the fact rows do not already say better.
    if facts.len() < 3 {
        return None;
    }

    let generated = generate(state, entity, &facts, lang).await?;
    // Best-effort: a blurb that fails to cache is a blurb regenerated next time, not a failure.
    let _ = cache
        .put(
            &key,
            &xustive_toold::Cached {
                fetched_at: xustive_core::now_unix(),
                observed_at: entity.updated_at,
                source: "local model, from stored claims".into(),
                licence: "generated".into(),
                payload: generated.clone(),
            },
            BLURB_TTL,
        )
        .await;
    Some(generated)
}

/// Choose between two candidates the resolver could not separate (M8-T04.1).
///
/// Live and uncached, for the reason in the module docs: which entity a query means is a property
/// of the query, and caching that would be a query log with extra steps. It is affordable anyway
/// because it runs only where the resolver already admits it is unsure — a rare case by
/// construction, since scoring puts an exact name match far above everything else.
///
/// Returns the index of the chosen candidate, or `None` for every failure: assist off, no model,
/// no answer in time, an answer that is not one of the options. The caller then ships the
/// deterministic leader, which is what happens today.
pub async fn disambiguate(
    state: &AppState,
    query: &str,
    options: &[&Entity],
    lang: &str,
) -> Option<usize> {
    if !state.config.ml.knowledge_assist || options.len() < 2 {
        return None;
    }
    let engine = state.summariser()?;

    let listed: Vec<String> = options
        .iter()
        .enumerate()
        .map(|(i, e)| {
            format!(
                "{}. {} — {}",
                i + 1,
                e.names.best_label(lang).unwrap_or(&e.id),
                e.description(lang)
                    .or_else(|| e.description("en"))
                    .unwrap_or("no description")
            )
        })
        .collect();

    let prompt = xustive_ml::prompt::Prompt {
        system: "You pick which entity a search is about. Reply with the number alone. If you \
                 cannot tell, reply with 0."
            .to_string(),
        user: format!("Search: {query}\n\n{}", listed.join("\n")),
        cited: Vec::new(),
    };

    let generated = engine
        .generate(
            prompt,
            xustive_ml::engine::Sampling::default(),
            // Half the blurb budget: this one is on the request path, and a panel that arrives
            // late is worse than one that ships the deterministic answer.
            BUDGET / 2,
        )
        .await
        .ok()?;

    // The first number in the reply, and only if it names one of the options. A model that
    // answered "the second one" or "3" out of two has not answered.
    let picked: usize = numbers(&generated.text).first()?.parse().ok()?;
    if picked == 0 || picked > options.len() {
        return None;
    }
    Some(picked - 1)
}

/// The claims worth handing to the model, rendered as plain `key: value` lines.
///
/// Only what the panel itself would show, and only values that are already text. A model given a
/// raw QID would write it into a sentence.
fn describable(entity: &Entity, lang: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(d) = entity
        .description(lang)
        .or_else(|| entity.description("en"))
    {
        out.push(format!("description: {d}"));
    }
    for fact in xustive_knowledge::template::select(entity) {
        let rendered = match &fact.value {
            Value::Text(t) => t.clone(),
            Value::Entity { label, .. } if !label.is_empty() => label.clone(),
            Value::Number(n) => n.to_string(),
            Value::Quantity { amount, unit } => format!("{amount} {unit}"),
            // Dates and scores are formatted per language at render time; handing the model a raw
            // timestamp would invite it to invent a date format, and a wrong one reads as a fact.
            _ => continue,
        };
        out.push(format!("{}: {rendered}", fact.key));
    }
    out
}

/// Ask the model for one or two sentences, and refuse anything ungrounded.
async fn generate(
    state: &AppState,
    entity: &Entity,
    facts: &[String],
    lang: &str,
) -> Option<String> {
    let engine = state.summariser()?;
    let name = entity.names.best_label(lang)?;

    let system = "You describe an entity in at most two short sentences, using ONLY the facts \
                  given. Never add a fact that is not listed. Never guess. If the facts are too \
                  thin for a sentence, reply with nothing at all."
        .to_string();
    let user = format!(
        "Name: {name}\nKind: {}\nFacts:\n{}\n\nWrite the description in {}.",
        entity.kind.as_str(),
        facts.join("\n"),
        match lang {
            "ar" | "ary" => "Arabic",
            "fr" => "French",
            _ => "English",
        }
    );

    let prompt = xustive_ml::prompt::Prompt {
        system,
        user,
        cited: Vec::new(),
    };
    let generated = engine
        .generate(prompt, xustive_ml::engine::Sampling::default(), BUDGET)
        .await
        .ok()?;

    let text = generated.text.trim();
    // A truncated blurb is a half-sentence, which reads as a bug rather than as brevity.
    if generated.truncated || text.is_empty() {
        return None;
    }
    if !grounded(text, facts) {
        tracing::debug!(id = %entity.id, "blurb refused: not grounded in the stored claims");
        return None;
    }
    Some(clip(text))
}

/// Whether the text stays inside what it was shown.
///
/// A cheap, deliberately conservative check: every number the model wrote must appear in the facts
/// it was given. Numbers are what a reader most trusts and what a model most readily invents — a
/// birth year that is off by one is indistinguishable from a correct one at a glance, which is
/// exactly why it must not be possible.
fn grounded(text: &str, facts: &[String]) -> bool {
    let corpus = facts.join(" ");
    numbers(text).iter().all(|n| corpus.contains(n.as_str()))
}

fn numbers(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Two sentences at most, whatever the model did.
fn clip(text: &str) -> String {
    let mut out = String::new();
    let mut sentences = 0;
    for ch in text.chars() {
        out.push(ch);
        if matches!(ch, '.' | '。' | '؟' | '!' | '\u{06D4}') {
            sentences += 1;
            if sentences >= 2 {
                break;
            }
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use xustive_knowledge::entity::{Fact, Names, Provenance};
    use xustive_knowledge::Kind;

    fn person() -> Entity {
        let mut e = Entity::new("Q1", Kind::Person, 0);
        e.names = Names {
            labels: vec![("en".into(), "Warda".into())],
            aliases: vec![],
        };
        e.descriptions = vec![("en".into(), "Algerian singer".into())];
        e.facts = vec![Fact {
            key: "occupation".into(),
            value: Value::Entity {
                id: "Q177220".into(),
                label: "singer".into(),
            },
            provenance: Provenance::wikidata("Q1"),
            as_of: None,
        }];
        e
    }

    #[test]
    fn an_invented_number_is_refused() {
        // The check that matters. A birth year off by one is indistinguishable from a correct one
        // at a glance, which is exactly why a model must not be able to produce one.
        let facts = vec!["description: Algerian singer".to_string()];
        assert!(!grounded("She was born in 1939.", &facts));
        assert!(grounded("She was an Algerian singer.", &facts));
    }

    #[test]
    fn a_number_that_was_given_is_allowed_through() {
        let facts = vec![
            "birth_date: 1939".to_string(),
            "occupation: singer".to_string(),
        ];
        assert!(grounded("Born 1939, a singer.", &facts));
    }

    #[test]
    fn the_blurb_is_clipped_to_two_sentences() {
        let out = clip("One. Two. Three. Four.");
        assert_eq!(out, "One. Two.");
        // Text with no terminator is returned whole rather than dropped.
        assert_eq!(clip("no terminator here"), "no terminator here");
    }

    #[test]
    fn only_renderable_claims_reach_the_model() {
        // A raw QID handed to a model gets written into a sentence, and a timestamp invites an
        // invented date format.
        let mut e = person();
        e.facts.push(Fact {
            key: "birth_date".into(),
            value: Value::Date {
                at: 0,
                precision: xustive_knowledge::DatePrecision::Day,
            },
            provenance: Provenance::wikidata("Q1"),
            as_of: None,
        });
        e.facts.push(Fact {
            key: "citizenship".into(),
            value: Value::Entity {
                id: "Q262".into(),
                label: String::new(),
            },
            provenance: Provenance::wikidata("Q1"),
            as_of: None,
        });
        let lines = describable(&e, "en");
        assert!(lines.iter().any(|l| l == "occupation: singer"));
        assert!(
            !lines.iter().any(|l| l.contains("birth_date")),
            "a raw date must not be sent"
        );
        assert!(
            !lines.iter().any(|l| l.contains("Q262")),
            "an unresolved id must not be sent"
        );
    }

    #[test]
    fn a_reply_that_names_no_option_is_not_an_answer() {
        // The parsing rule, tested through `numbers` since the model itself is not in scope here.
        // "the second one" and "3" out of two are both non-answers, and taking either would be a
        // silent wrong pick in the place readers trust most.
        assert!(numbers("the second one").is_empty());
        assert_eq!(numbers("2").first().map(String::as_str), Some("2"));
        assert_eq!(numbers("0").first().map(String::as_str), Some("0"));
    }

    #[test]
    fn the_cache_key_is_the_entity_never_the_query() {
        // The line ADR-0008 draws. `Q42` is enumerable and shared; a query is neither.
        assert_eq!(blurb_key("Q42"), "knowledge:blurb:v1:Q42");
    }
}
