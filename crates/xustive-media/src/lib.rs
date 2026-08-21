//! Media pipelines: image OCR (M3-T04). In-memory only — no file ever touches disk.
pub mod backend;
pub mod ocr;

pub use backend::{Fallback, OcrBackend, Sidecar, Tesseract};
