//! The coherence checker ([[Fingerprint Engine]] §4.2).
//!
//! Every invariant here is one real browsers hold automatically and a hand-assembled fingerprint
//! breaks. The catalogue test runs [`check`] over every shipped profile and fails on any violation,
//! so an incoherent profile cannot reach an identity — incoherence is the bug class that matters,
//! and it is checked mechanically rather than by eye.

use super::Profile;

/// One way a profile fails to hang together. Each carries a message naming the mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoherence {
    /// The browser version disagrees across the UA, `sec-ch-ua`, or the declared version.
    Version(String),
    /// The OS disagrees across the UA, `sec-ch-ua-platform`, or `navigator.platform`.
    Os(String),
    /// A non-Chromium browser carries `sec-ch-ua`, or a Chromium one is missing it.
    ClientHints(String),
    /// `accept-language` and `navigator.languages` disagree, or neither fits the proxy geography.
    Language(String),
    /// The timezone does not match the proxy geography.
    Timezone(String),
    /// The WebGL vendor and renderer name different hardware.
    Webgl(String),
}

impl Incoherence {
    pub fn message(&self) -> &str {
        match self {
            Incoherence::Version(m)
            | Incoherence::Os(m)
            | Incoherence::ClientHints(m)
            | Incoherence::Language(m)
            | Incoherence::Timezone(m)
            | Incoherence::Webgl(m) => m,
        }
    }
}

/// Check every §4.2 invariant for one profile. An empty result means the profile is coherent.
pub fn check(p: &Profile) -> Vec<Incoherence> {
    let mut issues = Vec::new();
    check_version(p, &mut issues);
    check_os(p, &mut issues);
    check_client_hints(p, &mut issues);
    check_language(p, &mut issues);
    check_timezone(p, &mut issues);
    check_webgl(p, &mut issues);
    issues
}

/// UA version == `sec-ch-ua` version == declared version (§4.2).
fn check_version(p: &Profile, out: &mut Vec<Incoherence>) {
    let major = p.major_version();
    if !p.user_agent.contains(major) {
        out.push(Incoherence::Version(format!(
            "{}: user-agent does not carry version {major}",
            p.id
        )));
    }
    if let Some(ch) = &p.sec_ch_ua {
        // `sec-ch-ua` lists the major version like `v="131"`.
        if !ch.contains(major) {
            out.push(Incoherence::Version(format!(
                "{}: sec-ch-ua does not carry version {major}",
                p.id
            )));
        }
    }
}

/// OS in the UA matches `sec-ch-ua-platform` and `navigator.platform` (§4.2).
fn check_os(p: &Profile, out: &mut Vec<Incoherence>) {
    if !p.user_agent.contains(p.os.ua_token()) {
        out.push(Incoherence::Os(format!(
            "{}: user-agent lacks the {:?} token {:?}",
            p.id,
            p.os,
            p.os.ua_token()
        )));
    }
    if p.navigator_platform != p.os.navigator_platform() {
        out.push(Incoherence::Os(format!(
            "{}: navigator.platform {:?} != {:?} for {:?}",
            p.id,
            p.navigator_platform,
            p.os.navigator_platform(),
            p.os
        )));
    }
    if let Some(plat) = &p.sec_ch_ua_platform {
        if plat != p.os.ch_ua_platform() {
            out.push(Incoherence::Os(format!(
                "{}: sec-ch-ua-platform {:?} != {:?}",
                p.id,
                plat,
                p.os.ch_ua_platform()
            )));
        }
    }
}

/// Only Chromium browsers emit `sec-ch-ua*`. Safari and Firefox must not; Chrome must (§4.2).
fn check_client_hints(p: &Profile, out: &mut Vec<Incoherence>) {
    let has_hints = p.sec_ch_ua.is_some() || p.sec_ch_ua_platform.is_some();
    match (p.browser.is_chromium(), has_hints) {
        (true, false) => out.push(Incoherence::ClientHints(format!(
            "{}: a Chromium profile must send sec-ch-ua",
            p.id
        ))),
        (false, true) => out.push(Incoherence::ClientHints(format!(
            "{}: {:?} must not send sec-ch-ua (Chromium-only)",
            p.id, p.browser
        ))),
        _ => {}
    }
}

/// `accept-language` agrees with `navigator.languages`, and the set is plausible for the geo (§4.2).
fn check_language(p: &Profile, out: &mut Vec<Incoherence>) {
    // The primary language of `accept-language` (before the first `,` or `;`) must be the first of
    // `navigator.languages` — the two are the same preference expressed twice.
    let primary_al = p
        .accept_language
        .split([',', ';'])
        .next()
        .unwrap_or_default()
        .trim();
    let primary_nav = p
        .navigator_languages
        .first()
        .map(String::as_str)
        .unwrap_or_default();
    if !primary_al.eq_ignore_ascii_case(primary_nav) {
        out.push(Incoherence::Language(format!(
            "{}: accept-language starts {primary_al:?} but navigator.languages starts {primary_nav:?}",
            p.id
        )));
    }
    // Geography plausibility: a DZ profile must offer Arabic or French somewhere — an Algiers IP
    // presenting `en-US` only is the mismatch the doc names.
    if p.geo.eq_ignore_ascii_case("DZ") {
        let al = p.accept_language.to_ascii_lowercase();
        if !al.contains("ar") && !al.contains("fr") {
            out.push(Incoherence::Language(format!(
                "{}: a DZ profile offers neither Arabic nor French: {:?}",
                p.id, p.accept_language
            )));
        }
    }
}

