//! The Fingerprint Engine (M2-T01b): coherent browser identities across TLS, HTTP/2, headers, and
//! the JS surface.
//!
//! The insight that shapes everything here is that **detection is about coherence, not any single
//! value** ([[Fingerprint Engine]] §1). A request whose TLS handshake says Chrome, whose headers say
//! Chrome, but whose `navigator.platform` says Linux is flagged not because any one field is wrong
//! but because real browsers never combine them that way. So a profile is generated and pinned as a
//! **unit** — individual fields are never mixed — and the load-bearing thing this module provides is
//! the mechanical check that every shipped profile actually satisfies the §4.2 invariants.
//!
//! What lives here is the **schema and the coherence checker**. The actual TLS/HTTP-2 impersonation
//! (a browser-accurate client library) and the headless CDP patching are integration work that
//! needs those libraries and a real browser; this is the part that decides whether a profile is
//! internally consistent, which is the bug class that matters.

mod coherence;

pub use coherence::{check, Incoherence};

use serde::{Deserialize, Serialize};

/// The browsers we model. Only Chromium browsers emit the `sec-ch-ua*` client-hint headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Browser {
    Chrome,
    Firefox,
    Safari,
}

impl Browser {
    /// Whether this browser sends `sec-ch-ua*` headers. Chromium only — Firefox and Safari never do,
    /// so a Safari profile carrying `sec-ch-ua` is incoherent (§4.2).
    pub const fn is_chromium(self) -> bool {
        matches!(self, Browser::Chrome)
    }
}

/// Operating systems we model, each with the token a real UA/header/JS-surface would present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Os {
    Windows11,
    MacOS,
    Android,
    IOS,
}

impl Os {
    /// The substring a genuine `User-Agent` for this OS contains.
    pub const fn ua_token(self) -> &'static str {
        match self {
            Os::Windows11 => "Windows NT 10.0",
            Os::MacOS => "Macintosh",
            Os::Android => "Android",
            Os::IOS => "iPhone",
        }
    }

    /// The `sec-ch-ua-platform` value (Chromium only).
    pub const fn ch_ua_platform(self) -> &'static str {
        match self {
            Os::Windows11 => "Windows",
            Os::MacOS => "macOS",
            Os::Android => "Android",
            Os::IOS => "iOS",
        }
    }

    /// The `navigator.platform` value the JS surface should report.
    pub const fn navigator_platform(self) -> &'static str {
        match self {
            Os::Windows11 => "Win32",
            Os::MacOS => "MacIntel",
            Os::Android => "Linux armv8l",
            Os::IOS => "iPhone",
        }
    }
}

/// WebRTC handling. There is no `Direct` — a leaked local IP defeats every other layer, so a profile
/// may only disable WebRTC or force it through the proxy (§4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebRtc {
    Disabled,
    ThroughProxy,
}

/// One coherent fingerprint profile — the unit that is pinned to an identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub browser: Browser,
    /// Full browser version, e.g. `"131.0.6778.86"`.
    pub version: String,
    pub os: Os,
    /// Expected proxy geography, e.g. `"DZ"` — the profile's language and timezone must agree with
    /// it, or an Algiers IP presenting a New York surface is the incoherence platforms look for.
    pub geo: String,

    pub user_agent: String,
    /// `sec-ch-ua` header value — present only for Chromium browsers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sec_ch_ua: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sec_ch_ua_platform: Option<String>,
    /// Exact header order — never a sorted map (§4.1).
    pub header_order: Vec<String>,
    pub accept_language: String,

    pub navigator_platform: String,
    pub navigator_languages: Vec<String>,
    pub timezone: String,
    pub webgl_vendor: String,
    pub webgl_renderer: String,
    pub webrtc: WebRtc,

    /// Ageing (§4.5): when the profile entered service and when it should retire. Zero means unset.
    #[serde(default)]
    pub introduced_at: i64,
    #[serde(default)]
    pub retire_after: i64,
}

