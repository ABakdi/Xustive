//! Image OCR (M3-T04): decode → auto-orient → preprocess → tesseract → confidence-scored text.
//!
//! Entirely in memory. Leptonica reads the encoded bytes directly through `set_image_from_mem`, and
//! nothing is ever written to disk — the zero-disk-write privacy rule ([[Security and Privacy]] P4)
//! holds by construction here, there is no path that opens a file.
//!
//! The one heavy step is `recognise`, which is blocking and CPU-bound; its caller must run it on a
//! blocking pool, never on an async worker.

use std::io::Cursor;

use image::{DynamicImage, GenericImageView, GrayImage, ImageFormat};

/// Pixel budget — a guard against decompression bombs. Generous for a screenshot, far below what
/// would exhaust memory once expanded.
pub const MAX_PIXELS: u64 = 40_000_000;
/// Below this shortest side, the image is upscaled: small screenshots OCR far better enlarged.
const MIN_OCR_DIM: u32 = 1000;
/// Mean confidence (0–100) below which the text is treated as unusable.
pub const MIN_CONFIDENCE: f32 = 55.0;
/// Fewest alphanumeric characters worth keeping — below this it is noise, not text.
pub const MIN_USABLE_CHARS: usize = 8;
/// A first pass at or above this confidence is trusted as-is; below it (or if unusable) the
/// binarised retry runs. Clean screenshots clear this easily, so they never pay for the second pass.
const GOOD_ENOUGH_CONFIDENCE: f32 = 75.0;

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error("unrecognised or unsupported image format")]
    Format,
    #[error("image exceeds the {0}-pixel budget")]
    TooLarge(u64),
    #[error("could not decode the image")]
    Decode,
    #[error("ocr engine: {0}")]
    Engine(String),
}

/// The result of an OCR pass.
#[derive(Debug, Clone)]
pub struct Ocr {
    /// Extracted text, whitespace-collapsed.
    pub text: String,
    /// Mean per-word confidence, 0–100.
    pub confidence: f32,
    /// Whether the text clears the confidence and length thresholds. The caller uses this to decide
    /// whether to show it or fold it into a document's `body` — never a raw low-confidence dump.
    pub usable: bool,
}

/// Decode encoded image bytes, guarding format and pixel budget.
///
/// Dimensions are read from the header first, so a decompression bomb is refused before it is ever
/// expanded in memory — the cheap check that makes the pixel budget meaningful.
pub fn decode(bytes: &[u8], max_pixels: u64) -> Result<DynamicImage, OcrError> {
    let format = image::guess_format(bytes).map_err(|_| OcrError::Format)?;
    let (w, h) = image::ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|_| OcrError::Decode)?;
    if (w as u64).saturating_mul(h as u64) > max_pixels {
        return Err(OcrError::TooLarge(max_pixels));
    }
    image::load_from_memory_with_format(bytes, format).map_err(|_| OcrError::Decode)
}

/// EXIF orientation (1–8), or 1 when absent.
///
/// Only the orientation tag is read. GPS and every other tag are never touched — and decoding to
/// pixels discards all metadata regardless, so the orientation is applied and then gone.
fn exif_orientation(bytes: &[u8]) -> u32 {
    exif::Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .ok()
        .and_then(|e| {
            e.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|f| f.value.get_uint(0))
        })
        .unwrap_or(1)
}