/// Timezone matches the proxy geography (§4.2).
fn check_timezone(p: &Profile, out: &mut Vec<Incoherence>) {
    if p.geo.eq_ignore_ascii_case("DZ") && p.timezone != "Africa/Algiers" {
        out.push(Incoherence::Timezone(format!(
            "{}: a DZ profile has timezone {:?}, expected Africa/Algiers",
            p.id, p.timezone
        )));
    }
}

/// The WebGL vendor and renderer must name the same GPU brand — a real pair, not a mix (§4.2).
fn check_webgl(p: &Profile, out: &mut Vec<Incoherence>) {
    // The GPU brand named in the vendor string must also appear in the renderer string. Apple's
    // integrated GPU is the exception: vendor "Apple …" pairs with renderer "Apple GPU".
    const BRANDS: [&str; 6] = ["NVIDIA", "Intel", "AMD", "Apple", "Qualcomm", "ARM"];
    let vendor = p.webgl_vendor.to_ascii_uppercase();
    let renderer = p.webgl_renderer.to_ascii_uppercase();
    let vendor_brand = BRANDS.iter().find(|b| vendor.contains(&b.to_uppercase()));
    match vendor_brand {
        Some(brand) if !renderer.contains(&brand.to_uppercase()) => {
            out.push(Incoherence::Webgl(format!(
                "{}: WebGL vendor names {brand} but renderer does not: {:?}",
                p.id, p.webgl_renderer
            )));
        }
        None => out.push(Incoherence::Webgl(format!(
            "{}: WebGL vendor {:?} names no known GPU brand",
            p.id, p.webgl_vendor
        ))),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Browser, Os, Profile, WebRtc};
    use super::*;

    fn coherent() -> Profile {
        Profile {
            id: "chrome-131-win11-dz".into(),
            browser: Browser::Chrome,
            version: "131.0.6778.86".into(),
            os: Os::Windows11,
            geo: "DZ".into(),
            user_agent:
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) … Chrome/131.0.0.0 Safari/537.36".into(),
            sec_ch_ua: Some("\"Chromium\";v=\"131\", \"Google Chrome\";v=\"131\"".into()),
            sec_ch_ua_platform: Some("Windows".into()),
            header_order: vec!["sec-ch-ua".into(), "user-agent".into()],
            accept_language: "fr-FR,fr;q=0.9,ar;q=0.8,en;q=0.7".into(),
            navigator_platform: "Win32".into(),
            navigator_languages: vec!["fr-FR".into(), "fr".into(), "ar".into(), "en".into()],
            timezone: "Africa/Algiers".into(),
            webgl_vendor: "Google Inc. (NVIDIA)".into(),
            webgl_renderer: "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060, D3D11)".into(),
            webrtc: WebRtc::Disabled,
            introduced_at: 0,
            retire_after: 0,
        }
    }

    #[test]
    fn a_coherent_profile_has_no_issues() {
        assert!(check(&coherent()).is_empty(), "{:?}", check(&coherent()));
    }

    #[test]
    fn a_version_mismatch_between_ua_and_sec_ch_ua_is_caught() {
        let mut p = coherent();
        p.sec_ch_ua = Some("\"Chromium\";v=\"120\"".into());
        assert!(check(&p)
            .iter()
            .any(|i| matches!(i, Incoherence::Version(_))));
    }

    #[test]
    fn a_safari_profile_carrying_sec_ch_ua_is_incoherent() {
        // Safari on iOS: no client hints, iOS surface. Start from that and inject sec-ch-ua.
        let mut p = coherent();
        p.browser = Browser::Safari;
        p.sec_ch_ua_platform = None; // otherwise the OS check for iOS would also fire
        assert!(
            check(&p)
                .iter()
                .any(|i| matches!(i, Incoherence::ClientHints(_))),
            "Safari must not emit sec-ch-ua"
        );
    }

    #[test]
    fn a_chromium_profile_missing_sec_ch_ua_is_incoherent() {
        let mut p = coherent();
        p.sec_ch_ua = None;
        p.sec_ch_ua_platform = None;
        assert!(check(&p)
            .iter()
            .any(|i| matches!(i, Incoherence::ClientHints(_))));
    }

    #[test]
    fn an_os_mismatch_in_navigator_platform_is_caught() {
        let mut p = coherent();
        p.navigator_platform = "Linux x86_64".into(); // says Linux while the UA says Windows
        assert!(check(&p).iter().any(|i| matches!(i, Incoherence::Os(_))));
    }

    #[test]
    fn a_dz_profile_without_arabic_or_french_is_incoherent() {
        let mut p = coherent();
        p.accept_language = "en-US,en;q=0.9".into();
        p.navigator_languages = vec!["en-US".into(), "en".into()];
        assert!(check(&p)
            .iter()
            .any(|i| matches!(i, Incoherence::Language(_))));
    }

    #[test]
    fn a_language_disagreement_between_headers_and_js_is_caught() {
        let mut p = coherent();
        p.navigator_languages = vec!["ar".into(), "fr".into()]; // JS says ar-first, header says fr-first
        assert!(check(&p)
            .iter()
            .any(|i| matches!(i, Incoherence::Language(_))));
    }

    #[test]
    fn a_dz_profile_in_the_wrong_timezone_is_caught() {
        let mut p = coherent();
        p.timezone = "America/New_York".into();
        assert!(check(&p)
            .iter()
            .any(|i| matches!(i, Incoherence::Timezone(_))));
    }

    #[test]
    fn a_mismatched_webgl_pair_is_caught() {
        let mut p = coherent();
        p.webgl_vendor = "Google Inc. (Intel)".into(); // Intel vendor…
                                                       // …but the renderer still says NVIDIA.
        assert!(check(&p).iter().any(|i| matches!(i, Incoherence::Webgl(_))));
    }
}
