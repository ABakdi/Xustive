//! The enrichment pipeline ([[Enrichment Pipeline]] §M2-T06.1, §M2-T06.2).
//!
//! After extraction, a document is annotated: its wilaya, its topics, a spam score, a quality
//! score. Those were four inline calls in the parser; this makes them **steps** behind one trait,
//! run by an ordered executor. The point is not indirection for its own sake — it is that the
//! executor can then make one decision the inline calls could not: under load, run only the
//! **required** steps and mark the document `Partial`, so a document is never dropped for lack of a
//! topic label, and a repass job (§M2-T06.9) can find the partial ones and finish them later.
//!
//! Every step has the same shape — `apply(&mut Document)` — even though they read and write
//! different fields. The one step that needed extra context, quality scoring, reads its missing
//! input (the extraction method) back off the document's `access_path`, which the parser has
//! already set by the time enrichment runs. That is what lets the signatures be uniform.

use xustive_core::{Document, EnrichmentLevel};

use crate::parse::{quality_score, Method};

/// One annotation applied to a document after extraction.
pub trait EnrichmentStep: Send + Sync {
    /// Stable name, for logging and per-step metrics.
    fn name(&self) -> &'static str;

    /// Whether the step must run even under load. Required steps feed ranking and spam suppression;
    /// optional ones are hints that improve grouping but whose absence does not break a result.
    fn required(&self) -> bool;

    /// Annotate the document in place.
    fn apply(&self, doc: &mut Document);
}

/// Geo/wilaya hinting (§M2-T06.5). Optional — a missing wilaya hint costs a filter facet, not a
/// result.
struct Gazetteer;
impl EnrichmentStep for Gazetteer {
    fn name(&self) -> &'static str {
        "gazetteer"
    }
    fn required(&self) -> bool {
        false
    }
    fn apply(&self, doc: &mut Document) {
        if let Some(hint) = crate::gazetteer::detect_wilaya(&format!("{} {}", doc.title, doc.body))
        {
            doc.geo.wilaya = Some(hint.code.to_string());
            doc.geo.wilaya_name = Some(hint.name.to_string());
        }
    }
}

/// Topic labelling (§M2-T06.6). Optional — grouping hints.
struct Topics;
impl EnrichmentStep for Topics {
    fn name(&self) -> &'static str {
        "topics"
    }
    fn required(&self) -> bool {
        false
    }
    fn apply(&self, doc: &mut Document) {
        doc.topics = crate::topics::label(&doc.title, &doc.body);
    }
}

/// Spam scoring (§M2-T06.4). Required — search suppresses at 0.8, so a missing score would let spam
/// through into default results.
struct Spam;
impl EnrichmentStep for Spam {
    fn name(&self) -> &'static str {
        "spam"
    }
    fn required(&self) -> bool {
        true
    }
    fn apply(&self, doc: &mut Document) {
        doc.spam_score = crate::spam::spam_score(&doc.title, &doc.body);
    }
}

/// Quality scoring (§M2-T06.3). Required — it feeds ranking directly. Reads the extraction method
/// back off `access_path`, which the parser set before enrichment.
struct Quality;
impl EnrichmentStep for Quality {
    fn name(&self) -> &'static str {
        "quality"
    }
    fn required(&self) -> bool {
        true
    }
    fn apply(&self, doc: &mut Document) {
        let method = doc
            .access_path
            .as_deref()
            .map(Method::parse)
            .unwrap_or(Method::Fallback);
        doc.quality_score = quality_score(doc, method);
    }
}

/// The ordered set of steps.
pub struct Pipeline {
    steps: Vec<Box<dyn EnrichmentStep>>,
}

/// What the executor did, for observability and the repass job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    /// The steps that actually ran, in order.
    pub applied: Vec<&'static str>,
    /// The optional steps skipped because the run was `Partial`.
    pub skipped: Vec<&'static str>,
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::standard()
    }
}

