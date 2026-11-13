//! `is_local_path` and `normalize_email`, ported from the Rails app's tests.

use thymus::{is_local_path, normalize_email};

#[test]
fn local_paths_are_allowed() {
    for path in [
        "/",
        "/kosik",
        "/kosik/udaje",
        "/muj-ucet",
        "/ok?x=1",
        "/a#b",
        "/ok?next=//x",
        "/č",
    ] {
        assert!(is_local_path(path), "{path:?}");
    }
}

#[test]
fn other_sites_and_tricks_are_refused() {
    for path in [
        "",
        "kosik",
        "//evil.com",
        "//evil.example.com",
        "/\\evil.com",
        "/\\evil.example",
        "\\\\evil.com",
        "https://evil.com",
        "https://evil.example.com",
        "javascript:alert(1)",
        "/\tevil",
        "/\t/evil.com",
        "/\n/evil.com",
        "/ok\r\nSet-Cookie: x=1",
        "/ok\0",
        "/ok\u{7f}",
        "/ok\u{85}",
        "/a\\b",
        " /ok",
    ] {
        assert!(!is_local_path(path), "{path:?}");
    }
}

#[test]
fn normalizes_emails() {
    assert_eq!(normalize_email("jana@example.com"), "jana@example.com");
    assert_eq!(normalize_email("  Jana@Example.COM \n"), "jana@example.com");
    assert_eq!(normalize_email("\tŽofie@Example.cz"), "žofie@example.cz");
    assert_eq!(normalize_email("   "), "");
}