impl Profile {
    /// Parse a profile from a TOML document.
    pub fn from_toml(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// The major version number, e.g. `"131"` from `"131.0.6778.86"`.
    pub fn major_version(&self) -> &str {
        self.version.split('.').next().unwrap_or(&self.version)
    }

    /// Whether this profile has aged out at `now` — past its `retire_after`, if one is set. Its
    /// identities are then migrated to the successor of the **same browser and OS** (§4.5).
    pub fn is_retired(&self, now: i64) -> bool {
        self.retire_after != 0 && now >= self.retire_after
    }

    /// Whether `successor` is a valid migration target for this profile: the same browser and OS, a
    /// newer version. Bumping Chrome 131→133 on Windows is normal; switching to Safari is not.
    pub fn can_migrate_to(&self, successor: &Profile) -> bool {
        self.browser == successor.browser
            && self.os == successor.os
            && successor.version.as_str() > self.version.as_str()
    }
}

/// A loaded set of profiles.
#[derive(Debug, Clone, Default)]
pub struct Catalogue {
    profiles: Vec<Profile>,
}

impl Catalogue {
    /// Load every `*.toml` in a directory as a profile. A file that fails to parse is a hard error —
    /// a malformed fingerprint is one that would be assigned and then flagged, worse than absent.
    pub fn load_dir(dir: &str) -> Result<Self, String> {
        let mut profiles = Vec::new();
        let entries = std::fs::read_dir(dir).map_err(|e| format!("cannot read {dir}: {e}"))?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let profile =
                Profile::from_toml(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            profiles.push(profile);
        }
        profiles.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(Self { profiles })
    }

    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_version_and_migration_rules() {
        let base = Profile {
            id: "chrome-131-win11-dz".into(),
            browser: Browser::Chrome,
            version: "131.0.6778.86".into(),
            os: Os::Windows11,
            geo: "DZ".into(),
            user_agent: "…".into(),
            sec_ch_ua: None,
            sec_ch_ua_platform: None,
            header_order: vec![],
            accept_language: "fr".into(),
            navigator_platform: "Win32".into(),
            navigator_languages: vec!["fr".into()],
            timezone: "Africa/Algiers".into(),
            webgl_vendor: "Google Inc. (NVIDIA)".into(),
            webgl_renderer: "ANGLE (NVIDIA)".into(),
            webrtc: WebRtc::Disabled,
            introduced_at: 0,
            retire_after: 0,
        };
        assert_eq!(base.major_version(), "131");

        let newer = Profile {
            id: "chrome-133-win11-dz".into(),
            version: "133.0.1.1".into(),
            ..base.clone()
        };
        assert!(
            base.can_migrate_to(&newer),
            "same browser+OS, newer version"
        );

        let safari = Profile {
            browser: Browser::Safari,
            ..newer.clone()
        };
        assert!(
            !base.can_migrate_to(&safari),
            "never migrate across browsers"
        );
    }

    #[test]
    fn retirement_is_gated_on_the_retire_after_stamp() {
        let mut p = Profile::from_toml(SAMPLE).unwrap();
        p.retire_after = 0;
        assert!(!p.is_retired(i64::MAX), "unset retire_after never retires");
        p.retire_after = 1000;
        assert!(!p.is_retired(999));
        assert!(p.is_retired(1000));
    }

    const SAMPLE: &str = r#"
id = "chrome-131-win11-dz"
browser = "Chrome"
version = "131.0.6778.86"
os = "Windows11"
geo = "DZ"
user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
sec_ch_ua = "\"Chromium\";v=\"131\", \"Google Chrome\";v=\"131\""
sec_ch_ua_platform = "Windows"
header_order = ["sec-ch-ua","user-agent","accept","accept-language"]
accept_language = "fr-FR,fr;q=0.9,ar;q=0.8,en;q=0.7"
navigator_platform = "Win32"
navigator_languages = ["fr-FR","fr","ar","en"]
timezone = "Africa/Algiers"
webgl_vendor = "Google Inc. (NVIDIA)"
webgl_renderer = "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0, D3D11)"
webrtc = "disabled"
"#;

    #[test]
    fn a_profile_parses_from_toml() {
        let p = Profile::from_toml(SAMPLE).unwrap();
        assert_eq!(p.browser, Browser::Chrome);
        assert_eq!(p.os, Os::Windows11);
        assert_eq!(p.navigator_languages.len(), 4);
        assert_eq!(p.webrtc, WebRtc::Disabled);
    }
}
