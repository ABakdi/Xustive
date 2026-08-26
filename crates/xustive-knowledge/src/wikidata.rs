//! Turning a Wikidata entity document into an [`Entity`].
//!
//! Pure and synchronous on purpose. Every fact this product will state about a person or a film
//! passes through here, so it is the part that most needs to be testable against saved documents
//! rather than against a live endpoint that changes under the test.
//!
//! Two things it deliberately does not do. It does not fetch — the harvester does that, on the
//! ingestion plane, where [[ADR-0001 - Two-Plane Architecture]] allows it. And it does not decide
//! what to *show*: it extracts every fact it recognises, and the per-kind template picks. Parsing
//! and presentation drift apart if the parser starts having opinions.

use serde_json::Value as Json;

use crate::entity::{
    Authority, DatePrecision, Entity, Extract, Fact, Image, Names, Provenance, Value, LANGS,
};
use crate::kind;

/// External-identifier properties, and how to turn one into a link.
///
/// This table is the whole mechanism behind "a film shows IMDb and Rotten Tomatoes". The
/// identifiers are CC0 facts recorded by Wikidata; the URLs are built from them. Nothing here
/// fetches or scrapes those sites, which their terms forbid and
/// [[ADR-0019 - The Knowledge Layer]] rules out.
const AUTHORITIES: &[(&str, &str, &str)] = &[
    ("P345", "imdb", "https://www.imdb.com/title/{}/"),
    (
        "P1258",
        "rotten_tomatoes",
        "https://www.rottentomatoes.com/{}",
    ),
    ("P4947", "tmdb", "https://www.themoviedb.org/movie/{}"),
    ("P1712", "metacritic", "https://www.metacritic.com/{}"),
    ("P434", "musicbrainz", "https://musicbrainz.org/artist/{}"),
    ("P2013", "facebook", "https://www.facebook.com/{}"),
    ("P2002", "x", "https://x.com/{}"),
];

/// IMDb identifiers distinguish people from titles by prefix, and the title URL is wrong for a
/// person. Getting this wrong would produce a confident dead link on every actor's panel.
fn imdb_url(id: &str) -> String {
    if id.starts_with("nm") {
        format!("https://www.imdb.com/name/{id}/")
    } else {
        format!("https://www.imdb.com/title/{id}/")
    }
}

/// Properties worth storing as facts, with the stable key the interface translates.
///
/// Kept as one table across kinds rather than one per kind: a property means the same thing
/// wherever it appears, and the template decides which of them a given card shows.
const FACT_PROPS: &[(&str, &str)] = &[
    // People
    ("P569", "birth_date"),
    ("P570", "death_date"),
    ("P19", "birth_place"),
    ("P106", "occupation"),
    ("P27", "citizenship"),
    ("P39", "position_held"),
    ("P54", "member_of_team"),
    ("P800", "notable_work"),
    // Film and television
    ("P57", "director"),
    ("P58", "screenwriter"),
    ("P161", "cast"),
    ("P136", "genre"),
    ("P577", "release_date"),
    ("P2047", "duration"),
    ("P364", "original_language"),
    ("P272", "production_company"),
    // Places
    ("P1082", "population"),
    ("P17", "country"),
    ("P131", "located_in"),
    ("P2046", "area"),
    // Organisations
    ("P571", "inception"),
    ("P159", "headquarters"),
    ("P112", "founder"),
    ("P169", "chief_executive"),
    // Works
    ("P50", "author"),
    ("P123", "publisher"),
    ("P175", "performer"),
    ("P1476", "title"),
    // Life
    ("P225", "taxon_name"),
];

