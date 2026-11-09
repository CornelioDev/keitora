// SPDX-License-Identifier: MIT OR Apache-2.0

use std::net::{Ipv4Addr, Ipv6Addr};

use super::*;

fn rule(pattern: &str) -> Option<Rule> {
    Rule::parse(&SitePattern::new(pattern))
}

fn domain(name: &str) -> Host<String> {
    Host::Domain(name.to_owned())
}

fn localhost() -> Host<String> {
    Host::Ipv4(Ipv4Addr::LOCALHOST)
}

#[test]
fn rule_reads_the_domain_and_the_path() {
    assert_eq!(
        rule("  *.Reddit.COM./R/Rust/  "),
        Some(Rule {
            host: domain("reddit.com"),
            path: "/r/rust".to_owned(),
        })
    );
    assert_eq!(
        rule("bücher.de"),
        Some(Rule {
            host: domain("xn--bcher-kva.de"),
            path: String::new(),
        })
    );
}

#[test]
fn rule_treats_a_backslash_like_a_slash() {
    assert_eq!(
        rule("youtube.com\\shorts\\"),
        Some(Rule {
            host: domain("youtube.com"),
            path: "/shorts".to_owned(),
        })
    );
}

#[test]
fn rule_resolves_dot_segments_in_its_path() {
    assert_eq!(
        rule("youtube.com/feed/../shorts"),
        Some(Rule {
            host: domain("youtube.com"),
            path: "/shorts".to_owned(),
        })
    );
}

#[test]
fn rule_with_only_a_slash_covers_the_whole_site() {
    assert_eq!(rule("youtube.com/").map(|r| r.path), Some(String::new()));
}

#[test]
fn rule_keeps_ip_addresses() {
    assert_eq!(rule("127.0.0.1/admin").map(|r| r.host), Some(localhost()));
    assert_eq!(
        rule("[::1]").map(|r| r.host),
        Some(Host::<String>::Ipv6(Ipv6Addr::LOCALHOST))
    );
}

#[test]
fn unusable_patterns_give_no_rule() {
    for pattern in [
        "",
        " ",
        "\u{3000}",
        "*",
        "*.",
        ".",
        "..",
        "/",
        "/r/rust",
        "*./r/rust",
        "**.reddit.com",
        "*.*.reddit.com",
        "reddit.*",
        "%2a.reddit.com",
        "a..reddit.com",
        ".reddit.com",
        "reddit.com..",
        "https://reddit.com",
        "reddit.com:443",
        "user@reddit.com",
        "reddit com",
        "xn--zz.com",
        "[::1",
        "youtube.com/watch?v=abc",
        "youtube.com/watch#t=3",
    ] {
        assert_eq!(rule(pattern), None, "{pattern:?}");
    }
}

#[test]
fn rule_covers_its_domain_and_subdomains_by_whole_labels() {
    let rule = rule("reddit.com").expect("a usable pattern");
    assert!(rule.covers_host(&domain("reddit.com")));
    assert!(rule.covers_host(&domain("old.reddit.com")));
    assert!(!rule.covers_host(&domain("notreddit.com")));
    assert!(!rule.covers_host(&domain("reddit.com.evil.com")));
    assert!(!rule.covers_host(&domain("com")));
}

#[test]
fn ip_rule_covers_only_that_address() {
    let rule = rule("127.0.0.1").expect("a usable pattern");
    assert!(rule.covers_host(&localhost()));
    assert!(!rule.covers_host(&Host::<String>::Ipv4(Ipv4Addr::new(10, 127, 0, 1))));
    assert!(!rule.covers_host(&domain("127.0.0.1")));
}

#[test]
fn rule_covers_its_path_by_whole_segments() {
    let rule = rule("youtube.com/shorts").expect("a usable pattern");
    assert!(rule.covers_path("/shorts"));
    assert!(rule.covers_path("/shorts/"));
    assert!(rule.covers_path("/shorts/abc"));
    assert!(!rule.covers_path("/shortsfoo"));
    assert!(!rule.covers_path("/"));
}

#[test]
fn page_keeps_the_host_and_path_of_web_urls_only() {
    let page =
        Page::parse("HTTPS://User:Pass@WWW.Reddit.com.:8443/R/%72ust?q=1#top").expect("a web URL");
    assert_eq!(page.host, domain("www.reddit.com"));
    assert_eq!(page.path, "/r/rust");

    for input in [
        "",
        "reddit.com",
        "ftp://reddit.com/",
        "about:blank",
        "https://",
    ] {
        assert!(Page::parse(input).is_none(), "{input:?}");
    }
}

#[test]
fn normalize_path_decodes_unreserved_characters_only() {
    assert_eq!(normalize_path("/%41%62%2D%2e%5F%7e%30"), "/ab-._~0");
    assert_eq!(normalize_path("/a%2Fb%20c"), "/a%2fb%20c");
    assert_eq!(normalize_path("/Caf%C3%A9"), "/caf%c3%a9");
}

#[test]
fn normalize_path_keeps_incomplete_or_invalid_escapes() {
    assert_eq!(normalize_path("/%"), "/%");
    assert_eq!(normalize_path("/%4"), "/%4");
    assert_eq!(normalize_path("/%ZZ"), "/%zz");
    assert_eq!(normalize_path("/%%41"), "/%a");
    assert_eq!(normalize_path("/%\u{e9}1"), "/%\u{e9}1");
}

