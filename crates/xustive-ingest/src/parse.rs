//! HTML to canonical `Document`.
//!
//! Extraction runs as a cascade, best source first, and records which one succeeded so that a
//! site redesign shows up as a shift in `extraction_method` rather than as silently worse
//! results.
//!
//! 1. **JSON-LD** (`schema.org/NewsArticle`) — by far the best dates and authors when present.
//! 2. **OpenGraph / Twitter cards** — near-universal, good titles and descriptions.
//! 3. **Density-based extraction** — find the block with the most text and the fewest links.
//! 4. **Fallback** — `<title>` plus every paragraph.

use scraper::{Html, Selector};
use serde_json::Value;

use xustive_core::{
    hash, new_id, now_unix, BodySource, DatePrecision, Document, Lang, Media, MediaKind, Script,
    SourceType,
};
use xustive_lang::Detector;
use xustive_text::script as text_script;

use crate::date;

/// Which strategy produced the body. Tracked so extraction quality is observable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    JsonLd,
    OpenGraph,
    Density,
    Fallback,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::JsonLd => "json-ld",
            Self::OpenGraph => "opengraph",
            Self::Density => "density",
            Self::Fallback => "fallback",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParseConfig {
    pub excerpt_chars: usize,
    pub max_body_bytes: usize,
    pub max_media: usize,
    /// Below this many characters a page has no content worth indexing.
    pub min_body_chars: usize,
    pub max_outlinks: usize,
}

impl Default for ParseConfig {
    fn default() -> Self {
        Self {
            excerpt_chars: 320,
            max_body_bytes: 200 * 1024,
            max_media: 4,
            min_body_chars: 120,
            max_outlinks: 200,
        }
    }
}

#[derive(Debug)]
pub struct Parsed {
    pub document: Document,
    pub outlinks: Vec<String>,
    pub method: Method,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("no usable content ({chars} chars, minimum {min})")]
    TooLittleContent { chars: usize, min: usize },
    #[error("page is marked noindex")]
    NoIndex,
}

pub struct Parser {
    detector: Detector,
    config: ParseConfig,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new(ParseConfig::default())
    }
}

impl Parser {
    pub fn new(config: ParseConfig) -> Self {
        Self {
            detector: Detector::default(),
            config,
        }
    }

