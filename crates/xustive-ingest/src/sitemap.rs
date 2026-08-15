//! Sitemap and RSS parsing.
//!
//! The cheapest way to find a site's content: a list the publisher maintains themselves, instead
//! of discovering URLs by following links and guessing which ones are articles.

use quick_xml::events::Event;
use quick_xml::Reader;

/// Extract URLs from a sitemap, sitemap index, or RSS/Atom feed.
///
/// One parser for all of them: they differ only in which element holds the URL, so matching on
/// the tag name covers every case without deciding the format up front.
pub fn extract_urls(xml: &str, max: usize) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut out: Vec<String> = Vec::new();
    let mut buf = Vec::new();
    let mut capture = false;

    loop {
        match reader.read_event_into(&mut buf) {
            // `Start` and `Empty` are handled identically. Atom writes `<link href="..."/>`,
            // which quick-xml reports as `Empty` rather than `Start`, so matching only on
            // `Start` silently skips every Atom feed.
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref());
                // <loc> is sitemaps; <link> and <guid> cover RSS and Atom.
                capture = matches!(name.as_str(), "loc" | "link" | "guid");

                // Atom puts the URL in an attribute rather than a text node.
                if name == "link" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"href" {
                            if let Ok(v) = attr.unescape_value() {
                                push_url(&mut out, v.trim(), max);
                            }
                        }
                    }
                }
            }
            Ok(Event::Text(e)) if capture => {
                if let Ok(t) = e.unescape() {
                    push_url(&mut out, t.trim(), max);
                }
            }
            Ok(Event::CData(e)) if capture => {
                if let Ok(s) = std::str::from_utf8(e.as_ref()) {
                    push_url(&mut out, s.trim(), max);
                }
            }
            Ok(Event::End(_)) => capture = false,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        if out.len() >= max {
            break;
        }
        buf.clear();
    }
    out
}

/// A sitemap entry, with the modification time the publisher stated (M2-T15.6).
///
/// `lastmod` is the highest-yield freshness signal there is: one fetch of a sitemap reports on
/// hundreds of URLs at once, and a page whose `lastmod` is no newer than our last fetch has not
/// changed — so it can be skipped entirely, which is cheaper even than a 304, because it is no
/// request at all.
///
/// It is a *hint*, not proof: plenty of sites stamp `lastmod` with the build time and move it on
/// every page nightly whether the content changed or not. So a newer `lastmod` schedules a visit;
/// it does not replace the content-hash comparison that decides whether anything actually moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub url: String,
    /// Unix seconds, when a parseable `<lastmod>` was present.
    pub lastmod: Option<i64>,
}

/// Extract `<url><loc>/<lastmod>` pairs from a sitemap.
///
/// Sitemaps only — `lastmod` is a sitemap element, and RSS/Atom carry their own date fields that
/// mean something subtly different (publication, not modification). Feeds still go through
/// [`extract_urls`], which is unchanged.
pub fn extract_entries(xml: &str, max: usize) -> Vec<Entry> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut out: Vec<Entry> = Vec::new();
    let mut buf = Vec::new();
    let mut loc: Option<String> = None;
    let mut lastmod: Option<i64> = None;
    let mut field: Field = Field::None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                field = match local_name(e.name().as_ref()).as_str() {
                    "loc" => Field::Loc,
                    "lastmod" => Field::Lastmod,
                    _ => Field::None,
                };
            }
            Ok(Event::Text(e)) => {
                let Ok(t) = e.unescape() else { continue };
                let t = t.trim();
                match field {
                    Field::Loc if t.starts_with("http") => loc = Some(t.to_string()),
                    Field::Lastmod => lastmod = parse_w3c_date(t),
                    _ => {}
                }
            }
            // `</url>` closes one entry. Emitting on the closing `<url>` rather than on the next
            // `<loc>` keeps a `lastmod` bound to the URL it belongs to even when the two are
            // separated by other elements.
            Ok(Event::End(e)) => {
                if local_name(e.name().as_ref()) == "url" {
                    if let Some(url) = loc.take() {
                        out.push(Entry { url, lastmod });
                    }
                    lastmod = None;
                }
                field = Field::None;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        if out.len() >= max {
            break;
        }
        buf.clear();
    }
    out
}

enum Field {
    None,
    Loc,
    Lastmod,
}

/// Parse a W3C datetime (the sitemap date format) to unix seconds.
///
/// Accepts the two forms sitemaps actually use: a bare date `2026-08-15`, and a full timestamp
/// `2026-08-15T09:30:00+01:00`. A bare date is read as midnight UTC — good enough for "is this
/// newer than our last fetch", which is the only question asked of it.
fn parse_w3c_date(s: &str) -> Option<i64> {
    let date = s.get(0..10)?;
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }

    // Days since the epoch by the civil-from-days algorithm (Howard Hinnant). No chrono dependency
    // for one conversion, and it is exact for any Gregorian date.
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    let secs = days * 86_400;

    // Add the time of day when present, ignoring the zone offset — a few hours cannot change the
    // "newer than last fetch" answer, and parsing offsets by hand invites the bugs this avoids.
    if let Some(time) = s.get(11..19) {
        let mut t = time.split(':');
        let (Some(hh), Some(mm), Some(ss)) = (t.next(), t.next(), t.next()) else {
            return Some(secs);
        };
        if let (Ok(hh), Ok(mm), Ok(ss)) = (hh.parse::<i64>(), mm.parse::<i64>(), ss.parse::<i64>())
        {
            return Some(secs + hh * 3600 + mm * 60 + ss);
        }
    }
    Some(secs)
}

