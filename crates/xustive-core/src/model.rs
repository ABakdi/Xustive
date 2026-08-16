//! The canonical entities every component agrees on.
//!
//! If a struct is not here, it is local to a component and must not cross a queue or an API
//! boundary. Field semantics that are easy to get wrong are documented on the field.

use serde::{Deserialize, Serialize};

/// Current schema version. Readers must tolerate `version <= SCHEMA_VERSION`.
pub const SCHEMA_VERSION: u32 = 1;

/// Where a document came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    Web,
    Facebook,
    Instagram,
    Tiktok,
}

impl SourceType {
    pub const ALL: [SourceType; 4] = [Self::Web, Self::Facebook, Self::Instagram, Self::Tiktok];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Facebook => "facebook",
            Self::Instagram => "instagram",
            Self::Tiktok => "tiktok",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "web" => Some(Self::Web),
            "facebook" | "fb" => Some(Self::Facebook),
            "instagram" | "ig" => Some(Self::Instagram),
            "tiktok" | "tt" => Some(Self::Tiktok),
            _ => None,
        }
    }

    /// Whether this source needs the `platform` crawl profile rather than `open_web`.
    pub const fn is_platform(self) -> bool {
        !matches!(self, Self::Web)
    }
}

/// Detected language. `Ary` is Algerian Darija, which is not the same as `Ar` for our purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Ar,
    Ary,
    Fr,
    En,
    Mixed,
    /// Undetermined. A *safe* answer: retrieval widens instead of narrowing wrongly.
    Und,
}