impl Pipeline {
    /// The standard pipeline. Required steps (spam, quality) last, so an optional step's output can
    /// feed a required one in future without reordering; today the steps are independent, but the
    /// order is fixed rather than incidental.
    pub fn standard() -> Self {
        Self {
            steps: vec![
                Box::new(Gazetteer),
                Box::new(Topics),
                Box::new(Spam),
                Box::new(Quality),
            ],
        }
    }

    /// Run the pipeline over `doc`. At `Full`, every step runs. At `Partial` — chosen by the caller
    /// under load — only the required steps run, and the document is stamped `Partial` so a repass
    /// can complete it. `Full` explicitly stamps `Full`, so a repass that finishes a document clears
    /// the marker.
    pub fn run(&self, doc: &mut Document, level: EnrichmentLevel) -> Ran {
        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        for step in &self.steps {
            if level == EnrichmentLevel::Partial && !step.required() {
                skipped.push(step.name());
                continue;
            }
            step.apply(doc);
            applied.push(step.name());
        }
        doc.enrichment_level = level;
        Ran { applied, skipped }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xustive_core::{Document, SourceType};

    fn doc_with(title: &str, body: &str) -> Document {
        let mut d = Document::new("01J", "https://x.dz/a", SourceType::Web);
        d.title = title.into();
        d.body = body.into();
        d.body_len = body.len();
        d.access_path = Some("density".into());
        d
    }

    #[test]
    fn a_full_run_applies_every_step_and_marks_the_document_full() {
        let mut d = doc_with(
            "الجزائر العاصمة تشهد نموا اقتصاديا",
            "شهدت ولاية الجزائر العاصمة هذا الأسبوع نموا في الاقتصاد والتجارة والاستثمار الوطني.",
        );
        let ran = Pipeline::standard().run(&mut d, EnrichmentLevel::Full);
        assert_eq!(ran.applied, vec!["gazetteer", "topics", "spam", "quality"]);
        assert!(ran.skipped.is_empty());
        assert_eq!(d.enrichment_level, EnrichmentLevel::Full);
        assert!(d.quality_score > 0.0, "quality was scored");
    }

    #[test]
    fn a_partial_run_skips_optional_steps_and_marks_partial() {
        let mut d = doc_with("عنوان", "نص المقال القصير جدا هنا.");
        // Give it a wilaya and topic vocabulary so the skipped steps *would* have populated fields.
        let ran = Pipeline::standard().run(&mut d, EnrichmentLevel::Partial);
        assert_eq!(ran.applied, vec!["spam", "quality"], "only required steps");
        assert_eq!(ran.skipped, vec!["gazetteer", "topics"]);
        assert_eq!(d.enrichment_level, EnrichmentLevel::Partial);
        // The required steps still ran.
        assert!(
            d.geo.wilaya.is_none(),
            "the optional gazetteer step was skipped"
        );
        assert!(d.topics.is_empty(), "the optional topics step was skipped");
    }

    #[test]
    fn a_repass_at_full_clears_the_partial_marker_and_fills_the_optional_fields() {
        // First a partial run under load…
        let mut d = doc_with(
            "ولاية وهران",
            "شهدت ولاية وهران افتتاح ملعب رياضي جديد لكرة القدم بحضور الجمهور الرياضي.",
        );
        Pipeline::standard().run(&mut d, EnrichmentLevel::Partial);
        assert_eq!(d.enrichment_level, EnrichmentLevel::Partial);
        assert!(d.geo.wilaya.is_none());

        // …then the repass finishes it.
        Pipeline::standard().run(&mut d, EnrichmentLevel::Full);
        assert_eq!(d.enrichment_level, EnrichmentLevel::Full);
        assert!(d.geo.wilaya.is_some(), "the repass filled the wilaya hint");
    }

    #[test]
    fn required_and_optional_steps_are_classified_as_documented() {
        let p = Pipeline::standard();
        let required: Vec<&str> = p
            .steps
            .iter()
            .filter(|s| s.required())
            .map(|s| s.name())
            .collect();
        assert_eq!(required, vec!["spam", "quality"]);
    }
}