/// True when the document is a sitemap index (a list of other sitemaps) rather than of pages.
pub fn is_index(xml: &str) -> bool {
    xml.contains("<sitemapindex") || xml.contains(":sitemapindex")
}

fn local_name(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    s.rsplit(':').next().unwrap_or(&s).to_ascii_lowercase()
}

fn push_url(out: &mut Vec<String>, candidate: &str, max: usize) {
    if out.len() >= max {
        return;
    }
    if !candidate.starts_with("http://") && !candidate.starts_with("https://") {
        return;
    }
    let url = candidate.to_string();
    if !out.contains(&url) {
        out.push(url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_urlset() {
        let xml = r#"<?xml version="1.0"?>
        <urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
          <url><loc>https://a.dz/1</loc><lastmod>2026-08-01</lastmod></url>
          <url><loc>https://a.dz/2</loc></url>
        </urlset>"#;
        assert_eq!(
            extract_urls(xml, 100),
            vec!["https://a.dz/1", "https://a.dz/2"]
        );
        assert!(!is_index(xml));
    }

    #[test]
    fn detects_and_parses_a_sitemap_index() {
        let xml = r#"<?xml version="1.0"?>
        <sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
          <sitemap><loc>https://a.dz/sitemap-1.xml</loc></sitemap>
        </sitemapindex>"#;
        assert!(is_index(xml));
        assert_eq!(extract_urls(xml, 100), vec!["https://a.dz/sitemap-1.xml"]);
    }

    #[test]
    fn parses_rss() {
        let xml = r#"<rss version="2.0"><channel>
          <link>https://a.dz/</link>
          <item><title>x</title><link>https://a.dz/article/1</link></item>
          <item><title>y</title><link>https://a.dz/article/2</link></item>
        </channel></rss>"#;
        let urls = extract_urls(xml, 100);
        assert!(urls.contains(&"https://a.dz/article/1".to_string()));
        assert!(urls.contains(&"https://a.dz/article/2".to_string()));
    }

    #[test]
    fn parses_atom_where_the_url_is_an_attribute() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom">
          <entry><link href="https://a.dz/atom/1"/></entry>
        </feed>"#;
        assert!(extract_urls(xml, 100).contains(&"https://a.dz/atom/1".to_string()));
    }

    #[test]
    fn handles_cdata_and_namespaced_tags() {
        let xml = r#"<urlset xmlns:s="http://x"><url><s:loc><![CDATA[https://a.dz/c]]></s:loc></url></urlset>"#;
        assert_eq!(extract_urls(xml, 10), vec!["https://a.dz/c"]);
    }

    #[test]
    fn non_http_and_duplicate_urls_are_dropped() {
        let xml = r#"<urlset>
          <url><loc>ftp://a.dz/x</loc></url>
          <url><loc>https://a.dz/1</loc></url>
          <url><loc>https://a.dz/1</loc></url>
        </urlset>"#;
        assert_eq!(extract_urls(xml, 10), vec!["https://a.dz/1"]);
    }

    #[test]
    fn respects_the_cap() {
        let mut xml = String::from("<urlset>");
        for i in 0..500 {
            xml.push_str(&format!("<url><loc>https://a.dz/{i}</loc></url>"));
        }
        xml.push_str("</urlset>");
        assert_eq!(extract_urls(&xml, 50).len(), 50);
    }

    #[test]
    fn malformed_xml_returns_what_it_could_read() {
        let xml = "<urlset><url><loc>https://a.dz/1</loc></url><url><loc>unclosed";
        let _ = extract_urls(xml, 10);
    }
}

#[cfg(test)]
mod lastmod_tests {
    use super::*;

    #[test]
    fn entries_pair_each_url_with_its_lastmod() {
        let xml = r#"<?xml version="1.0"?>
        <urlset>
          <url><loc>https://example.dz/a</loc><lastmod>2026-08-10</lastmod></url>
          <url><loc>https://example.dz/b</loc><lastmod>2026-08-15T09:30:00+01:00</lastmod></url>
          <url><loc>https://example.dz/c</loc></url>
        </urlset>"#;
        let e = extract_entries(xml, 100);
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].url, "https://example.dz/a");
        assert!(e[0].lastmod.is_some());
        assert!(
            e[1].lastmod.unwrap() > e[0].lastmod.unwrap(),
            "later date sorts later"
        );
        assert_eq!(
            e[2].lastmod, None,
            "a URL with no lastmod is None, not zero"
        );
    }

    /// A `lastmod` between the loc and the closing tag, or reordered, still binds correctly.
    #[test]
    fn lastmod_does_not_leak_between_entries() {
        let xml = r#"<urlset>
          <url><lastmod>2026-01-01</lastmod><loc>https://example.dz/dated</loc></url>
          <url><loc>https://example.dz/undated</loc></url>
        </urlset>"#;
        let e = extract_entries(xml, 100);
        assert_eq!(e[0].lastmod, Some(1_767_225_600));
        assert_eq!(
            e[1].lastmod, None,
            "the previous entry's date must not carry over"
        );
    }

    #[test]
    fn the_date_epoch_is_correct() {
        // 1970-01-01 is 0; 2026-08-15 is a value cross-checkable against `date -d`.
        assert_eq!(parse_w3c_date("1970-01-01"), Some(0));
        assert_eq!(parse_w3c_date("2000-03-01"), Some(951_868_800));
        assert_eq!(parse_w3c_date("not-a-date"), None);
        assert_eq!(parse_w3c_date("2026-13-01"), None, "month 13 is rejected");
    }
}