/// Parse one Wikidata entity document.
///
/// Returns `None` only when the document has no id — everything else degrades to a thinner entity,
/// because a film missing its runtime is still worth a panel and refusing the whole document over
/// one unparseable claim would make coverage hostage to Wikidata's messiest corners.
pub fn parse(doc: &Json, now: i64) -> Option<Entity> {
    let id = doc.get("id")?.as_str()?.to_string();
    let claims = doc.get("claims").unwrap_or(&Json::Null);

    let instance_of = entity_ids(claims, "P31");
    let kind = kind::from_instance_of(&instance_of);

    let mut entity = Entity::new(id.clone(), kind, now);
    entity.names = names(doc);
    entity.descriptions = per_language(doc.get("descriptions"));
    entity.prominence = doc
        .get("sitelinks")
        .and_then(Json::as_object)
        .map(|m| m.len() as u32)
        .unwrap_or(0);

    let prov = Provenance::wikidata(&id);

    for (prop, key) in FACT_PROPS {
        for value in values_of(claims, prop) {
            if let Some(v) = to_value(&value) {
                entity.facts.push(Fact {
                    key: (*key).to_string(),
                    value: v,
                    provenance: prov.clone(),
                    as_of: None,
                });
            }
        }
    }

    entity.facts.extend(review_scores(claims, &prov));
    entity.authorities = authorities(claims);
    entity.images = images(claims);

    Some(entity)
}

/// Review scores from `P444`, attributed to the reviewer named in the `P447` qualifier.
///
/// An unattributed score is dropped rather than shown: `85%` means something different from
/// Metacritic than from a Rotten Tomatoes audience, and a number without its source is the kind of
/// confident non-fact [[Instant Answers]] §2 exists to prevent.
fn review_scores(claims: &Json, prov: &Provenance) -> Vec<Fact> {
    let mut out = Vec::new();
    let Some(statements) = claims.get("P444").and_then(Json::as_array) else {
        return out;
    };
    for st in statements {
        if rank_is_deprecated(st) {
            continue;
        }
        let Some(raw) = st
            .pointer("/mainsnak/datavalue/value")
            .and_then(Json::as_str)
        else {
            continue;
        };
        let Some(reviewer) = st
            .pointer("/qualifiers/P447/0/datavalue/value/id")
            .and_then(Json::as_str)
        else {
            continue;
        };
        let Some((value, best)) = parse_score(raw) else {
            continue;
        };
        out.push(Fact {
            key: "review_score".into(),
            value: Value::Score {
                value,
                best,
                reviewer: reviewer.to_string(),
            },
            provenance: prov.clone(),
            as_of: None,
        });
    }
    out
}

/// Scores arrive as free text: `85%`, `7.8/10`, `4/5`. Anything else is left alone rather than
/// coerced — a misread scale is a wrong answer that looks precise.
fn parse_score(raw: &str) -> Option<(f64, f64)> {
    let raw = raw.trim();
    if let Some(pct) = raw.strip_suffix('%') {
        return pct.trim().parse::<f64>().ok().map(|v| (v, 100.0));
    }
    let (value, best) = raw.split_once('/')?;
    Some((value.trim().parse().ok()?, best.trim().parse().ok()?))
}

fn authorities(claims: &Json) -> Vec<Authority> {
    let mut out = Vec::new();
    for (prop, key, template) in AUTHORITIES {
        let Some(id) = values_of(claims, prop)
            .into_iter()
            .find_map(|v| v.as_str().map(str::to_string))
        else {
            continue;
        };
        let url = if *key == "imdb" {
            imdb_url(&id)
        } else {
            template.replace("{}", &id)
        };
        out.push(Authority {
            key: (*key).to_string(),
            id,
            url,
        });
    }
    out
}

/// Commons images. The URL is built through Special:FilePath, which redirects to the current file
/// and therefore survives the re-uploads that break a hardcoded upload URL.
fn images(claims: &Json) -> Vec<Image> {
    values_of(claims, "P18")
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .take(1)
        .map(|file| {
            let encoded = file.replace(' ', "_");
            Image {
                url: format!(
                    "https://commons.wikimedia.org/wiki/Special:FilePath/{}?width=480",
                    urlencode(&encoded)
                ),
                author: None,
                // Deliberately the honest default: Commons files are individually licensed and the
                // licence lives on the file page, which is why `credit_url` is mandatory here. The
                // harvester fills the real licence in when it reads the file's metadata.
                licence: "see credit".into(),
                credit_url: Some(format!(
                    "https://commons.wikimedia.org/wiki/File:{}",
                    urlencode(&encoded)
                )),
            }
        })
        .collect()
}

