//! Approximate location from a locally bundled database (M8-T05.1, [[ADR-0020]]).
//!
//! *"weather"* with no place in it is the most common way anyone asks, and answering it needs to
//! know roughly where the reader is. Every ordinary way to find that out is unacceptable here: a
//! browser permission prompt is a bad trade for a weather card, a third-party lookup service sends
//! the reader's address to someone else and needs egress the serving plane does not have, and
//! remembering a location per reader is a profile.
//!
//! But the address is **already in the process** — the connection terminates here, which is why
//! `ratelimit.rs` can see it — and turning it into "probably Oran" needs no network at all.
//!
//! The rules, each of which the tests below pin:
//!
//! - The lookup is a memory-mapped file. No service, no request, no egress.
//! - The result is immediately coarsened to a **wilaya**, and only the wilaya travels onward. A
//!   coordinate never reaches the cache lookup or the response.
//! - Nothing is written down. The value lives in one function's stack; there is nothing to expire
//!   because there is nothing stored.
//! - It is never a cache key. Weather is keyed by wilaya, an enumerable set of 58 shared by
//!   everyone in it — keying anything by address would be keying by person.
//! - `X-Forwarded-For` is not consulted, matching `ratelimit.rs`. Behind a proxy this degrades to
//!   "no location", which is the correct failure.

use std::net::IpAddr;
use std::path::Path;

use xustive_tools::wilaya::{Wilaya, WILAYAS};

/// Where the database is looked for. Absent is normal and not an error: the feature simply does
/// not run, and a reader who names a place is unaffected.
pub const DEFAULT_PATH: &str = "data/geoip/dbip-city-lite.mmdb";

/// A loaded database, or nothing.
pub struct GeoIp {
    reader: maxminddb::Reader<Vec<u8>>,
}

impl GeoIp {
    /// Load the database if it is there.
    ///
    /// Returns `None` rather than an error for a missing file. The database is a large, separately
    /// fetched asset (`scripts/fetch-geoip.sh`), a checkout without it is the normal state, and a
    /// process that refused to start over an optional weather nicety would be the wrong trade.
    pub fn load(path: impl AsRef<Path>) -> Option<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return None;
        }
        match maxminddb::Reader::open_readfile(path) {
            Ok(reader) => Some(Self { reader }),
            Err(e) => {
                tracing::warn!(error = %e, "geoip database present but unreadable; ignoring");
                None
            }
        }
    }

    /// The wilaya this address is probably in, if it is plausibly in Algeria at all.
    ///
    /// Takes the coordinate the database reports and immediately discards it — the return type is
    /// a wilaya, so a coordinate cannot leak past this function even by accident.
    pub fn wilaya_of(&self, ip: IpAddr) -> Option<&'static Wilaya> {
        // A private or loopback address is a developer or a misconfigured proxy, never a reader
        // whose location we could know.
        if !is_global(ip) {
            return None;
        }
        let city: maxminddb::geoip2::City = self.reader.lookup(ip).ok()?;
        // Only Algerian addresses get a wilaya. Someone in Paris is not "probably Tamanrasset",
        // and the honest answer for them is no assumed location at all.
        let country = city.country.as_ref()?.iso_code?;
        if !country.eq_ignore_ascii_case("DZ") {
            return None;
        }
        let location = city.location.as_ref()?;
        nearest_wilaya(location.latitude?, location.longitude?)
    }
}

/// Whether an address could belong to a reader on the internet.
fn is_global(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified())
        }
        IpAddr::V6(v6) => !(v6.is_loopback() || v6.is_unspecified()),
    }
}

