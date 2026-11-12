use rstest::rstest;
use url::Url;

use super::{Jar, MAX_SIZE};

/// A jar that kept what the answer from `from` set with `set_cookies`.
fn kept(from: &str, set_cookies: &[&str]) -> Jar {
    let jar = Jar::default();
    let headers: Vec<(String, String)> = set_cookies
        .iter()
        .map(|value| ("set-cookie".to_owned(), (*value).to_owned()))
        .collect();
    jar.keep(&Url::parse(from).unwrap(), &headers);
    jar
}

/// The `Cookie` header `jar` gives a request to `url`.
fn sent(jar: &Jar, url: &str) -> Option<String> {
    jar.cookie(&Url::parse(url).unwrap())
}

#[rstest]
#[case::its_own_host("https://shop.example.com/b", Some("hop=1"))]
#[case::another_host_of_its_site("https://www.example.com/", None)]
#[case::another_site("https://example.org/", None)]
fn a_cookie_with_no_domain_goes_back_to_its_host_alone(
    #[case] to: &str,
    #[case] cookie: Option<&str>,
) {
    let jar = kept("https://shop.example.com/a", &["hop=1; Path=/"]);

    assert_eq!(sent(&jar, to).as_deref(), cookie);
}

#[rstest]
#[case::a_secure_cookie_over_plain_http("hop=1; Path=/; Secure", "http://example.com/", None)]
#[case::a_secure_cookie_over_tls("hop=1; Path=/; Secure", "https://example.com/", Some("hop=1"))]
#[case::a_domain_cookie_on_a_subdomain(
    "hop=1; Domain=example.com",
    "https://www.example.com/",
    Some("hop=1")
)]
#[case::a_path_it_was_not_set_for("hop=1; Path=/account", "https://example.com/", None)]
#[case::a_cookie_already_expired("hop=1; Max-Age=0", "https://example.com/", None)]
#[case::a_cookie_for_another_site("hop=1; Domain=example.org", "https://example.org/", None)]
#[case::a_cookie_for_a_public_suffix("hop=1; Domain=com", "https://example.com/", None)]
#[case::a_cookie_for_a_multi_label_public_suffix(
    "hop=1; Domain=co.uk",
    "https://example.co.uk/",
    None
)]
fn a_cookie_is_sent_where_a_browser_sends_it(
    #[case] set_cookie: &str,
    #[case] to: &str,
    #[case] cookie: Option<&str>,
) {
    let jar = kept("https://example.com/", &[set_cookie]);

    assert_eq!(sent(&jar, to).as_deref(), cookie);
}

#[test]
fn a_host_that_is_a_public_suffix_keeps_its_own_cookie_for_itself_alone() {
    let jar = kept("https://github.io/", &["hop=1; Domain=github.io"]);

    assert_eq!(
        (
            sent(&jar, "https://github.io/").as_deref(),
            sent(&jar, "https://someone.github.io/")
        ),
        (Some("hop=1"), None)
    );
}

#[test]
fn cookies_leave_longest_path_first_then_by_name() {
    let jar = kept(
        "https://example.com/shop/cart",
        &["b=2; Path=/", "a=1; Path=/", "c=3; Path=/shop"],
    );

    assert_eq!(
        sent(&jar, "https://example.com/shop/cart").as_deref(),
        Some("c=3; a=1; b=2")
    );
}

#[test]
fn a_later_answer_replaces_and_removes_a_cookie() {
    let jar = kept("https://example.com/", &["a=1; Path=/", "b=1; Path=/"]);
    jar.keep(
        &Url::parse("https://example.com/").unwrap(),
        &[
            ("set-cookie".to_owned(), "a=2; Path=/".to_owned()),
            ("set-cookie".to_owned(), "b=; Path=/; Max-Age=0".to_owned()),
        ],
    );

    assert_eq!(sent(&jar, "https://example.com/").as_deref(), Some("a=2"));
}

#[test]
fn a_set_cookie_past_the_size_a_browser_keeps_is_dropped() {
    let long = format!("a={}", "x".repeat(MAX_SIZE));
    let jar = kept("https://example.com/", &[&long, "b=1"]);

    assert_eq!(sent(&jar, "https://example.com/").as_deref(), Some("b=1"));
}