/// Percent-encode the characters that actually break a Wikimedia path. Not a general encoder — a
/// general one would encode the `_` and `-` that are ordinary in file names.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn names(doc: &Json) -> Names {
    let labels = per_language(doc.get("labels"));
    let mut aliases = Vec::new();
    if let Some(map) = doc.get("aliases").and_then(Json::as_object) {
        for (lang, list) in map {
            if !LANGS.contains(&lang.as_str()) {
                continue;
            }
            for a in list.as_array().into_iter().flatten() {
                if let Some(v) = a.get("value").and_then(Json::as_str) {
                    aliases.push((lang.clone(), v.to_string()));
                }
            }
        }
    }
    Names { labels, aliases }
}

/// Wikidata's `{lang: {language, value}}` maps, restricted to the languages we speak. Storing the
/// other three hundred would multiply the index for readers who will never see them.
fn per_language(node: Option<&Json>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(map) = node.and_then(Json::as_object) {
        for lang in LANGS {
            if let Some(v) = map
                .get(lang)
                .and_then(|e| e.get("value"))
                .and_then(Json::as_str)
            {
                out.push((lang.to_string(), v.to_string()));
            }
        }
    }
    out
}

/// Statement values for a property, skipping deprecated ones.
///
/// Deprecated rank exists precisely to mark a claim editors have decided is wrong but kept for the
/// record. Reading it back as a fact would republish a known error.
fn values_of(claims: &Json, prop: &str) -> Vec<Json> {
    claims
        .get(prop)
        .and_then(Json::as_array)
        .map(|sts| {
            sts.iter()
                .filter(|st| !rank_is_deprecated(st))
                .filter_map(|st| st.pointer("/mainsnak/datavalue/value").cloned())
                .collect()
        })
        .unwrap_or_default()
}

fn rank_is_deprecated(statement: &Json) -> bool {
    statement.get("rank").and_then(Json::as_str) == Some("deprecated")
}

fn entity_ids(claims: &Json, prop: &str) -> Vec<String> {
    values_of(claims, prop)
        .into_iter()
        .filter_map(|v| v.get("id").and_then(Json::as_str).map(str::to_string))
        .collect()
}

/// One Wikidata datavalue to one typed [`Value`].
fn to_value(v: &Json) -> Option<Value> {
    if let Some(s) = v.as_str() {
        return Some(Value::Text(s.to_string()));
    }
    // An entity reference. The label is filled in later by [`fill_entity_labels`] once the
    // harvester has resolved the referenced ids in a batch — one round trip for a whole panel
    // rather than one per director.
    if let Some(id) = v.get("id").and_then(Json::as_str) {
        return Some(Value::Entity {
            id: id.to_string(),
            label: String::new(),
        });
    }
    if let Some(time) = v.get("time").and_then(Json::as_str) {
        let precision = v.get("precision").and_then(Json::as_i64).unwrap_or(11);
        return parse_time(time, precision);
    }
    if let Some(amount) = v.get("amount").and_then(Json::as_str) {
        let n: f64 = amount.trim_start_matches('+').parse().ok()?;
        let unit = v
            .get("unit")
            .and_then(Json::as_str)
            .filter(|u| *u != "1")
            .and_then(|u| u.rsplit('/').next())
            .unwrap_or("")
            .to_string();
        return Some(if unit.is_empty() {
            Value::Number(n)
        } else {
            Value::Quantity { amount: n, unit }
        });
    }
    if let Some(text) = v.get("text").and_then(Json::as_str) {
        return Some(Value::Text(text.to_string()));
    }
    None
}

/// `+1952-03-11T00:00:00Z` with a precision code.
///
/// Precision below year is dropped rather than rounded: a decade-precision date rendered as a year
/// asserts nine years nobody claimed. BCE dates are dropped too — they are outside the range of
/// anything this product answers, and the arithmetic to place them correctly is not worth carrying
/// for entities no one searches here.
fn parse_time(time: &str, precision: i64) -> Option<Value> {
    let rest = time.strip_prefix('+')?;
    let (date, _) = rest.split_once('T')?;
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;

    let (precision, month, day) = match precision {
        11 => (DatePrecision::Day, month.max(1), day.max(1)),
        10 => (DatePrecision::Month, month.max(1), 1),
        9 => (DatePrecision::Year, 1, 1),
        _ => return None,
    };
    Some(Value::Date {
        at: unix_from_civil(year, month, day),
        precision,
    })
}

