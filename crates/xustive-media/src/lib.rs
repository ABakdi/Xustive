//! Media pipelines: image OCR (M3-T04) and perceptual hashing. In-memory only — no file touches disk.
pub mod backend;
pub mod ext;
pub mod ocr;
pub mod phash;

pub use backend::{Fallback, OcrBackend, Sidecar, Tesseract};
pub use phash::{dhash, hamming};
