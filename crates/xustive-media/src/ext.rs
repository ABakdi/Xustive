//! An image's file type, from its URL or from its first bytes (M10-T02.1).
//!
//! The chips on the reverse-image page filter by type, and a type nobody wrote down is a chip
//! nobody can click. The URL is the cheap guess at extraction time; the bytes are the truth once
//! the embed pass has fetched them — a `.jpg` that is a PNG is common enough on the web that the
//! sniff wins when the two disagree.

/// The types the page offers. `jpeg` is `jpg`; anything else is absent, not `unknown`.
pub const KNOWN: &[&str] = &["png", "jpg", "gif", "webp", "svg", "avif", "bmp", "tiff"];

/// The extension named by a URL's path, normalised, or `None`.
pub fn from_url(url: &str) -> Option<&'static str> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    normalise(&ext)
}

/// The type the bytes actually are, by magic number, or `None` when they are none of ours.
pub fn from_bytes(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && (&bytes[8..12] == b"avif" || &bytes[8..12] == b"avis")
    {
        Some("avif")
    } else if bytes.starts_with(b"BM") {
        Some("bmp")
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        Some("tiff")
    } else {
        // SVG is text; look for the root element in the first kilobyte.
        let head = &bytes[..bytes.len().min(1024)];
        let text = String::from_utf8_lossy(head);
        if text.contains("<svg") {
            Some("svg")
        } else {
            None
        }
    }
}

fn normalise(ext: &str) -> Option<&'static str> {
    match ext {
        "jpg" | "jpeg" | "jpe" | "jfif" => Some("jpg"),
        "tif" | "tiff" => Some("tiff"),
        other => KNOWN.iter().copied().find(|k| *k == other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_url_names_the_type_and_jpeg_is_jpg() {
        assert_eq!(from_url("https://x.dz/a/b.JPEG?w=1"), Some("jpg"));
        assert_eq!(from_url("https://x.dz/a/b.png#top"), Some("png"));
        assert_eq!(from_url("https://x.dz/a/image"), None);
        assert_eq!(from_url("https://x.dz/a/b.exe"), None);
    }

    #[test]
    fn the_bytes_win_over_the_name() {
        // A PNG served as .jpg is a PNG.
        assert_eq!(from_bytes(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(from_bytes(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(from_bytes(b"GIF89a......"), Some("gif"));
        assert_eq!(from_bytes(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(
            from_bytes(b"<?xml version=\"1.0\"?><svg xmlns=..."),
            Some("svg")
        );
        assert_eq!(from_bytes(b"hello"), None);
    }
}