/// Days from the civil calendar to Unix seconds — Howard Hinnant's algorithm, valid across the
/// whole proleptic Gregorian range and free of the leap-year edge cases a hand-rolled version
/// invariably gets wrong in February.
fn unix_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe - 719_468) * 86_400
}

/// Fill in the labels of referenced entities once the harvester has resolved them.
///
/// References whose label is still unknown keep an empty one, and the renderer drops those — a
/// director shown as `Q8877` is worse than a director not shown.
///
/// Review-score reviewers are filled here too. They arrive as a bare QID, and the whole reason a
/// score without a named reviewer is refused is that `99/100` means nothing until you know who
/// said it — `Q105584` names the reviewer no better than nothing does.
pub fn fill_entity_labels(entity: &mut Entity, labels: &[(String, String)]) {
    let lookup = |id: &str| labels.iter().find(|(k, _)| k == id).map(|(_, l)| l.clone());
    for fact in &mut entity.facts {
        match &mut fact.value {
            Value::Entity { id, label } if label.is_empty() => {
                if let Some(l) = lookup(id) {
                    *label = l;
                }
            }
            Value::Score { reviewer, .. } if is_qid(reviewer) => {
                if let Some(l) = lookup(reviewer) {
                    *reviewer = l;
                }
            }
            // A unit is an entity too. `117 Q7727` is not a runtime; `117 minute` is.
            Value::Quantity { unit, .. } if is_qid(unit) => {
                if let Some(l) = lookup(unit) {
                    *unit = l;
                }
            }
            _ => {}
        }
    }
}

