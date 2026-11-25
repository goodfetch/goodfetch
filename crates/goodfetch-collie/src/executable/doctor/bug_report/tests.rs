//! The bug-report file's reads and its URL reduction, in process: the log
//! folder's lines in, the recent Error codes out; a text in, its URLs
//! reduced or kept whole out (spec #564).
#![expect(
    clippy::disallowed_methods,
    reason = "a test writes the log files the read takes, outside any async worker"
)]

use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::json;

use super::{RECENT_ERRORS, RecentError, Urls, recent_errors, urls_in};

/// Each URL in a text is reduced to its scheme and host, its port, path,
/// query, fragment and credentials dropped; `--include-urls` keeps it
/// whole but its credentials, which are no part of a URL a bug report
/// needs. A text with no URL is kept as it is.
///
/// Covers: story:4-84
#[rstest]
#[case::path_and_query(
    "`page.goto` to https://shop.example.test:8443/cart?token=abc#top failed.",
    "`page.goto` to https://shop.example.test failed.",
    "`page.goto` to https://shop.example.test:8443/cart?token=abc#top failed."
)]
#[case::credentials(
    "proxy at http://user:pa55@proxy.example.test:3128/ refused",
    "proxy at http://proxy.example.test refused",
    "proxy at http://proxy.example.test:3128/ refused"
)]
#[case::quoted_ipv6(
    "\"ws://[::1]:9222/devtools/browser/x\" closed",
    "\"ws://[::1]\" closed",
    "\"ws://[::1]:9222/devtools/browser/x\" closed"
)]
#[case::two_urls(
    "from https://a.example.test/x to https://b.example.test/y",
    "from https://a.example.test to https://b.example.test",
    "from https://a.example.test/x to https://b.example.test/y"
)]
#[case::no_url(
    "no scheme here: a.example.test/x",
    "no scheme here: a.example.test/x",
    "no scheme here: a.example.test/x"
)]
fn each_url_is_reduced_to_its_scheme_and_host_unless_kept_whole(
    #[case] text: &str,
    #[case] reduced: &str,
    #[case] whole: &str,
) {
    assert_eq!(
        (urls_in(text, Urls::Reduced), urls_in(text, Urls::Whole)),
        (reduced.to_owned(), whole.to_owned())
    );
}

/// The recent Error codes are the log folder's lines that carry one, as an
/// `error_code` or a `code` field, from every file, oldest first by their
/// time, the newest [`RECENT_ERRORS`] kept; a line with no code, and a line
/// that is no JSON, are not.
///
/// Covers: story:4-84
#[test]
fn the_recent_error_codes_are_the_log_lines_that_carry_one_newest_kept() {
    let env = TestEnv::new();
    let folder = env.home().join("logs");
    std::fs::create_dir_all(&folder).unwrap();
    let line = |time: String, fields: serde_json::Value| {
        json!({ "timestamp": time, "level": "ERROR", "fields": fields, "target": "goodfetch" })
            .to_string()
    };
    let mut daemon: Vec<String> = (0..RECENT_ERRORS)
        .map(|second| {
            line(
                format!("2026-11-24T10:00:{:02}Z", second % 60),
                json!({ "message": "request failed", "method": "page.goto", "error_code": "navigation_failed" }),
            )
        })
        .collect();
    daemon.insert(
        0,
        line(
            "2026-11-24T09:00:00Z".to_owned(),
            json!({ "message": "the oldest", "code": "setting_invalid" }),
        ),
    );
    daemon.push(line(
        "2026-11-24T11:00:00Z".to_owned(),
        json!({ "message": "started" }),
    ));
    daemon.push("not JSON".to_owned());
    std::fs::write(folder.join("daemon-0.0.jsonl"), daemon.join("\n")).unwrap();
    std::fs::write(
        folder.join("goodfetch-collie-7.jsonl"),
        line(
            "2026-11-24T12:00:00Z".to_owned(),
            json!({ "message": "the Daemon cannot be reached", "code": "daemon_unreachable" }),
        ),
    )
    .unwrap();

    let errors = recent_errors(&folder);

    assert_eq!(
        (
            errors.len(),
            errors.first().map(|error| error.code.as_str()),
            errors.last()
        ),
        (
            RECENT_ERRORS,
            Some("navigation_failed"),
            Some(&RecentError {
                time: Some("2026-11-24T12:00:00Z".to_owned()),
                code: "daemon_unreachable".to_owned(),
                method: None,
                message: Some("the Daemon cannot be reached".to_owned()),
            }),
        )
    );
}

/// A home with no log folder has no recent Error codes.
///
/// Covers: story:4-84
#[test]
fn a_home_with_no_log_folder_has_no_recent_error_codes() {
    let env = TestEnv::new();

    assert_eq!(recent_errors(&env.home().join("logs")), Vec::new());
}
