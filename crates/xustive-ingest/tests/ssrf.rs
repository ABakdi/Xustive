//! SSRF: the ways a URL reaches inside the network anyway.
//!
//! The crawler fetches whatever the open web points it at, so every URL is attacker-controlled by
//! definition — a link on any page we crawl is an instruction from a stranger. The classic
//! consequence is reading a cloud metadata endpoint and walking away with instance credentials.
//!
//! The existing unit test covers the obvious literals. These are the bypasses that get past a
//! guard which only checks the literals: alternative encodings of the same address, a hostname
//! that *resolves* somewhere private, and — the one most implementations miss — a redirect from a
//! perfectly public host to a private one.
//!
//! Each case here is an attack that has worked against real crawlers.

use xustive_core::safe_url::{is_public, SafeUrl, UrlError};

fn refused(url: &str) -> bool {
    match SafeUrl::parse(url) {
        Err(_) => true,
        // Parsed. That is only safe if it could not name a private address.
        Ok(safe) => safe.host_str().is_empty(),
    }
}

#[test]
fn literal_private_and_loopback_addresses_are_refused() {
    for url in [
        "http://127.0.0.1/",
        "http://127.1/",
        "http://0.0.0.0/",
        "http://10.0.0.1/",
        "http://172.16.0.1/",
        "http://192.168.1.1/",
        "http://[::1]/",
        "http://[::]/",
    ] {
        assert!(refused(url), "{url} was accepted");
    }
}

#[test]
fn the_cloud_metadata_endpoint_is_refused() {
    // The single highest-value SSRF target: it answers without authentication and hands out
    // instance credentials. 169.254.169.254 on every major provider.
    for url in [
        "http://169.254.169.254/latest/meta-data/",
        "http://169.254.169.254/computeMetadata/v1/",
        "http://[fd00:ec2::254]/latest/meta-data/",
        // Link-local by another name.
        "http://169.254.1.1/",
    ] {
        assert!(refused(url), "{url} reached metadata");
    }
}

#[test]
fn ipv4_mapped_ipv6_does_not_launder_a_private_address() {
    // `::ffff:127.0.0.1` is loopback wearing a v6 hat. A guard that checks v4 and v6 separately,
    // without unwrapping the mapping, lets every private v4 address through in this form.
    for url in [
        "http://[::ffff:127.0.0.1]/",
        "http://[::ffff:10.0.0.1]/",
        "http://[::ffff:169.254.169.254]/",
        "http://[0:0:0:0:0:ffff:127.0.0.1]/",
    ] {
        assert!(
            refused(url),
            "{url} laundered a private v4 address through v6"
        );
    }
}

#[test]
fn decimal_and_octal_spellings_of_loopback_are_refused() {
    // `2130706433` is 127.0.0.1 as a single integer, and `0177.0.0.1` is it in octal. Both are
    // accepted by most resolvers and by `inet_aton`, and neither looks like a private address to
    // a guard doing string comparison.
    for url in [
        "http://2130706433/",
        "http://0177.0.0.1/",
        "http://0x7f.0x0.0x0.0x1/",
        "http://017700000001/",
    ] {
        assert!(
            refused(url),
            "{url} spelled loopback in a way that got through"
        );
    }
}

#[test]
fn non_http_schemes_are_refused() {
    // `file://` reads the disk, `gopher://` was the classic way to speak arbitrary protocols
    // through a fetcher, and `ftp://` reaches services that are rarely firewalled internally.
    for url in [
        "file:///etc/passwd",
        "file://localhost/etc/shadow",
        "gopher://127.0.0.1:6379/_INFO",
        "ftp://internal.example/",
        "data:text/html,<script>",
        "jar:http://example.com!/",
    ] {
        assert!(refused(url), "{url} used a scheme we should not speak");
    }
}

#[test]
fn credentials_in_the_authority_are_refused() {
    // `http://expected.com@127.0.0.1/` is read by a human as a request to expected.com and by a
    // parser as a request to 127.0.0.1. That gap is the whole attack.
    for url in [
        "http://example.com@127.0.0.1/",
        "http://user:pass@10.0.0.1/",
    ] {
        assert!(refused(url), "{url} hid its real host behind credentials");
    }
}

