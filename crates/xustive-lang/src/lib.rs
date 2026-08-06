//! Language detection and query expansion for the Algerian language mix.
//!
//! Two components live here, because they share the same lexicon machinery and the same
//! fundamental problem: Algerian text is Arabic, Darija, French and English, written in two
//! scripts, frequently mixed inside one sentence.
//!
//! - [`detect`] — which language is this?
//! - [`expand`] — what else might the user have meant? (arriving with the query expander)

pub mod detect;
pub mod lexicon;
pub mod translit;

pub use detect::{Detection, Detector, DetectorConfig};
pub use lexicon::{Lexicon, Score};