impl Lang {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ar => "ar",
            Self::Ary => "ary",
            Self::Fr => "fr",
            Self::En => "en",
            Self::Mixed => "mixed",
            Self::Und => "und",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ar" => Some(Self::Ar),
            "ary" | "darija" | "dz" => Some(Self::Ary),
            "fr" => Some(Self::Fr),
            "en" => Some(Self::En),
            "mixed" => Some(Self::Mixed),
            "und" | "auto" => Some(Self::Und),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Script {
    Arabic,
    Latin,
    Mixed,
    Unknown,
}

/// How precisely we know `published_at`.
///
/// `Unknown` means we fell back to the crawl date. Ranking halves the freshness contribution in
/// that case, and the UI renders "date unknown" rather than inventing a date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatePrecision {
    Second,
    Day,
    Month,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SentimentLabel {
    Positive,
    Neutral,
    Negative,
}

impl SentimentLabel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Neutral => "neutral",
            Self::Negative => "negative",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "positive" => Some(Self::Positive),
            "neutral" => Some(Self::Neutral),
            "negative" => Some(Self::Negative),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sentiment {
    pub label: SentimentLabel,
    /// −1.0 … +1.0
    pub score: f32,
    pub confidence: f32,
    /// Provenance, e.g. `"vader-dz@1"`. Lets us backfill selectively when the model changes.
    pub model: String,
}

impl Default for Sentiment {
    fn default() -> Self {
        Self {
            label: SentimentLabel::Neutral,
            score: 0.0,
            confidence: 0.0,
            model: "none".into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub verified: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Engagement {
    #[serde(default)]
    pub likes: u64,
    #[serde(default)]
    pub comments: u64,
    #[serde(default)]
    pub shares: u64,
    #[serde(default)]
    pub views: u64,
    #[serde(default)]
    pub captured_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Media {
    #[serde(rename = "type")]
    pub kind: MediaKind,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb_url: Option<String>,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// Text extracted by OCR. For image-first sources this often carries the real content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_lang: Option<String>,
    /// Foreign key into the vector index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_id: Option<String>,
    /// Perceptual hash, for image dedup and embedding reuse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phash: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Geo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wilaya: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wilaya_name: Option<String>,
}

/// Where `body` came from, when it is not the source's own text field.
///
/// Recorded rather than inferred, so a document whose text came from OCR is visibly different
/// from one that had a real caption — it matters for both ranking and debugging.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodySource {
    /// The platform's own text/caption field.
    #[default]
    Native,
    /// Backfilled from OCR because the caption was empty.
    Ocr,
    /// Caption plus platform-provided speech-to-text.
    CaptionAsr,
}

/// The primary indexed entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// ULID: time-sortable, so id order approximates ingestion order.
    pub id: String,
    /// BLAKE3 of the normalised body. Exact-duplicate key.
    pub content_hash: String,
    /// 64-bit SimHash as hex. Near-duplicate key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simhash: Option<String>,

    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_url: Option<String>,
    pub domain: String,
    pub source_type: SourceType,
    pub source_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_post_id: Option<String>,

    pub title: String,
    pub excerpt: String,
    pub body: String,
    #[serde(default)]
    pub body_len: usize,
    #[serde(default)]
    pub body_source: BodySource,

    pub language: Lang,
    #[serde(default)]
    pub language_confidence: f32,
    pub script: Script,
    /// Arabizi-folded form, for cross-script matching.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translit_body: Option<String>,

    #[serde(default)]
    pub author: Author,

    /// Unix seconds. **Never** back-filled from `crawled_at` without setting
    /// `published_at_precision = Unknown`.
    pub published_at: i64,
    pub crawled_at: i64,
    #[serde(default)]
    pub indexed_at: i64,
    pub published_at_precision: DatePrecision,

    #[serde(default)]
    pub sentiment: Sentiment,
    #[serde(default)]
    pub engagement: Engagement,
    #[serde(default)]
    pub comments_count: u64,

    #[serde(default)]
    pub media: Vec<Media>,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub geo: Geo,

    #[serde(default)]
    pub quality_score: f32,
    #[serde(default)]
    pub spam_score: f32,
    #[serde(default)]
    pub is_nsfw: bool,
    #[serde(default = "default_true")]
    pub robots_indexable: bool,
    #[serde(default)]
    pub http_status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_method: Option<String>,
    /// Which collection path produced this, so a broken path can be re-collected selectively.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_path: Option<String>,
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
}

fn default_true() -> bool {
    true
}
fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

impl Document {
    /// Minimal valid document, for tests and for builders that fill the rest in.
    pub fn new(id: impl Into<String>, url: impl Into<String>, source_type: SourceType) -> Self {
        let url = url.into();
        let domain = domain_of(&url).unwrap_or_default();
        Self {
            id: id.into(),
            content_hash: String::new(),
            simhash: None,
            url,
            canonical_url: None,
            domain,
            source_type,
            source_id: String::new(),
            platform_post_id: None,
            title: String::new(),
            excerpt: String::new(),
            body: String::new(),
            body_len: 0,
            body_source: BodySource::Native,
            language: Lang::Und,
            language_confidence: 0.0,
            script: Script::Unknown,
            translit_body: None,
            author: Author::default(),
            published_at: 0,
            crawled_at: 0,
            indexed_at: 0,
            published_at_precision: DatePrecision::Unknown,
            sentiment: Sentiment::default(),
            engagement: Engagement::default(),
            comments_count: 0,
            media: Vec::new(),
            entities: Vec::new(),
            topics: Vec::new(),
            geo: Geo::default(),
            quality_score: 0.0,
            spam_score: 0.0,
            is_nsfw: false,
            robots_indexable: true,
            http_status: 0,
            fetch_method: None,
            access_path: None,
            schema_version: SCHEMA_VERSION,
        }
    }

    /// Effective freshness timestamp, discounting dates we guessed.
    pub fn is_date_trustworthy(&self) -> bool {
        !matches!(self.published_at_precision, DatePrecision::Unknown)
    }
}

/// A comment on a [`Document`]. Lives in its own index — comments outnumber documents ~5:1 and
/// nesting them would mean rewriting whole documents as replies arrive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub document_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_comment_id: Option<String>,
    pub source_type: SourceType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_comment_id: Option<String>,
    pub body: String,
    pub language: Lang,
    #[serde(default)]
    pub author: Author,
    pub published_at: i64,
    pub crawled_at: i64,
    #[serde(default)]
    pub sentiment: Sentiment,
    #[serde(default)]
    pub likes: u64,
    pub content_hash: String,
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
}

/// Accountability rating of a source. About who is answerable, not about agreement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TrustTier {
    A,
    B,
    C,
}

impl TrustTier {
    /// Ranking multiplier contributed by the trust signal.
    pub const fn weight(self) -> f32 {
        match self {
            Self::A => 1.0,
            Self::B => 0.6,
            Self::C => 0.3,
        }
    }
}

/// Why we are permitted to collect from a source. Required on every registry entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegalBasis {
    PublicWebRobotsOk,
    PlatformApiAuthorized,
    OwnerConsent,
    HashtagApiQuota,
    /// Direct collection under the accepted-risk decision.
    DirectCollection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CrawlFrequency {
    Realtime,
    Hourly,
    Daily,
    Weekly,
}

impl CrawlFrequency {
    pub const fn seconds(self) -> i64 {
        match self {
            Self::Realtime => 300,
            Self::Hourly => 3_600,
            Self::Daily => 86_400,
            Self::Weekly => 604_800,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrawlPolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub frequency: CrawlFrequency,
    #[serde(default = "default_max_docs")]
    pub max_docs_per_run: u32,
    #[serde(default = "default_true")]
    pub respect_robots: bool,
    #[serde(default = "default_crawl_delay")]
    pub crawl_delay_ms: u64,
    #[serde(default = "default_depth")]
    pub depth_limit: u8,
}

fn default_max_docs() -> u32 {
    500
}
fn default_crawl_delay() -> u64 {
    1_500
}
fn default_depth() -> u8 {
    3
}

impl Default for CrawlPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            frequency: CrawlFrequency::Daily,
            max_docs_per_run: default_max_docs(),
            respect_robots: true,
            crawl_delay_ms: default_crawl_delay(),
            depth_limit: default_depth(),
        }
    }
}