#[test]
fn decode_unreserved_rejects_everything_else() {
    assert_eq!(decode_unreserved('4', '1'), Some('A'));
    assert_eq!(decode_unreserved('7', 'E'), Some('~'));
    assert_eq!(decode_unreserved('2', 'F'), None);
    assert_eq!(decode_unreserved('g', '0'), None);
    assert_eq!(decode_unreserved('0', 'g'), None);
}

#[test]
fn canonical_ip_turns_ipv4_mapped_addresses_into_ipv4_only() {
    let mapped = Host::<String>::Ipv6(Ipv4Addr::LOCALHOST.to_ipv6_mapped());
    assert_eq!(canonical_ip(mapped), localhost());
    let loopback = Host::<String>::Ipv6(Ipv6Addr::LOCALHOST);
    assert_eq!(canonical_ip(loopback.clone()), loopback);
    assert_eq!(canonical_ip(localhost()), localhost());
    assert_eq!(canonical_ip(domain("a.com")), domain("a.com"));
}

#[test]
fn rule_reads_an_ipv4_mapped_address_as_ipv4() {
    assert_eq!(rule("[::ffff:7f00:1]").map(|r| r.host), Some(localhost()));
}

#[test]
fn page_ignores_every_trailing_dot_of_its_host() {
    let page = Page::parse("https://old.reddit.com.../").expect("a web URL");
    assert_eq!(page.host, domain("old.reddit.com"));
    for input in ["https://./", "https://../", "https://.../"] {
        assert!(Page::parse(input).is_none(), "{input:?}");
    }
}

#[test]
fn page_reads_an_ipv4_mapped_address_as_ipv4() {
    let page = Page::parse("http://[::ffff:127.0.0.1]/").expect("a web URL");
    assert_eq!(page.host, localhost());
}

#[test]
fn ambiguous_paths_have_double_slashes_encoded_slashes_or_parameters() {
    for path in ["//a", "/a//", "/a%2fb", "/a%5cb", "/a;b", "/;", "/a%3bb"] {
        assert!(is_ambiguous(path), "{path:?}");
    }
    for path in ["/", "/a/b", "/a%2eb", "/a%25b", "/a%2"] {
        assert!(!is_ambiguous(path), "{path:?}");
    }
}

#[test]
fn lenient_readings_combine_the_three_steps_in_order() {
    assert_eq!(
        lenient_readings("//feed%2f..;x/shorts"),
        [
            "//feed%2f..;x/shorts",
            "/feed%2f..;x/shorts",
            "//feed%2f../shorts",
            "/feed%2f../shorts",
            "//feed/..;x/shorts",
            "/feed/..;x/shorts",
            "//shorts",
            "/shorts",
        ]
    );
}

#[test]
fn decode_slashes_reads_encoded_slashes_and_backslashes_as_slashes() {
    assert_eq!(decode_slashes("/a%2fb%5cc%25d"), "/a/b/c%25d");
}

#[test]
fn strip_parameters_removes_them_from_each_segment() {
    assert_eq!(strip_parameters("/a;x/b;y;z/c;"), "/a/b/c");
    assert_eq!(strip_parameters("/;x"), "/");
    assert_eq!(strip_parameters("/a/b"), "/a/b");
    assert_eq!(strip_parameters("/a%3bx/b%3b/..%3by/c"), "/a/b/../c");
}

#[test]
fn collapse_slashes_merges_runs_of_slashes() {
    assert_eq!(collapse_slashes("///a//b/c///"), "/a/b/c/");
}

#[test]
fn resolve_dot_segments_works_as_whatwg_does() {
    assert_eq!(resolve_dot_segments("/a/b/../c"), "/a/c");
    assert_eq!(resolve_dot_segments("/a/./b"), "/a/b");
    assert_eq!(resolve_dot_segments("/a/.."), "/");
    assert_eq!(resolve_dot_segments("/a/."), "/a/");
    assert_eq!(resolve_dot_segments("/../../a"), "/a");
    assert_eq!(resolve_dot_segments("/"), "/");
    assert_eq!(resolve_dot_segments("//a"), "//a");
    assert_eq!(resolve_dot_segments("/a/..;/b"), "/a/..;/b");
}

#[test]
fn is_usable_agrees_with_rule_parse() {
    for pattern in ["reddit.com", "*.x.com/a", "[::1]", " ", "https://x.com"] {
        let pattern = SitePattern::new(pattern);
        assert_eq!(is_usable(&pattern), Rule::parse(&pattern).is_some());
    }
}

#[test]
fn matcher_keeps_only_usable_site_patterns() {
    let blocklist = Blocklist {
        id: "list".to_owned(),
        name: "List".to_owned(),
        color: "#000000".to_owned(),
        targets: vec![
            Target::Site {
                pattern: SitePattern::new("reddit.com"),
            },
            Target::Site {
                pattern: SitePattern::new(" "),
            },
            Target::App {
                name: "Steam".to_owned(),
                ids: crate::model::AppIdentifiers {
                    windows_exe: Some("steam.exe".to_owned()),
                    ..crate::model::AppIdentifiers::default()
                },
            },
        ],
        exceptions: vec![SitePattern::new("*"), SitePattern::new("old.reddit.com")],
        block_during_breaks: false,
    };
    let matcher = SiteMatcher::new(&blocklist);
    assert_eq!(matcher.targets.len(), 1);
    assert_eq!(matcher.exceptions.len(), 1);
    assert!(matcher.is_blocked("https://www.reddit.com/"));
    assert!(!matcher.is_blocked("https://old.reddit.com/"));
    assert!(!matcher.is_blocked("not a url"));
}