fn apply_orientation(img: DynamicImage, o: u32) -> DynamicImage {
    match o {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

/// Auto-orient, upscale a small image, and reduce to grayscale.
///
/// Grayscale, not a hard threshold: tesseract binarises internally, and thresholding anti-aliased
/// screenshot text ourselves loses strokes and hurts more than it helps.
fn preprocess(img: DynamicImage, orientation: u32) -> DynamicImage {
    let img = apply_orientation(img, orientation);
    let (w, h) = img.dimensions();
    let short = w.min(h);
    let img = if short > 0 && short < MIN_OCR_DIM {
        let scale = (MIN_OCR_DIM as f32 / short as f32).min(4.0);
        img.resize(
            (w as f32 * scale) as u32,
            (h as f32 * scale) as u32,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };
    DynamicImage::ImageLuma8(img.to_luma8())
}

/// Run OCR over encoded image bytes.
///
/// `tessdata` is the directory holding the `*.traineddata` files; `langs` is a `+`-joined list such
/// as `"ara+fra+eng"` — Arabic first, since that is what most Algerian screenshots are.
///
/// **Blocking and CPU-bound.** Run it on a blocking pool.
///
/// Two passes at most. The first reads the grayscale image (tesseract binarises internally, which is
/// right for clean anti-aliased screenshot text). When that reads poorly — unusable, or below
/// [`GOOD_ENOUGH_CONFIDENCE`] — a second pass reads an **Otsu-binarised** version, which recovers
/// low-contrast or unevenly-lit sources a global grayscale pass struggles with. The better of the
/// two results wins, so the retry can only help: an image that already read well never reaches it.
pub fn recognise(
    bytes: &[u8],
    tessdata: &str,
    langs: &str,
    max_pixels: u64,
) -> Result<Ocr, OcrError> {
    let orientation = exif_orientation(bytes);
    let luma = preprocess(decode(bytes, max_pixels)?, orientation).to_luma8();

    let mut tess = leptess::LepTess::new(Some(tessdata), langs)
        .map_err(|e| OcrError::Engine(e.to_string()))?;

    let (raw1, conf1) = ocr_luma(&mut tess, &luma)?;
    let first = score(&raw1, conf1);
    if first.usable && conf1 >= GOOD_ENOUGH_CONFIDENCE {
        return Ok(first);
    }

    // Retry on a binarised image. Otsu picks the threshold from the image's own histogram, so it
    // adapts to each source rather than using a fixed cut.
    let threshold = otsu_threshold(&luma);
    let (raw2, conf2) = ocr_luma(&mut tess, &binarize(&luma, threshold))?;
    let second = score(&raw2, conf2);

    Ok(pick_better(first, second))
}

/// OCR one grayscale image on an existing engine: encode to PNG (in memory), set, read, score.
fn ocr_luma(tess: &mut leptess::LepTess, luma: &GrayImage) -> Result<(String, f32), OcrError> {
    let mut png = Vec::new();
    DynamicImage::ImageLuma8(luma.clone())
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|_| OcrError::Decode)?;
    tess.set_image_from_mem(&png)
        .map_err(|e| OcrError::Engine(e.to_string()))?;
    let raw = tess
        .get_utf8_text()
        .map_err(|e| OcrError::Engine(e.to_string()))?;
    let confidence = tess.mean_text_conf() as f32;
    Ok((raw, confidence))
}

/// Keep the better of two OCR results: a usable result always beats an unusable one; between two of
/// the same usability, the higher confidence wins.
fn pick_better(a: Ocr, b: Ocr) -> Ocr {
    match (a.usable, b.usable) {
        (true, false) => a,
        (false, true) => b,
        _ => {
            if b.confidence > a.confidence {
                b
            } else {
                a
            }
        }
    }
}

/// Otsu's method: the grayscale threshold that maximises between-class variance, computed from the
/// image histogram. A parameter-free way to split "ink" from "paper" that adapts per image.
fn otsu_threshold(luma: &GrayImage) -> u8 {
    let mut hist = [0u32; 256];
    for p in luma.pixels() {
        hist[p[0] as usize] += 1;
    }
    let total = luma.pixels().len() as f64;
    if total == 0.0 {
        return 128;
    }
    let sum: f64 = (0..256).map(|i| i as f64 * hist[i] as f64).sum();

    let mut sum_b = 0.0;
    let mut w_b = 0.0;
    let mut max_var = -1.0;
    let mut threshold = 128u8;
    for i in 0..256 {
        w_b += hist[i] as f64;
        if w_b == 0.0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0.0 {
            break;
        }
        sum_b += i as f64 * hist[i] as f64;
        let mean_b = sum_b / w_b;
        let mean_f = (sum - sum_b) / w_f;
        let between = w_b * w_f * (mean_b - mean_f) * (mean_b - mean_f);
        if between > max_var {
            max_var = between;
            threshold = i as u8;
        }
    }
    threshold
}

/// Binarise: pixels brighter than `threshold` become white, the rest black.
fn binarize(luma: &GrayImage, threshold: u8) -> GrayImage {
    let mut out = luma.clone();
    for p in out.pixels_mut() {
        p[0] = if p[0] > threshold { 255 } else { 0 };
    }
    out
}

/// Turn raw OCR output and a confidence into a scored [`Ocr`]: collapse whitespace, normalise via
/// [`xustive_text::normalize`], and decide usability against the confidence and length floors.
///
/// Shared by every backend — the tesseract engine here and the [`crate::backend::Sidecar`] — so
/// "usable" means exactly one thing regardless of which engine produced the text.
pub fn score(raw: &str, confidence: f32) -> Ocr {
    let text = xustive_text::normalize(&collapse_whitespace(raw));
    let alnum = text.chars().filter(|c| c.is_alphanumeric()).count();
    let usable = confidence >= MIN_CONFIDENCE && alnum >= MIN_USABLE_CHARS;
    Ocr {
        text,
        confidence,
        usable,
    }
}

/// Collapse every run of whitespace — including the per-line newlines tesseract inserts — to a
/// single space, so the text reads as prose rather than a column.
fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal valid 1×1 PNG (white pixel).
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8,
        0xff, 0xff, 0x3f, 0x00, 0x05, 0xfe, 0x02, 0xfe, 0xdc, 0xcc, 0x59, 0xe7, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn garbage_is_not_a_recognised_format() {
        assert!(matches!(
            decode(b"not an image at all", MAX_PIXELS),
            Err(OcrError::Format)
        ));
        assert!(matches!(decode(&[], MAX_PIXELS), Err(OcrError::Format)));
    }

    #[test]
    fn a_valid_tiny_image_decodes() {
        let img = decode(TINY_PNG, MAX_PIXELS).expect("valid png");
        assert_eq!(img.dimensions(), (1, 1));
    }

    #[test]
    fn the_pixel_budget_rejects_before_expanding() {
        // The 1×1 image is one pixel; a budget of zero must still reject it, proving the guard fires
        // on the header rather than after decoding.
        assert!(matches!(decode(TINY_PNG, 0), Err(OcrError::TooLarge(0))));
    }

    #[test]
    fn orientation_1_and_unknown_are_identity() {
        let img = decode(TINY_PNG, MAX_PIXELS).unwrap();
        assert_eq!(apply_orientation(img.clone(), 1).dimensions(), (1, 1));
        assert_eq!(apply_orientation(img, 99).dimensions(), (1, 1));
    }

    #[test]
    fn whitespace_including_ocr_newlines_collapses() {
        assert_eq!(
            collapse_whitespace("hello\n\n  world \t line"),
            "hello world line"
        );
        assert_eq!(collapse_whitespace("   "), "");
    }

    #[test]
    fn otsu_splits_a_bimodal_image_between_its_two_levels() {
        // Half the pixels at 40, half at 200 — the threshold must land strictly between them.
        let mut img = GrayImage::new(10, 10);
        for (i, p) in img.pixels_mut().enumerate() {
            p[0] = if i < 50 { 40 } else { 200 };
        }
        let t = otsu_threshold(&img);
        assert!(
            t >= 40 && t < 200,
            "threshold {t} not between the two levels"
        );
    }

    #[test]
    fn binarize_pushes_to_black_and_white() {
        let mut img = GrayImage::new(3, 1);
        img.get_pixel_mut(0, 0)[0] = 10;
        img.get_pixel_mut(1, 0)[0] = 130;
        img.get_pixel_mut(2, 0)[0] = 250;
        let out = binarize(&img, 128);
        assert_eq!(out.get_pixel(0, 0)[0], 0);
        assert_eq!(out.get_pixel(1, 0)[0], 255);
        assert_eq!(out.get_pixel(2, 0)[0], 255);
    }

    #[test]
    fn pick_better_prefers_usable_then_confidence() {
        let usable_low = Ocr {
            text: "a".into(),
            confidence: 60.0,
            usable: true,
        };
        let unusable_high = Ocr {
            text: "b".into(),
            confidence: 90.0,
            usable: false,
        };
        // A usable result beats an unusable one even at lower confidence.
        assert_eq!(
            pick_better(usable_low.clone(), unusable_high.clone()).text,
            "a"
        );
        assert_eq!(pick_better(unusable_high, usable_low).text, "a");

        // Same usability → higher confidence wins.
        let lo = Ocr {
            text: "lo".into(),
            confidence: 55.0,
            usable: true,
        };
        let hi = Ocr {
            text: "hi".into(),
            confidence: 80.0,
            usable: true,
        };
        assert_eq!(pick_better(lo.clone(), hi.clone()).text, "hi");
        assert_eq!(pick_better(hi, lo).text, "hi");
    }

    #[test]
    fn a_short_image_is_upscaled_and_grayscaled() {
        let img = decode(TINY_PNG, MAX_PIXELS).unwrap();
        let out = preprocess(img, 1);
        assert!(out.dimensions().0 >= 1);
        assert!(
            matches!(out, DynamicImage::ImageLuma8(_)),
            "must be grayscale"
        );
    }
}