/// Where a source sits in its lifecycle (§6 of [[Data Sources Registry]]).
///
/// `Proposed → Reviewed → Approved → Active → (Degraded) → Disabled → Archived`. Only `Active` and
/// `Degraded` are crawled — `Degraded` is still active but flagged, so it keeps being crawled while
/// an operator decides. `Disabled` and everything after it is off. A source can drop to `Disabled`
/// from *any* state (legal-basis lapse, opt-out, takedown, persistent failure).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Lifecycle {
    #[default]
    Proposed,
    Reviewed,
    Approved,
    Active,
    Degraded,
    Disabled,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub kind: SourceType,
    pub display_name: String,
    #[serde(default)]
    pub entry_points: Vec<String>,
    #[serde(default)]
    pub languages: Vec<Lang>,
    #[serde(default)]
    pub crawl_policy: CrawlPolicy,
    pub trust_tier: TrustTier,
    pub legal_basis: LegalBasis,
    #[serde(default)]
    pub approved: bool,
    /// Defaults to `Proposed` so a record hand-added without the field is *not* crawlable until it
    /// is explicitly moved to `Active` — the safe default for a submission.
    #[serde(default)]
    pub lifecycle: Lifecycle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default)]
    pub last_run_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
}

impl Source {
    /// The single predicate the crawler asks before injecting a source's entry points. True only
    /// for an approved source in an active state whose crawl policy is enabled — so no call site
    /// can crawl an un-reviewed, disabled, or paused source by forgetting one of the checks.
    pub fn is_crawlable(&self) -> bool {
        self.approved
            && self.crawl_policy.enabled
            && matches!(self.lifecycle, Lifecycle::Active | Lifecycle::Degraded)
    }

    /// Move to `Disabled` because the legal basis lapsed (§5) — a token revoked, an app removed,
    /// `robots.txt` changed to forbid us. Returns whether anything changed, so a caller exports
    /// only on a real transition. Auto-disable, not a flag: a lapsed basis is not a source we are
    /// allowed to keep crawling while someone decides.
    pub fn disable_for_lapsed_basis(&mut self) -> bool {
        if self.lifecycle == Lifecycle::Disabled || self.lifecycle == Lifecycle::Archived {
            return false;
        }
        self.lifecycle = Lifecycle::Disabled;
        self.append_note("auto-disabled: legal basis lapsed");
        true
    }

    fn append_note(&mut self, msg: &str) {
        let note = format!("[{msg}]");
        match &mut self.notes {
            Some(n) if !n.is_empty() => {
                n.push(' ');
                n.push_str(&note);
            }
            _ => self.notes = Some(note),
        }
    }
}

