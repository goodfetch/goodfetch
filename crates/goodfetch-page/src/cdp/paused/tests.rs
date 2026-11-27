//! Which of its browser context's cookies a shared or service worker's
//! request carries, by the cookie's domain, path and `Secure` flag.

use goodfetch_cdp::generated::network;
use serde_json::json;

use super::sent_to;

/// The value of every cookie here, visibly fake.
const COOKIE_VALUE: &str = "test-value";

/// A cookie of `domain` and `path`, `Secure` or not, as the browser
/// reports it.
fn cookie(domain: &str, path: &str, secure: bool) -> network::Cookie {
    serde_json::from_value(json!({
        "name": "session",
        "value": COOKIE_VALUE,
        "domain": domain,
        "path": path,
        "expires": -1,
        "size": 18,
        "httpOnly": false,
        "secure": secure,
        "session": true,
        "priority": "Medium",
        "sourceScheme": "Secure",
        "sourcePort": 443,
    }))
    .expect("a cookie as the browser reports it")
}

fn url(text: &str) -> url::Url {
    url::Url::parse(text).expect("a URL")
}

#[test]
fn a_host_only_cookie_goes_to_its_one_host() {
    let host_only = cookie("site.test", "/", false);
    assert!(sent_to(&host_only, &url("https://site.test/probe.js")));
    assert!(sent_to(&host_only, &url("https://SITE.test/")));
    assert!(!sent_to(&host_only, &url("https://www.site.test/")));
    assert!(!sent_to(&host_only, &url("https://othersite.test/")));
}

#[test]
fn a_domain_cookie_goes_to_its_host_and_its_subdomains_only() {
    let domain = cookie(".site.test", "/", false);
    assert!(sent_to(&domain, &url("https://site.test/")));
    assert!(sent_to(&domain, &url("https://a.b.site.test/")));
    assert!(!sent_to(&domain, &url("https://othersite.test/")));
    assert!(!sent_to(&domain, &url("https://site.test.evil/")));
}

#[test]
fn a_cookie_goes_to_its_path_and_the_paths_under_it() {
    let folder = cookie("site.test", "/app", false);
    assert!(sent_to(&folder, &url("https://site.test/app")));
    assert!(sent_to(&folder, &url("https://site.test/app/worker.js")));
    assert!(!sent_to(&folder, &url("https://site.test/apple")));
    assert!(!sent_to(&folder, &url("https://site.test/")));
    let slashed = cookie("site.test", "/app/", false);
    assert!(sent_to(&slashed, &url("https://site.test/app/worker.js")));
}

#[test]
fn a_secure_cookie_goes_over_tls_only() {
    let secure = cookie("site.test", "/", true);
    assert!(sent_to(&secure, &url("https://site.test/")));
    assert!(sent_to(&secure, &url("wss://site.test/socket")));
    assert!(!sent_to(&secure, &url("http://site.test/")));
}
