//! Perceptual image hashing (dHash).
//!
//! A 64-bit fingerprint of an image's *structure*, robust to rescaling and mild recompression: two
//! visually identical images hash the same, and near-duplicates hash close (small Hamming distance).
//! It is not a cryptographic hash — that is the point. A byte hash changes when a single pixel does;
//! this stays put, which is what makes it useful for finding the same image reposted at a different
//! size ([[Deduplication Service]] §4.4) and for skipping redundant CLIP embeddings ([[Vector Index]]
//! §5).
//!
//! # dHash, specifically
//!
//! Grayscale, resize to 9×8, and for each row emit a bit per adjacent pixel pair: 1 if the left is
//! brighter than the right. Eight rows × eight comparisons = 64 bits. Comparing *gradients* rather
//! than absolute brightness is what makes it survive a global exposure or contrast shift.
//!
//! In memory only, like the rest of this crate — no file is ever written.

use image::imageops::FilterType;

use crate::ocr::{self, OcrError};

/// Resize width/height for the 8×8 gradient (one extra column for the horizontal difference).
const HASH_W: u32 = 9;
const HASH_H: u32 = 8;

/// Compute the dHash of an encoded image, as 16 lowercase hex characters (64 bits).
///
/// Returns `None` on an undecodable or oversized image — a fingerprint is an enrichment, and a bad
/// image simply gets none rather than failing anything.
pub fn dhash(bytes: &[u8], max_pixels: u64) -> Option<String> {
    let img = ocr::decode(bytes, max_pixels).ok()?;
    Some(dhash_image(&img))
}

/// The dHash of an already-decoded image. Split out so it is unit-testable without encoding.
pub fn dhash_image(img: &image::DynamicImage) -> String {
    // Resize on the grayscale image; Triangle is cheap and smooth enough for a structural hash.
    let small = image::DynamicImage::ImageLuma8(img.to_luma8()).resize_exact(
        HASH_W,
        HASH_H,
        FilterType::Triangle,
    );
    let luma = small.to_luma8();

    let mut bits: u64 = 0;
    for y in 0..HASH_H {
        for x in 0..(HASH_W - 1) {
            let left = luma.get_pixel(x, y)[0];
            let right = luma.get_pixel(x + 1, y)[0];
            bits <<= 1;
            if left > right {
                bits |= 1;
            }
        }
    }
    format!("{bits:016x}")
}

/// Hamming distance between two dHash hex strings — the number of differing bits (0–64).
///
/// Returns `None` if either string is not a valid 16-char hex hash. A distance of 0 is an exact
/// structural match; small distances (≲ 10) are near-duplicates.
pub fn hamming(a: &str, b: &str) -> Option<u32> {
    let a = u64::from_str_radix(a, 16).ok()?;
    let b = u64::from_str_radix(b, 16).ok()?;
    Some((a ^ b).count_ones())
}

/// Decode an image only far enough to fingerprint it — a thin wrapper matching [`ocr::recognise`]'s
/// error type, for callers that want to distinguish "bad image" from "no hash".
pub fn try_dhash(bytes: &[u8], max_pixels: u64) -> Result<String, OcrError> {
    let img = ocr::decode(bytes, max_pixels)?;
    Ok(dhash_image(&img))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbImage};

    fn solid(w: u32, h: u32, v: u8) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, image::Rgb([v, v, v])))
    }

    /// A right-to-left (decreasing) gradient: each pixel is brighter than the one to its right, so
    /// every adjacent comparison sets a bit — the hash is non-trivial, not all-zero.
    fn gradient(w: u32, h: u32) -> DynamicImage {
        let mut img = RgbImage::new(w, h);
        for (x, _y, px) in img.enumerate_pixels_mut() {
            let v = (255 - (x * 255) / w.max(1)) as u8;
            *px = image::Rgb([v, v, v]);
        }
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn a_flat_image_hashes_to_zero() {
        // No gradient anywhere: every "is the left brighter" comparison is false.
        assert_eq!(dhash_image(&solid(64, 64, 128)), "0000000000000000");
    }

    #[test]
    fn identical_images_hash_identically() {
        let a = gradient(200, 120);
        let b = gradient(200, 120);
        assert_eq!(dhash_image(&a), dhash_image(&b));
    }

    #[test]
    fn a_rescaled_image_hashes_close() {
        // The same gradient at half size must be a near-duplicate: dHash is scale-robust.
        let full = gradient(400, 240);
        let half = gradient(200, 120);
        let d = hamming(&dhash_image(&full), &dhash_image(&half)).unwrap();
        assert!(
            d <= 4,
            "rescaled duplicate should be near, got distance {d}"
        );
    }

    #[test]
    fn different_structure_hashes_far() {
        // A brightness gradient vs a flat field differ in most gradient bits.
        let g = dhash_image(&gradient(200, 120));
        let f = dhash_image(&solid(200, 120, 128));
        let d = hamming(&g, &f).unwrap();
        assert!(d >= 8, "distinct images should be far, got distance {d}");
    }

    #[test]
    fn hamming_of_equal_is_zero_and_of_all_bits_is_64() {
        assert_eq!(hamming("ffffffffffffffff", "ffffffffffffffff"), Some(0));
        assert_eq!(hamming("ffffffffffffffff", "0000000000000000"), Some(64));
        assert_eq!(hamming("not hex", "0000000000000000"), None);
    }
}
