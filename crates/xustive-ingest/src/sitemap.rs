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