/// Registrable domain of a URL, lowercased, `www.` stripped.
pub fn domain_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(host.trim_start_matches("www.").to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_round_trips_through_json() {
        let d = Document::new("01J8ZK", "https://www.elkhabar.com/a/1", SourceType::Web);
        let json = serde_json::to_string(&d).unwrap();
        let back: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn document_deserializes_from_minimal_json() {
        // Forward compatibility: optional fields must not be required on the wire.
        let json = r#"{
            "id":"01J","content_hash":"b3:x","url":"https://example.dz/",
            "domain":"example.dz","source_type":"web","source_id":"s",
            "title":"t","excerpt":"e","body":"b",
            "language":"ar","script":"arabic",
            "published_at":1,"crawled_at":2,"published_at_precision":"day"
        }"#;
        let d: Document = serde_json::from_str(json).unwrap();
        assert_eq!(d.schema_version, SCHEMA_VERSION);
        assert!(d.robots_indexable);
        assert_eq!(d.media.len(), 0);
    }

    #[test]
    fn domain_is_extracted_and_normalised() {
        assert_eq!(
            domain_of("https://WWW.ElKhabar.com/x").as_deref(),
            Some("elkhabar.com")
        );
        assert_eq!(
            domain_of("http://example.dz:8080/").as_deref(),
            Some("example.dz")
        );
        assert_eq!(domain_of("not a url"), None);
    }

    #[test]
    fn source_type_round_trips_and_parses_aliases() {
        for st in SourceType::ALL {
            assert_eq!(SourceType::parse(st.as_str()), Some(st));
        }
        assert_eq!(SourceType::parse("FB"), Some(SourceType::Facebook));
        assert_eq!(SourceType::parse("nope"), None);
    }

    #[test]
    fn only_web_uses_the_open_web_profile() {
        assert!(!SourceType::Web.is_platform());
        assert!(SourceType::Facebook.is_platform());
        assert!(SourceType::Instagram.is_platform());
        assert!(SourceType::Tiktok.is_platform());
    }

    #[test]
    fn lang_parses_darija_aliases() {
        assert_eq!(Lang::parse("ary"), Some(Lang::Ary));
        assert_eq!(Lang::parse("darija"), Some(Lang::Ary));
        assert_eq!(Lang::parse("auto"), Some(Lang::Und));
    }

    #[test]
    fn unknown_precision_marks_date_untrustworthy() {
        let mut d = Document::new("1", "https://example.dz/", SourceType::Web);
        d.published_at_precision = DatePrecision::Unknown;
        assert!(!d.is_date_trustworthy());
        d.published_at_precision = DatePrecision::Day;
        assert!(d.is_date_trustworthy());
    }

    #[test]
    fn trust_tiers_have_the_documented_weights() {
        assert_eq!(TrustTier::A.weight(), 1.0);
        assert_eq!(TrustTier::B.weight(), 0.6);
        assert_eq!(TrustTier::C.weight(), 0.3);
    }

    #[test]
    fn enums_serialize_as_lowercase_strings() {
        assert_eq!(
            serde_json::to_string(&SourceType::Facebook).unwrap(),
            "\"facebook\""
        );
        assert_eq!(serde_json::to_string(&Lang::Ary).unwrap(), "\"ary\"");
        assert_eq!(
            serde_json::to_string(&SentimentLabel::Negative).unwrap(),
            "\"negative\""
        );
        assert_eq!(serde_json::to_string(&TrustTier::A).unwrap(), "\"A\"");
    }

    #[test]
    fn comment_round_trips() {
        let c = Comment {
            id: "c1".into(),
            document_id: "d1".into(),
            parent_comment_id: None,
            source_type: SourceType::Facebook,
            platform_comment_id: Some("123".into()),
            body: "واش راك".into(),
            language: Lang::Ary,
            author: Author::default(),
            published_at: 1,
            crawled_at: 2,
            sentiment: Sentiment::default(),
            likes: 3,
            content_hash: "b3:y".into(),
            schema_version: SCHEMA_VERSION,
        };
        let back: Comment = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(c, back);
    }
}