#[test]
fn a_public_url_is_still_accepted() {
    // The other direction. A guard that refuses everything is trivially secure and useless, and
    // the failure would look like an empty index rather than an error.
    for url in [
        "https://elkhabar.com/articles/1",
        "http://aps.dz/",
        "https://www.elmoudjahid.dz/fr/sports",
    ] {
        assert!(!refused(url), "{url} should be crawlable");
    }
}

#[test]
fn a_redirect_to_a_private_address_is_refused() {
    // The bypass most implementations miss. The first request goes to a genuinely public host, so
    // every pre-flight check passes; the *response* then points inside the network. Anything that
    // validates only the URL it was given, and follows redirects with the HTTP client's built-in
    // policy, is vulnerable here.
    let public = SafeUrl::parse("https://example.dz/start").expect("public start");
    for location in [
        "http://127.0.0.1/",
        "http://169.254.169.254/latest/meta-data/",
        "http://10.0.0.1/admin",
        "http://[::1]/",
        "file:///etc/passwd",
        // Relative redirects resolve against the current URL, so this one stays public — but the
        // protocol-relative form below changes host and must be re-checked.
        "//127.0.0.1/",
    ] {
        let result = public.redirect_to(location, 0);
        assert!(
            result.is_err(),
            "a redirect to {location} was accepted: {:?}",
            result.map(|u| u.as_str().to_string())
        );
    }
}

#[test]
fn a_redirect_to_another_public_host_is_allowed() {
    // Cross-host redirects are ordinary — http to https to www to canonical is most of the web.
    let public = SafeUrl::parse("http://example.dz/start").expect("start");
    let next = public
        .redirect_to("https://www.example.dz/start", 0)
        .expect("a public redirect should be followed");
    assert!(next.as_str().starts_with("https://www.example.dz"));
}

#[test]
fn a_redirect_chain_is_bounded() {
    let public = SafeUrl::parse("https://example.dz/").expect("start");
    // Whatever the limit is, it must be enforced — an unbounded chain is a denial of service
    // against ourselves, and a slow one that looks like a hung worker.
    let result = public.redirect_to("https://example.dz/next", 1_000);
    assert!(matches!(result, Err(UrlError::TooManyRedirects)));
}

#[test]
fn resolved_addresses_are_checked_not_just_the_hostname() {
    // A hostname is not an address. `internal.example.com` looks entirely public until it
    // resolves to 10.0.0.5, and DNS is controlled by whoever owns the domain we are crawling.
    let url = SafeUrl::parse("https://example.dz/").expect("parses");

    let private: Vec<std::net::IpAddr> = ["10.0.0.5", "127.0.0.1", "169.254.169.254", "::1"]
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
    for ip in &private {
        assert!(
            url.check_resolved(&[*ip]).is_err(),
            "{ip} passed the resolved-address check"
        );
    }

    // And one bad address among several must sink the lot. A round-robin record with a single
    // internal entry would otherwise be a coin flip on every request.
    let mixed = vec![
        "93.184.216.34".parse().unwrap(),
        "10.0.0.5".parse().unwrap(),
    ];
    assert!(
        url.check_resolved(&mixed).is_err(),
        "a private address hid among public ones"
    );

    // A wholly public set passes.
    assert!(url
        .check_resolved(&["93.184.216.34".parse().unwrap()])
        .is_ok());
}

#[test]
fn the_reserved_ranges_are_classified_correctly() {
    // Spot-checks on the classifier the whole guard rests on, at the range boundaries — off-by-one
    // errors here are invisible until something inside the network answers.
    for ip in [
        "10.0.0.0",
        "10.255.255.255",
        "172.16.0.0",
        "172.31.255.255",
        "192.168.0.0",
        "192.168.255.255",
        "127.0.0.1",
        "169.254.0.1",
        "100.64.0.1",
        "0.0.0.0",
        "224.0.0.1",
        "255.255.255.255",
    ] {
        assert!(!is_public(ip.parse().unwrap()), "{ip} was called public");
    }
    // Just outside the private ranges.
    for ip in [
        "9.255.255.255",
        "11.0.0.0",
        "172.15.255.255",
        "172.32.0.0",
        "192.167.255.255",
    ] {
        assert!(is_public(ip.parse().unwrap()), "{ip} was called private");
    }
}

#[test]
fn malformed_urls_are_refused_rather_than_panicking() {
    for url in [
        "",
        "http://",
        "://x",
        "http://[",
        "h ttp://x",
        "http://:80/",
        "\0",
    ] {
        let _ = SafeUrl::parse(url);
    }
}