/// Whether a string is still an unresolved entity id rather than a name.
pub fn is_qid(s: &str) -> bool {
    s.strip_prefix('Q')
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

/// Attach a Wikipedia extract, which is CC BY-SA rather than CC0 and therefore carries different
/// attribution from everything else on the card.
pub fn attach_extract(entity: &mut Entity, lang: &str, text: String, article_url: String) {
    entity.extracts.retain(|e| e.lang != lang);
    entity.extracts.push(Extract {
        lang: lang.to_string(),
        text,
        provenance: Provenance {
            source: "Wikipedia".into(),
            licence: "CC-BY-SA-4.0".into(),
            url: Some(article_url),
        },
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::Kind;
    use serde_json::json;

    fn film() -> Json {
        json!({
            "id": "Q83495",
            "labels": {
                "en": {"language": "en", "value": "The Matrix"},
                "fr": {"language": "fr", "value": "Matrix"},
                "ar": {"language": "ar", "value": "المصفوفة"},
                "de": {"language": "de", "value": "Matrix"}
            },
            "descriptions": {"en": {"language": "en", "value": "1999 film"}},
            "aliases": {"en": [{"language": "en", "value": "Matrix"}]},
            "sitelinks": {"enwiki": {}, "frwiki": {}, "arwiki": {}},
            "claims": {
                "P31": [{"rank": "normal", "mainsnak": {"datavalue": {"value": {"entity-type": "item", "id": "Q11424"}}}}],
                "P57": [{"rank": "normal", "mainsnak": {"datavalue": {"value": {"entity-type": "item", "id": "Q510034"}}}}],
                "P577": [{"rank": "normal", "mainsnak": {"datavalue": {"value": {"time": "+1999-03-31T00:00:00Z", "precision": 11}}}}],
                "P2047": [{"rank": "normal", "mainsnak": {"datavalue": {"value": {"amount": "+136", "unit": "http://www.wikidata.org/entity/Q7727"}}}}],
                "P345": [{"rank": "normal", "mainsnak": {"datavalue": {"value": "tt0133093"}}}],
                "P1258": [{"rank": "normal", "mainsnak": {"datavalue": {"value": "m/matrix"}}}],
                "P444": [
                    {"rank": "normal",
                     "mainsnak": {"datavalue": {"value": "83%"}},
                     "qualifiers": {"P447": [{"datavalue": {"value": {"id": "Q105584"}}}]}},
                    {"rank": "normal", "mainsnak": {"datavalue": {"value": "73/100"}}}
                ]
            }
        })
    }

    #[test]
    fn a_film_document_yields_its_kind_facts_and_authorities() {
        let e = parse(&film(), 1_000).unwrap();
        assert_eq!(e.kind, Kind::Film);
        assert_eq!(e.names.best_label("fr"), Some("Matrix"));
        assert_eq!(e.description("en"), Some("1999 film"));
        assert_eq!(e.prominence, 3);
        assert_eq!(
            e.authority("imdb").map(|a| a.url.as_str()),
            Some("https://www.imdb.com/title/tt0133093/")
        );
        assert_eq!(
            e.authority("rotten_tomatoes").map(|a| a.url.as_str()),
            Some("https://www.rottentomatoes.com/m/matrix")
        );
    }

    #[test]
    fn only_the_four_languages_we_speak_are_stored() {
        // Wikidata carries three hundred languages. Keeping them all would multiply the index for
        // readers who will never see them.
        let e = parse(&film(), 0).unwrap();
        assert!(e.names.label("de").is_none());
        assert_eq!(e.names.labels.len(), 3);
    }

    #[test]
    fn a_score_without_a_named_reviewer_is_dropped() {
        // 83% from Rotten Tomatoes and 73/100 from nobody: the second is a number that looks
        // precise and means nothing, which is the failure Instant Answers §2 exists to prevent.
        let e = parse(&film(), 0).unwrap();
        let scores: Vec<_> = e.facts.iter().filter(|f| f.key == "review_score").collect();
        assert_eq!(scores.len(), 1);
        match &scores[0].value {
            Value::Score {
                value,
                best,
                reviewer,
            } => {
                assert_eq!(*value, 83.0);
                assert_eq!(*best, 100.0);
                assert_eq!(reviewer, "Q105584");
            }
            other => panic!("expected a score, got {other:?}"),
        }
    }

    #[test]
    fn a_date_keeps_the_precision_the_publisher_asserted() {
        let e = parse(&film(), 0).unwrap();
        match e.fact("release_date").map(|f| &f.value) {
            Some(Value::Date { at, precision }) => {
                assert_eq!(*precision, DatePrecision::Day);
                // 1999-03-31.
                assert_eq!(*at, 922_838_400);
            }
            other => panic!("expected a day-precision date, got {other:?}"),
        }
    }

    #[test]
    fn a_decade_precision_date_is_dropped_rather_than_rounded_to_a_year() {
        // Precision 8 is a decade. Rendering it as a year asserts nine years nobody claimed.
        assert!(parse_time("+1990-01-01T00:00:00Z", 8).is_none());
        assert!(parse_time("-0044-03-15T00:00:00Z", 11).is_none());
    }

    #[test]
    fn a_quantity_keeps_its_unit_apart_from_a_bare_number() {
        // So a renderer cannot print a runtime as a population.
        let e = parse(&film(), 0).unwrap();
        match e.fact("duration").map(|f| &f.value) {
            Some(Value::Quantity { amount, unit }) => {
                assert_eq!(*amount, 136.0);
                assert_eq!(unit, "Q7727");
            }
            other => panic!("expected a quantity, got {other:?}"),
        }
    }

    #[test]
    fn a_deprecated_statement_is_not_republished_as_a_fact() {
        // Deprecated rank exists to mark a claim editors decided is wrong. Reading it back would
        // republish a known error under our name.
        let doc = json!({
            "id": "Q1",
            "claims": {
                "P31": [{"rank": "normal", "mainsnak": {"datavalue": {"value": {"id": "Q5"}}}}],
                "P106": [
                    {"rank": "deprecated", "mainsnak": {"datavalue": {"value": {"id": "Q_WRONG"}}}},
                    {"rank": "normal", "mainsnak": {"datavalue": {"value": {"id": "Q_RIGHT"}}}}
                ]
            }
        });
        let e = parse(&doc, 0).unwrap();
        let occupations: Vec<_> = e.facts.iter().filter(|f| f.key == "occupation").collect();
        assert_eq!(occupations.len(), 1);
        assert!(matches!(&occupations[0].value, Value::Entity { id, .. } if id == "Q_RIGHT"));
    }

    #[test]
    fn an_imdb_person_id_builds_a_name_url_not_a_title_url() {
        // Same property, two URL shapes. Getting it wrong puts a confident dead link on every
        // actor's panel.
        assert_eq!(
            imdb_url("nm0000229"),
            "https://www.imdb.com/name/nm0000229/"
        );
        assert_eq!(
            imdb_url("tt0133093"),
            "https://www.imdb.com/title/tt0133093/"
        );
    }

    #[test]
    fn a_reviewer_qid_is_resolved_to_a_name_like_any_other_reference() {
        // The point of demanding P447 was attribution. `Q105584` attributes a score no better
        // than nothing does, so the reviewer is resolved on the same pass as a director.
        let mut e = parse(&film(), 0).unwrap();
        fill_entity_labels(
            &mut e,
            &[("Q105584".to_string(), "Rotten Tomatoes".to_string())],
        );
        assert!(matches!(
            e.fact("review_score").map(|f| &f.value),
            Some(Value::Score { reviewer, .. }) if reviewer == "Rotten Tomatoes"
        ));
    }

    #[test]
    fn a_quantity_unit_is_resolved_like_any_other_reference() {
        // `117 Q7727` is not a runtime a reader can use.
        let mut e = parse(&film(), 0).unwrap();
        fill_entity_labels(&mut e, &[("Q7727".to_string(), "minute".to_string())]);
        assert!(matches!(
            e.fact("duration").map(|f| &f.value),
            Some(Value::Quantity { unit, .. }) if unit == "minute"
        ));
    }

    #[test]
    fn a_resolved_reviewer_name_is_not_mistaken_for_an_id() {
        // "Q Score" is a real audience-measurement brand; treating any leading Q as an id would
        // re-resolve a name that was already correct.
        assert!(is_qid("Q105584"));
        assert!(!is_qid("Q Score"));
        assert!(!is_qid("Quentin"));
        assert!(!is_qid("Q"));
    }

    #[test]
    fn an_unresolved_entity_reference_keeps_an_empty_label_until_it_is_filled() {
        let mut e = parse(&film(), 0).unwrap();
        assert!(matches!(
            e.fact("director").map(|f| &f.value),
            Some(Value::Entity { label, .. }) if label.is_empty()
        ));
        fill_entity_labels(
            &mut e,
            &[("Q510034".to_string(), "Lana Wachowski".to_string())],
        );
        assert!(matches!(
            e.fact("director").map(|f| &f.value),
            Some(Value::Entity { label, .. }) if label == "Lana Wachowski"
        ));
    }

    #[test]
    fn a_document_without_an_id_is_refused_but_a_thin_one_is_not() {
        // Refusing a whole document over one missing claim would make coverage hostage to
        // Wikidata's messiest corners.
        assert!(parse(&json!({"labels": {}}), 0).is_none());
        let thin = parse(&json!({"id": "Q9", "claims": {}}), 0).unwrap();
        assert_eq!(thin.kind, Kind::Concept);
    }

    #[test]
    fn an_extract_carries_share_alike_attribution_not_the_cc0_of_the_claims() {
        // The reason provenance is per-field: one card mixes CC0 claims with a CC BY-SA paragraph.
        let mut e = parse(&film(), 0).unwrap();
        attach_extract(
            &mut e,
            "ar",
            "…".into(),
            "https://ar.wikipedia.org/wiki/X".into(),
        );
        let x = e.extract("ary").unwrap();
        assert_eq!(x.provenance.licence, "CC-BY-SA-4.0");
        assert_eq!(e.fact("director").unwrap().provenance.licence, "CC0-1.0");
    }

    #[test]
    fn re_attaching_an_extract_replaces_rather_than_duplicates() {
        let mut e = parse(&film(), 0).unwrap();
        attach_extract(&mut e, "en", "first".into(), "u".into());
        attach_extract(&mut e, "en", "second".into(), "u".into());
        assert_eq!(e.extracts.len(), 1);
        assert_eq!(e.extract("en").unwrap().text, "second");
    }

    #[test]
    fn a_file_name_with_spaces_and_apostrophes_makes_a_usable_url() {
        let imgs =
            images(&json!({"P18": [{"mainsnak": {"datavalue": {"value": "L'Oran d'antan.jpg"}}}]}));
        assert_eq!(imgs.len(), 1);
        assert!(imgs[0].url.contains("L%27Oran_d%27antan.jpg"));
        assert!(
            imgs[0].credit_url.is_some(),
            "a Commons image must be creditable"
        );
    }
}
