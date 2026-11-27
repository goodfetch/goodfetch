//! Unit tests and fuzz-seed replays for `state_import` (ADR 0056, ADR 0067;
//! ticket #735).

use goodfetch_protocol::settings::session_source::StateFileFormat;
use serde_json::json;

use super::{
    detect_format, filter_domains, parse_cdp_cookies, parse_cookie_header, parse_cookies_txt,
    parse_storage_state,
};

#[test]
fn detects_all_four_state_file_formats() {
    assert_eq!(
        detect_format(b"  {\"cookies\": [], \"origins\": []}"),
        StateFileFormat::StorageState
    );
    assert_eq!(
        detect_format(b"\n[{\"name\":\"s\",\"value\":\"1\",\"domain\":\"a.test\"}]"),
        StateFileFormat::CdpCookies
    );
    assert_eq!(
        detect_format(b"age-encryption.org/v1\n-> scrypt"),
        StateFileFormat::SessionExport
    );
    assert_eq!(
        detect_format(b"# Netscape HTTP Cookie File\n.a.test\tTRUE\t/\tTRUE\t0\ts\t1\n"),
        StateFileFormat::CookiesTxt
    );
}

/// Covers: story:5-64
#[test]
fn parses_playwright_1_64_storage_state_with_indexed_db_and_opfs() {
    let raw = serde_json::to_vec(&json!({
        "cookies": [{
            "name": "sid",
            "value": "tok-1",
            "domain": "shop.test",
            "path": "/",
            "expires": 1_900_000_000.0,
            "httpOnly": true,
            "secure": true,
            "sameSite": "Lax",
        }],
        "origins": [{
            "origin": "https://shop.test",
            "localStorage": [{ "name": "theme", "value": "dark" }],
            "indexedDB": [{ "name": "app", "version": 1, "stores": [] }],
            "opfs": [{ "path": "session.bin", "contentBase64": "AQID" }],
        }],
        "virtualAuthenticators": [{ "protocol": "ctap2" }],
    }))
    .unwrap();
    let imported = parse_storage_state(&raw).unwrap();
    assert_eq!(imported.cookies.len(), 1);
    assert_eq!(imported.cookies[0].name, "sid");
    assert!(imported.cookies[0].http_only);
    assert_eq!(imported.origins.len(), 1);
    assert_eq!(
        imported.origins[0].local_storage,
        vec![("theme".to_owned(), "dark".to_owned())]
    );
    assert!(imported.origins[0].indexed_db.is_some());
    assert!(imported.origins[0].opfs.is_some());
}

/// Covers: story:5-64
#[test]
fn parses_netscape_cookies_txt_and_cdp_cookie_array() {
    let txt = b"# Netscape HTTP Cookie File\n\
#HttpOnly_.auth.test\tTRUE\t/\tTRUE\t1900000000\tsid\tval1\n\
other.test\tFALSE\t/app\tFALSE\t0\tpref\tval2\n";
    let imported = parse_cookies_txt(txt).unwrap();
    assert_eq!(imported.cookies.len(), 2);
    assert!(imported.cookies[0].http_only);
    assert!(imported.cookies[0].secure);
    assert_eq!(imported.cookies[0].domain, ".auth.test");
    assert!(!imported.cookies[1].http_only);
    assert_eq!(imported.cookies[1].expires, None);

    let filtered = filter_domains(imported, &["auth.test".to_owned()]);
    assert_eq!(filtered.cookies.len(), 1);
    assert_eq!(filtered.cookies[0].name, "sid");

    let cdp = br#"[{"name":"c","value":"v","domain":"api.test","path":"/","httpOnly":false,"secure":true}]"#;
    let from_cdp = parse_cdp_cookies(cdp).unwrap();
    assert_eq!(from_cdp.cookies.len(), 1);
    assert_eq!(from_cdp.cookies[0].domain, "api.test");
}

/// Covers: story:5-65
#[test]
fn parses_cookie_header_for_one_domain_only() {
    let imported = parse_cookie_header("Cookie: sid=abc; theme=light", "shop.test").unwrap();
    assert_eq!(imported.cookies.len(), 2);
    assert!(imported.cookies.iter().all(|c| c.domain == "shop.test"));
    assert_eq!(imported.cookies[0].name, "sid");
    assert_eq!(imported.cookies[0].value, "abc");
    assert!(parse_cookie_header("", "shop.test").is_err());
    assert!(parse_cookie_header("sid=abc", "").is_err());
}

mod storage_state_fuzz {
    goodfetch_test_support::fuzz_replay!(super::super::fuzz_storage_state, "fuzz/storage_state");
}

mod cookies_txt_fuzz {
    goodfetch_test_support::fuzz_replay!(super::super::fuzz_cookies_txt, "fuzz/cookies_txt");
}
