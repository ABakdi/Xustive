//! OCR against a real rendered screenshot (M3-T04.8, the CER check in miniature).
//!
//! Gated on the traineddata being present — it is operator-provisioned (`data/tessdata/`, git-ignored
//! like the LLM models), so CI without it skips rather than fails, mirroring the SERP fixture tests.

use std::path::PathBuf;

fn tessdata() -> Option<String> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/tessdata");
    (dir.join("eng.traineddata").exists()).then(|| dir.display().to_string())
}

#[test]
fn a_rendered_screenshot_is_read_back() {
    let Some(tessdata) = tessdata() else {
        eprintln!("skipping: data/tessdata not provisioned");
        return;
    };
    let bytes = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/screenshot.png"),
    )
    .expect("fixture present");

    let ocr =
        xustive_media::ocr::recognise(&bytes, &tessdata, "fra+eng", xustive_media::ocr::MAX_PIXELS)
            .expect("ocr runs");

    // The rendered text was "Bonjour Algerie / search engine / paracetamol 500mg".
    let lower = ocr.text.to_lowercase();
    assert!(
        lower.contains("paracetamol") && lower.contains("algerie"),
        "expected the rendered words, got {:?} (conf {})",
        ocr.text,
        ocr.confidence
    );
    assert!(
        ocr.usable,
        "clean rendered text should be usable (conf {})",
        ocr.confidence
    );
}