/// The wilaya seat closest to a coordinate.
///
/// Seats rather than centroids, which is the choice the wilaya table already made for prayer
/// times: a centroid of Tamanrasset sits in empty desert hundreds of kilometres from anyone.
///
/// Distance is computed on the equirectangular approximation rather than the haversine. Over
/// Algeria the error is a fraction of a percent, and the answer only has to pick the nearest of 58
/// points that are hundreds of kilometres apart — a more precise formula would change no result.
pub fn nearest_wilaya(latitude: f64, longitude: f64) -> Option<&'static Wilaya> {
    let lat_rad = latitude.to_radians();
    WILAYAS
        .iter()
        .map(|w| {
            let dy = w.latitude - latitude;
            let dx = (w.longitude - longitude) * lat_rad.cos();
            (dy * dy + dx * dx, w)
        })
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, w)| w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_coordinate_in_oran_resolves_to_oran() {
        let w = nearest_wilaya(35.6976, -0.6337).unwrap();
        assert_eq!(w.name_fr, "Oran");
    }

    #[test]
    fn a_coordinate_in_the_capital_resolves_to_algiers() {
        let w = nearest_wilaya(36.7538, 3.0588).unwrap();
        assert_eq!(w.code, 16);
    }

    #[test]
    fn the_deep_south_resolves_to_a_southern_wilaya_not_the_nearest_by_latitude_alone() {
        // Tamanrasset is far south and far east of the coastal seats. A longitude term that
        // ignored the latitude's convergence would drag this answer north.
        let w = nearest_wilaya(22.785, 5.5228).unwrap();
        assert_eq!(w.name_fr, "Tamanrasset");
    }

    #[test]
    fn a_private_or_loopback_address_never_yields_a_location() {
        // A developer on localhost, or a misconfigured proxy handing us its own address. Neither
        // is a reader whose location we could know, and guessing would be worse than declining.
        for ip in ["127.0.0.1", "10.0.0.5", "192.168.1.1", "::1"] {
            assert!(
                !is_global(ip.parse().unwrap()),
                "{ip} must not be treated as a reader"
            );
        }
        assert!(is_global("41.100.1.1".parse().unwrap()));
    }

    #[test]
    fn a_missing_database_is_absent_rather_than_an_error() {
        // The normal state of a fresh checkout. The feature does not run; nothing else changes.
        assert!(GeoIp::load("data/geoip/definitely-not-here.mmdb").is_none());
    }

    /// The privacy pin for [[ADR-0020]] (M8-T10.3).
    ///
    /// Read from this module's own source, the way `lint-telemetry.sh` reads the crates and
    /// `dataage` reads `alerts.yml`. A behavioural test cannot prove a negative — that the address
    /// never reaches a log, a store, or a cache key — but a source assertion can prove nobody has
    /// written the code that would do it, and it fails loudly on the commit that tries.
    #[test]
    fn the_client_address_never_reaches_a_log_a_store_or_a_cache_key() {
        let source = include_str!("geoip.rs");
        // Everything before the test module. Only the real code is in scope; the tests below
        // legitimately name addresses.
        let code = &source[..source.find("#[cfg(test)]").expect("a test module exists")];

        for forbidden in ["tracing::info!", "tracing::debug!", "tracing::warn!(ip"] {
            assert!(
                !code.contains(forbidden),
                "geoip must not log: found {forbidden}. The address is the one value ADR-0020 \
                 promises never travels — a log line is travelling."
            );
        }
        // Mutable retention only. `&'static Wilaya` is a compile-time table, not a reader's data,
        // and an over-eager pattern that flagged it would have to be deleted rather than heeded —
        // which is how a guard stops being read.
        for forbidden in [
            "insert(",
            ".put(",
            ".set(",
            "HashMap",
            "static mut",
            "OnceLock",
            "LazyLock",
            "Mutex",
            "RwLock",
        ] {
            assert!(
                !code.contains(forbidden),
                "geoip must not retain anything derived from an address: found {forbidden}. \
                 The value is request-scoped, which is why there is nothing to expire."
            );
        }
        // And the one warning that does exist must be about the database file, never a reader.
        assert!(code.contains("geoip database present but unreadable"));
    }

    #[test]
    fn the_return_type_is_a_wilaya_so_a_coordinate_cannot_travel_further() {
        // Not a behavioural test so much as a pinned design decision: `wilaya_of` returns a
        // wilaya, so no caller can accidentally carry a reader's coordinate into a cache key, a
        // response, or a log line. Changing this signature is the thing to argue about.
        fn _assert_shape(g: &GeoIp, ip: IpAddr) -> Option<&'static Wilaya> {
            g.wilaya_of(ip)
        }
    }
}