    /// Parse a fetched page into a `Document`.
    pub fn parse(
        &self,
        html: &str,
        url: &str,
        source_id: &str,
        source_type: SourceType,
    ) -> Result<Parsed, ParseError> {
        let doc = Html::parse_document(html);
        let now = now_unix();

        if is_noindex(&doc) {
            return Err(ParseError::NoIndex);
        }

        let jsonld = extract_jsonld(&doc);

        // Title, best source first.
        let title = first_non_empty([
            jsonld.as_ref().and_then(|j| string_field(j, "headline")),
            meta(&doc, "property", "og:title"),
            meta(&doc, "name", "twitter:title"),
            text_of(&doc, "h1"),
            text_of(&doc, "title"),
        ])
        .map(|t| clean(&t))
        .unwrap_or_default();

        // Body, with the method recorded.
        let (body, method) = self.extract_body(&doc, jsonld.as_ref());
        let body = truncate_bytes(&body, self.config.max_body_bytes);

        if body.chars().count() < self.config.min_body_chars {
            return Err(ParseError::TooLittleContent {
                chars: body.chars().count(),
                min: self.config.min_body_chars,
            });
        }

        // Date. Anything we cannot parse becomes the crawl time with `Unknown` precision — never
        // a guess presented as fact, because ranking discounts what it knows is uncertain.
        let published = first_non_empty([
            jsonld
                .as_ref()
                .and_then(|j| string_field(j, "datePublished")),
            meta(&doc, "property", "article:published_time"),
            meta(&doc, "name", "publish-date"),
            meta(&doc, "itemprop", "datePublished"),
            attr_of(&doc, "time", "datetime"),
            text_of(&doc, "time"),
        ])
        .and_then(|s| date::parse(&s, now));

        let (published_at, precision) = match published {
            Some(d) => (d.unix, d.precision),
            None => (now, DatePrecision::Unknown),
        };

        let excerpt = first_non_empty([
            meta(&doc, "property", "og:description"),
            meta(&doc, "name", "description"),
        ])
        .map(|d| clean(&d))
        .filter(|d| d.chars().count() > 40)
        .unwrap_or_else(|| excerpt_from(&body, self.config.excerpt_chars));

        let detection = self
            .detector
            .detect(&format!("{title} {}", head(&body, 2000)));

        let author = first_non_empty([
            jsonld.as_ref().and_then(author_of),
            meta(&doc, "name", "author"),
            meta(&doc, "property", "article:author"),
            text_of(&doc, ".author, .byline, [rel=author]"),
        ]);

        let canonical = link_rel(&doc, "canonical").unwrap_or_else(|| url.to_string());
        let media = self.extract_media(&doc, url);

        let mut document = Document::new(new_id(), url, source_type);
        document.canonical_url = Some(canonical);
        document.source_id = source_id.to_string();
        document.title = if title.is_empty() {
            head(&body, 80)
        } else {
            title
        };
        document.excerpt = excerpt;
        document.body_len = body.chars().count();
        document.content_hash = hash::content_hash(&body);
        document.simhash = hash::simhash(&body).map(hash::simhash_hex);
        document.body = body;
        document.body_source = BodySource::Native;
        document.language = detection.lang;
        document.language_confidence = detection.confidence;
        document.script = match detection.script {
            text_script::Script::Arabic => Script::Arabic,
            text_script::Script::Latin => Script::Latin,
            text_script::Script::Mixed => Script::Mixed,
            text_script::Script::Unknown => Script::Unknown,
        };
        document.author.name = author;
        document.published_at = published_at;
        document.published_at_precision = precision;
        document.crawled_at = now;
        document.indexed_at = now;
        document.media = media;
        document.entities = self.extract_entities(&document.title, &document.body);
        document.http_status = 200;
        document.fetch_method = Some("static".into());
        document.access_path = Some(method.as_str().into());
        document.quality_score = quality_score(&document, method);

        Ok(Parsed {
            outlinks: extract_outlinks(&doc, url, self.config.max_outlinks),
            document,
            method,
        })
    }

    /// The extraction cascade.
    fn extract_body(&self, doc: &Html, jsonld: Option<&Value>) -> (String, Method) {
        if let Some(text) = jsonld.and_then(|j| string_field(j, "articleBody")) {
            if text.chars().count() >= self.config.min_body_chars {
                return (clean(&text), Method::JsonLd);
            }
        }

        // Density: the container with the most text and the fewest links. Cheap, and it beats
        // per-site selectors on the long tail of sites nobody has written rules for.
        if let Some(text) = densest_block(doc) {
            if text.chars().count() >= self.config.min_body_chars {
                return (text, Method::Density);
            }
        }

        if let Some(desc) = meta(doc, "property", "og:description") {
            if desc.chars().count() >= self.config.min_body_chars {
                return (clean(&desc), Method::OpenGraph);
            }
        }

        (all_paragraphs(doc), Method::Fallback)
    }

    fn extract_media(&self, doc: &Html, base: &str) -> Vec<Media> {
        let mut out = Vec::new();
        if let Some(img) = meta(doc, "property", "og:image") {
            if let Some(abs) = absolutise(&img, base) {
                out.push(Media {
                    kind: MediaKind::Image,
                    url: abs,
                    thumb_url: None,
                    width: 0,
                    height: 0,
                    ocr_text: None,
                    ocr_lang: None,
                    embedding_id: None,
                    phash: None,
                });
            }
        }
        if let Ok(sel) = Selector::parse("article img, main img, .content img") {
            for el in doc.select(&sel).take(self.config.max_media) {
                if out.len() >= self.config.max_media {
                    break;
                }
                let Some(src) = el.value().attr("src") else {
                    continue;
                };
                // Skip tracking pixels and spacers.
                let too_small = el
                    .value()
                    .attr("width")
                    .and_then(|w| w.parse::<u32>().ok())
                    .is_some_and(|w| w < 200);
                if too_small {
                    continue;
                }
                if let Some(abs) = absolutise(src, base) {
                    if !out.iter().any(|m: &Media| m.url == abs) {
                        out.push(Media {
                            kind: MediaKind::Image,
                            url: abs,
                            thumb_url: None,
                            width: 0,
                            height: 0,
                            ocr_text: None,
                            ocr_lang: None,
                            embedding_id: None,
                            phash: None,
                        });
                    }
                }
            }
        }
        out
    }

