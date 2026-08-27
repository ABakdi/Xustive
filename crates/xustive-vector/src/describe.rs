//! Words about an image (M10): the sidecar's `describe` reply, and the rule that turns style
//! scores into one label.
//!
//! The sidecar returns raw cosines against every style prompt. CLIP's own logit scale is 100, so
//! a softmax over `100 × cosine` is the model's calibrated opinion; a label is kept only when the
//! top style holds at least half of it and twice the runner-up's share. Measured on 2026-08-27: a screenshot 0.95, photographs
//! 0.70–0.83, and the threshold leaves an ambiguous picture unlabelled rather than mislabelled —
//! a chip that filters wrongly is worse than no chip.

use std::collections::HashMap;

use serde::Deserialize;

/// What the sidecar says about one image.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Description {
    /// Cosine per style id, all of them.
    #[serde(default)]
    pub styles: HashMap<String, f32>,
    /// The top subjects, best first.
    #[serde(default)]
    pub subjects: Vec<Subject>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Subject {
    pub id: String,
    pub score: f32,
}

/// The vector and the words, from one `describe=1` call.
#[derive(Debug, Clone)]
pub struct Described {
    pub vector: Vec<f32>,
    pub description: Description,
}

/// CLIP's logit scale: the temperature that makes the softmax mean something.
pub const LOGIT_SCALE: f32 = 100.0;
/// The share of the softmax the top style must hold to become the label…
pub const MIN_STYLE_PROB: f32 = 0.5;
/// …and how far ahead of the runner-up it must be. A floor alone is not enough: with a few
/// styles a 0.004-cosine gap still clears 0.5, and a picture that is nearly as much one thing
/// as another is not either.
pub const MIN_STYLE_LEAD: f32 = 2.0;

impl Description {
    /// The one style this picture is, or `None` when CLIP is not sure enough.
    pub fn style_label(&self) -> Option<String> {
        let mut probs = self.style_probabilities();
        probs.sort_by(|a, b| b.1.total_cmp(&a.1));
        let (label, top) = probs.first().cloned()?;
        let second = probs.get(1).map(|p| p.1).unwrap_or(0.0);
        (top >= MIN_STYLE_PROB && top >= MIN_STYLE_LEAD * second).then_some(label)
    }

    /// Softmax over `LOGIT_SCALE × cosine`, per style.
    pub fn style_probabilities(&self) -> Vec<(String, f32)> {
        if self.styles.is_empty() {
            return Vec::new();
        }
        let max = self.styles.values().copied().fold(f32::MIN, f32::max);
        let exps: Vec<(String, f32)> = self
            .styles
            .iter()
            .map(|(k, v)| (k.clone(), ((v - max) * LOGIT_SCALE).exp()))
            .collect();
        let sum: f32 = exps.iter().map(|(_, e)| e).sum();
        exps.into_iter().map(|(k, e)| (k, e / sum)).collect()
    }

    /// The subjects worth saying out loud — the ones clearly above the rest — as ids.
    pub fn subject_labels(&self, max: usize) -> Vec<String> {
        // A subject counts when it is within 0.03 cosine of the best; below that CLIP is listing
        // what the picture is *not quite*.
        let Some(best) = self.subjects.first().map(|s| s.score) else {
            return Vec::new();
        };
        self.subjects
            .iter()
            .filter(|s| best - s.score <= 0.03)
            .take(max)
            .map(|s| s.id.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(pairs: &[(&str, f32)]) -> Description {
        Description {
            styles: pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            subjects: vec![],
        }
    }

    #[test]
    fn a_clear_winner_is_the_label_and_a_close_call_is_none() {
        // The measured screenshot: 0.2622 against 0.2292 → 0.95.
        let d = desc(&[
            ("screenshot", 0.2622),
            ("document", 0.2292),
            ("diagram", 0.2105),
        ]);
        assert_eq!(d.style_label().as_deref(), Some("screenshot"));
        // Two styles a hair apart: no label, no wrong chip.
        let d = desc(&[("photo", 0.240), ("illustration", 0.236), ("drawing", 0.20)]);
        assert_eq!(d.style_label(), None);
    }

    #[test]
    fn subjects_near_the_best_are_said_and_the_rest_are_not() {
        let d = Description {
            styles: HashMap::new(),
            subjects: vec![
                Subject {
                    id: "casbah".into(),
                    score: 0.2953,
                },
                Subject {
                    id: "mosque".into(),
                    score: 0.2928,
                },
                Subject {
                    id: "building".into(),
                    score: 0.2392,
                },
            ],
        };
        assert_eq!(d.subject_labels(5), vec!["casbah", "mosque"]);
    }
}