    /// Capitalised sequences and Arabic proper nouns, filtered against a stoplist.
    ///
    /// A gazetteer rather than a model: cheap, explainable, and good enough to feed the
    /// `entities` field that typo tolerance is disabled on.
    fn extract_entities(&self, title: &str, body: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let text = format!("{title} {}", head(body, 4000));
        for token in text.split_whitespace() {
            let t = token.trim_matches(|c: char| !c.is_alphanumeric());
            if t.chars().count() < 3 || t.chars().count() > 40 {
                continue;
            }
            let is_entity = t
                .chars()
                .next()
                .is_some_and(|c| c.is_uppercase() && c.is_alphabetic());
            if is_entity && !out.iter().any(|e| e == t) {
                out.push(t.to_string());
            }
            if out.len() >= 20 {
                break;
            }
        }
        out
    }
}

// --- extraction helpers ------------------------------------------------------------------

fn is_noindex(doc: &Html) -> bool {
    meta(doc, "name", "robots")
        .map(|v| v.to_lowercase().contains("noindex"))
        .unwrap_or(false)
}

fn meta(doc: &Html, attr: &str, value: &str) -> Option<String> {
    let sel = Selector::parse(&format!(r#"meta[{attr}="{value}"]"#)).ok()?;
    doc.select(&sel)
        .find_map(|el| el.value().attr("content"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn link_rel(doc: &Html, rel: &str) -> Option<String> {
    let sel = Selector::parse(&format!(r#"link[rel="{rel}"]"#)).ok()?;
    doc.select(&sel)
        .find_map(|el| el.value().attr("href"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn text_of(doc: &Html, selector: &str) -> Option<String> {
    let sel = Selector::parse(selector).ok()?;
    doc.select(&sel)
        .next()
        .map(|el| clean(&el.text().collect::<String>()))
        .filter(|s| !s.is_empty())
}

fn attr_of(doc: &Html, selector: &str, attr: &str) -> Option<String> {
    let sel = Selector::parse(selector).ok()?;
    doc.select(&sel)
        .find_map(|el| el.value().attr(attr))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// JSON-LD article node, if the page has one.
fn extract_jsonld(doc: &Html) -> Option<Value> {
    let sel = Selector::parse(r#"script[type="application/ld+json"]"#).ok()?;
    for el in doc.select(&sel) {
        let raw = el.text().collect::<String>();
        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        // The node may be a bare object, a @graph, or an array.
        let candidates: Vec<&Value> = match &v {
            Value::Array(a) => a.iter().collect(),
            Value::Object(o) => match o.get("@graph") {
                Some(Value::Array(a)) => a.iter().collect(),
                _ => vec![&v],
            },
            _ => continue,
        };
        for c in candidates {
            let ty = c.get("@type").and_then(|t| match t {
                Value::String(s) => Some(s.clone()),
                Value::Array(a) => a.first().and_then(|x| x.as_str()).map(str::to_string),
                _ => None,
            });
            if ty.is_some_and(|t| {
                let t = t.to_lowercase();
                t.contains("article") || t.contains("newsarticle") || t.contains("blogposting")
            }) {
                return Some(c.clone());
            }
        }
    }
    None
}

fn string_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn author_of(v: &Value) -> Option<String> {
    match v.get("author")? {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o.get("name").and_then(|n| n.as_str()).map(str::to_string),
        Value::Array(a) => a.first().and_then(|f| match f {
            Value::String(s) => Some(s.clone()),
            Value::Object(o) => o.get("name").and_then(|n| n.as_str()).map(str::to_string),
            _ => None,
        }),
        _ => None,
    }
}

/// The block with the most text and the lowest link density.
///
/// Navigation and related-links boxes are mostly anchor text, so weighting text against link
/// density separates article bodies from chrome without knowing anything about the site.
fn densest_block(doc: &Html) -> Option<String> {
    let sel = Selector::parse("article, main, [role=main], .article-content, .post-content, .entry-content, #content, .content, div")
        .ok()?;
    let link_sel = Selector::parse("a").ok()?;

    let mut best: Option<(f32, String)> = None;
    for el in doc.select(&sel) {
        let text = clean(&el.text().collect::<String>());
        let len = text.chars().count();
        if len < 200 {
            continue;
        }
        let link_chars: usize = el
            .select(&link_sel)
            .map(|a| a.text().collect::<String>().chars().count())
            .sum();
        let link_density = link_chars as f32 / len as f32;
        if link_density > 0.5 {
            continue;
        }
        let score = len as f32 * (1.0 - link_density);
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((score, text));
        }
    }
    best.map(|(_, t)| t)
}

fn all_paragraphs(doc: &Html) -> String {
    let Ok(sel) = Selector::parse("p") else {
        return String::new();
    };
    let joined: Vec<String> = doc
        .select(&sel)
        .map(|el| clean(&el.text().collect::<String>()))
        .filter(|t| t.chars().count() > 20)
        .collect();
    joined.join(" ")
}

fn extract_outlinks(doc: &Html, base: &str, max: usize) -> Vec<String> {
    let Ok(sel) = Selector::parse("a[href]") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for el in doc.select(&sel) {
        if out.len() >= max {
            break;
        }
        let Some(href) = el.value().attr("href") else {
            continue;
        };
        if href.starts_with('#') || href.starts_with("javascript:") || href.starts_with("mailto:") {
            continue;
        }
        if let Some(abs) = absolutise(href, base) {
            if !out.contains(&abs) {
                out.push(abs);
            }
        }
    }
    out
}

fn absolutise(href: &str, base: &str) -> Option<String> {
    let base = url::Url::parse(base).ok()?;
    let joined = base.join(href).ok()?;
    matches!(joined.scheme(), "http" | "https").then(|| joined.to_string())
}

/// Collapse whitespace and strip any markup that survived extraction.
///
/// Meta descriptions in particular are not reliably plain text — publishers paste article HTML
/// into `og:description`, and an excerpt containing a literal `<p>` reaches the results page and
/// is rendered as visible text by the escaping layer, which is correct behaviour producing a
/// visibly wrong result.
fn clean(s: &str) -> String {
    let stripped = strip_tags(s);
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Remove anything that looks like an HTML tag, and decode the handful of entities that appear
/// in practice.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for ch in s.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Truncate on a char boundary, respecting a byte budget.
fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// First `n` chars, cut at a sentence boundary where one is close by.
fn excerpt_from(body: &str, n: usize) -> String {
    let head: String = body.chars().take(n).collect();
    if body.chars().count() <= n {
        return head;
    }
    match head.rfind(['.', '؟', '?', '!', '۔']) {
        Some(i) if i > n / 2 => head[..=i].to_string(),
        _ => format!("{head}…"),
    }
}

fn first_non_empty<const N: usize>(candidates: [Option<String>; N]) -> Option<String> {
    candidates
        .into_iter()
        .flatten()
        .find(|s| !s.trim().is_empty())
}

/// Cheap, explainable quality signal. Feeds ranking and spam suppression.
fn quality_score(doc: &Document, method: Method) -> f32 {
    let mut score: f32 = 0.0;

    // Longer is better, with sharply diminishing returns past a few thousand characters.
    let len = doc.body_len as f32;
    score += 0.25 * (len / 3000.0).min(1.0);

    // A date we actually know is worth a lot: it is what makes freshness ranking meaningful.
    score += match doc.published_at_precision {
        DatePrecision::Second => 0.20,
        DatePrecision::Day => 0.18,
        DatePrecision::Month => 0.10,
        DatePrecision::Unknown => 0.0,
    };

    // Structured markup means the publisher cares about being read correctly.
    score += match method {
        Method::JsonLd => 0.20,
        Method::OpenGraph => 0.12,
        Method::Density => 0.10,
        Method::Fallback => 0.02,
    };

    if !doc.title.is_empty() && doc.title.chars().count() > 15 {
        score += 0.10;
    }
    if doc.author.name.is_some() {
        score += 0.08;
    }
    if !doc.media.is_empty() {
        score += 0.05;
    }
    if doc.language != Lang::Und {
        score += 0.12;
    }

    score.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> Parser {
        Parser::default()
    }

    const LONG_AR: &str = "أعلنت المصالح المعنية عن إجراءات جديدة تهدف إلى تبسيط العملية على \
        المواطنين حيث سيتم اعتماد المنصة الرقمية بشكل كامل بداية من الشهر المقبل وأوضح المسؤول \
        في تصريح للصحافة أن العملية ستمس عددا كبيرا من المستفيدين عبر مختلف ولايات الوطن مؤكدا \
        أن الآجال ستحترم بشكل صارم وفق ما تم الإعلان عنه سابقا في بيان رسمي";

    #[test]
    fn extracts_from_json_ld_when_present() {
        let html = format!(
            r#"<html><head><script type="application/ld+json">
            {{"@type":"NewsArticle","headline":"عنوان المقال",
              "datePublished":"2026-08-04T10:00:00+01:00",
              "author":{{"name":"محرر الجريدة"}},
              "articleBody":"{LONG_AR}"}}
            </script></head><body><p>ignored</p></body></html>"#
        );
        let r = p()
            .parse(&html, "https://example.dz/a", "test", SourceType::Web)
            .unwrap();
        assert_eq!(r.method, Method::JsonLd);
        assert_eq!(r.document.title, "عنوان المقال");
        assert_eq!(r.document.author.name.as_deref(), Some("محرر الجريدة"));
        assert_eq!(r.document.published_at_precision, DatePrecision::Second);
    }

    #[test]
    fn handles_json_ld_inside_a_graph() {
        let html = format!(
            r#"<html><head><script type="application/ld+json">
            {{"@graph":[{{"@type":"WebSite"}},
                        {{"@type":"NewsArticle","headline":"من الرسم البياني",
                          "articleBody":"{LONG_AR}"}}]}}
            </script></head><body></body></html>"#
        );
        let r = p()
            .parse(&html, "https://example.dz/a", "test", SourceType::Web)
            .unwrap();
        assert_eq!(r.document.title, "من الرسم البياني");
    }

    #[test]
    fn falls_back_to_density_extraction() {
        let html = format!(
            r#"<html><head><title>عنوان</title></head><body>
              <nav><a href="/1">رابط</a><a href="/2">رابط</a><a href="/3">رابط</a></nav>
              <article><p>{LONG_AR}</p></article>
              <footer><a href="/x">تذييل</a></footer>
            </body></html>"#
        );
        let r = p()
            .parse(&html, "https://example.dz/a", "test", SourceType::Web)
            .unwrap();
        assert_eq!(r.method, Method::Density);
        assert!(r.document.body.contains("المصالح المعنية"));
        assert!(
            !r.document.body.contains("تذييل"),
            "footer leaked into the body"
        );
    }

    #[test]
    fn navigation_heavy_blocks_are_rejected_by_link_density() {
        let mut links = String::new();
        for i in 0..80 {
            links.push_str(&format!(r#"<a href="/{i}">قسم من الأقسام الكثيرة هنا</a> "#));
        }
        let html = format!(
            r#"<html><body><div class="nav">{links}</div>
               <article><p>{LONG_AR}</p></article></body></html>"#
        );
        let r = p()
            .parse(&html, "https://example.dz/a", "test", SourceType::Web)
            .unwrap();
        assert!(
            r.document.body.contains("المصالح المعنية"),
            "picked the nav block instead of the article"
        );
    }

    #[test]
    fn a_page_with_no_content_is_rejected() {
        let html = "<html><head><title>x</title></head><body><p>short</p></body></html>";
        let err = p()
            .parse(html, "https://example.dz/a", "test", SourceType::Web)
            .unwrap_err();
        assert!(matches!(err, ParseError::TooLittleContent { .. }));
    }

    #[test]
    fn noindex_is_honoured() {
        let html = format!(
            r#"<html><head><meta name="robots" content="noindex, follow">
               </head><body><article><p>{LONG_AR}</p></article></body></html>"#
        );
        assert!(matches!(
            p().parse(&html, "https://example.dz/a", "t", SourceType::Web),
            Err(ParseError::NoIndex)
        ));
    }

    #[test]
    fn an_unparseable_date_is_marked_unknown_not_guessed() {
        // The important one: a crawl date presented as a publication date makes freshness
        // ranking a lie, so the uncertainty has to survive into the document.
        let html = format!("<html><body><article><p>{LONG_AR}</p></article></body></html>");
        let r = p()
            .parse(&html, "https://example.dz/a", "t", SourceType::Web)
            .unwrap();
        assert_eq!(r.document.published_at_precision, DatePrecision::Unknown);
        assert!(!r.document.is_date_trustworthy());
    }

    #[test]
    fn algerian_dates_in_meta_tags_are_parsed() {
        let html = format!(
            r#"<html><head><meta property="article:published_time" content="2026-08-04T09:15:00+01:00">
               </head><body><article><p>{LONG_AR}</p></article></body></html>"#
        );
        let r = p()
            .parse(&html, "https://example.dz/a", "t", SourceType::Web)
            .unwrap();
        assert_eq!(r.document.published_at_precision, DatePrecision::Second);
        assert!(r.document.is_date_trustworthy());
    }

    #[test]
    fn language_is_detected_from_the_extracted_text() {
        let html = format!("<html><body><article><p>{LONG_AR}</p></article></body></html>");
        let r = p()
            .parse(&html, "https://example.dz/a", "t", SourceType::Web)
            .unwrap();
        assert_eq!(r.document.language, Lang::Ar);
        assert_eq!(r.document.script, Script::Arabic);
    }

    #[test]
    fn darija_pages_are_detected_as_darija() {
        let body = "واش راكم خاوتي، شكون يعرف كيفاش ندير هاد الإجراء؟ بزاف الناس راهم يسقسيو \
                    على هاد الموضوع، نحاول نشرح بالتفصيل باش يفهم الجميع وين لازم يروح";
        let html = format!("<html><body><article><p>{body}</p></article></body></html>");
        let r = p()
            .parse(&html, "https://example.dz/a", "t", SourceType::Web)
            .unwrap();
        assert_eq!(r.document.language, Lang::Ary);
    }

    #[test]
    fn hashes_are_computed_for_deduplication() {
        let html = format!("<html><body><article><p>{LONG_AR}</p></article></body></html>");
        let r = p()
            .parse(&html, "https://example.dz/a", "t", SourceType::Web)
            .unwrap();
        assert!(r.document.content_hash.starts_with("b3:"));
        assert!(
            r.document.simhash.is_some(),
            "article-length text should have a simhash"
        );
    }

    #[test]
    fn the_same_content_at_two_urls_hashes_identically() {
        // What makes cross-posted content collapsible.
        let html = format!("<html><body><article><p>{LONG_AR}</p></article></body></html>");
        let a = p()
            .parse(&html, "https://a.dz/x", "a", SourceType::Web)
            .unwrap();
        let b = p()
            .parse(&html, "https://b.dz/y", "b", SourceType::Web)
            .unwrap();
        assert_eq!(a.document.content_hash, b.document.content_hash);
    }

    #[test]
    fn outlinks_are_absolutised_and_filtered() {
        // `r##"…"##`: the fragment link contains `"#`, which would close a single-hash
        // raw string early.
        let html = format!(
            r##"<html><body><article><p>{LONG_AR}</p></article>
               <a href="/rel">a</a><a href="https://other.dz/x">b</a>
               <a href="#frag">c</a><a href="javascript:void(0)">d</a>
               <a href="mailto:x@y.dz">e</a></body></html>"##
        );
        let r = p()
            .parse(&html, "https://example.dz/dir/page", "t", SourceType::Web)
            .unwrap();
        assert!(r.outlinks.contains(&"https://example.dz/rel".to_string()));
        assert!(r.outlinks.contains(&"https://other.dz/x".to_string()));
        assert!(!r.outlinks.iter().any(|l| l.contains("javascript")));
        assert!(!r.outlinks.iter().any(|l| l.contains("mailto")));
    }

    #[test]
    fn canonical_url_is_preferred_when_declared() {
        let html = format!(
            r#"<html><head><link rel="canonical" href="https://example.dz/canonical">
               </head><body><article><p>{LONG_AR}</p></article></body></html>"#
        );
        let r = p()
            .parse(
                &html,
                "https://example.dz/a?utm_source=x",
                "t",
                SourceType::Web,
            )
            .unwrap();
        assert_eq!(
            r.document.canonical_url.as_deref(),
            Some("https://example.dz/canonical")
        );
    }

    #[test]
    fn quality_rewards_structure_and_a_known_date() {
        let rich = format!(
            r#"<html><head><script type="application/ld+json">
            {{"@type":"NewsArticle","headline":"عنوان طويل بما فيه الكفاية للاختبار",
              "datePublished":"2026-08-04T10:00:00Z","author":{{"name":"محرر"}},
              "articleBody":"{LONG_AR}"}}</script></head><body></body></html>"#
        );
        let bare = format!("<html><body><p>{LONG_AR}</p></body></html>");

        let a = p()
            .parse(&rich, "https://example.dz/a", "t", SourceType::Web)
            .unwrap();
        let b = p()
            .parse(&bare, "https://example.dz/b", "t", SourceType::Web)
            .unwrap();
        assert!(
            a.document.quality_score > b.document.quality_score,
            "structured page scored {} vs bare {}",
            a.document.quality_score,
            b.document.quality_score
        );
    }

    #[test]
    fn malformed_html_does_not_panic() {
        for html in [
            "<html><body><article><p>unclosed",
            "<<<>>>",
            "",
            "<html><body>",
            &"<div>".repeat(500),
        ] {
            let _ = p().parse(html, "https://example.dz/a", "t", SourceType::Web);
        }
    }

    #[test]
    fn markup_never_reaches_the_excerpt_or_title() {
        // Publishers paste article HTML into og:description. The escaping layer would then
        // render a literal `<p>` as visible text — correct behaviour, visibly wrong result.
        let html = format!(
            r#"<html><head>
               <meta property="og:title" content="&lt;b&gt;عنوان&lt;/b&gt; مع وسوم">
               <meta property="og:description" content="<p>وصف طويل بما فيه الكفاية ليتجاوز الحد الأدنى المطلوب للاختبار</p>">
               </head><body><article><p>{LONG_AR}</p></article></body></html>"#
        );
        let r = p()
            .parse(&html, "https://example.dz/a", "t", SourceType::Web)
            .unwrap();
        assert!(
            !r.document.excerpt.contains('<'),
            "markup in excerpt: {}",
            r.document.excerpt
        );
        assert!(!r.document.excerpt.contains("&lt;"), "entity in excerpt");
        assert!(
            !r.document.title.contains('<'),
            "markup in title: {}",
            r.document.title
        );
    }

    #[test]
    fn strip_tags_handles_nesting_and_entities() {
        assert_eq!(strip_tags("<p>a<b>b</b></p>"), "ab");
        assert_eq!(strip_tags("a &amp; b"), "a & b");
        assert_eq!(strip_tags("a&nbsp;b"), "a b");
        assert_eq!(strip_tags("no markup"), "no markup");
    }

    #[test]
    fn excerpt_prefers_a_sentence_boundary() {
        let body = "جملة أولى قصيرة. جملة ثانية أطول بكثير من الأولى وتحتوي على تفاصيل إضافية.";
        let e = excerpt_from(body, 30);
        assert!(e.ends_with('.') || e.ends_with('…'));
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        let s = "الجزائر".repeat(100);
        let t = truncate_bytes(&s, 50);
        assert!(t.len() <= 50);
        assert!(std::str::from_utf8(t.as_bytes()).is_ok());
    }
}
