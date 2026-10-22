//! The comparison, in process, over hand-written copies of both watchlists
//! in their own layouts and a coverage file (`fixtures/`, as the README of
//! `xtask`'s `watchlist` integration fixture describes them).
//!
//! Covers: story:8-61

use std::path::{Path, PathBuf};

use rstest::rstest;

use super::{
    Coverage, Listed, Missing, Watchlist, body, describes_bot_protection, missing, read_listed,
    title, unkeyed,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/watchlist/fixtures")
}

fn listed(watchlist: Watchlist, name: &str) -> Listed {
    Listed {
        watchlist,
        name: name.to_owned(),
    }
}

fn missing_from_the_fixtures() -> Vec<Missing> {
    let root = fixtures();
    let listed = read_listed(&[
        (Watchlist::WebAppAnalyzer, &root.join("webappanalyzer")),
        (Watchlist::AntibotDetector, &root.join("antibot-detector")),
    ])
    .unwrap();
    missing(
        &listed,
        &Coverage::parse(include_str!("fixtures/vendors.toml")).unwrap(),
    )
}

/// Each vendor either list names that no rule covers is missing once, with
/// every listing of it: `DataDome`, listed by both, is one vendor.
#[test]
fn each_listed_vendor_no_rule_covers_is_missing_once() {
    assert_eq!(
        missing_from_the_fixtures(),
        vec![
            Missing {
                key: "akamai".to_owned(),
                listings: vec![listed(Watchlist::AntibotDetector, "akamai")],
            },
            Missing {
                key: "datadome".to_owned(),
                listings: vec![
                    listed(Watchlist::WebAppAnalyzer, "DataDome"),
                    listed(Watchlist::AntibotDetector, "datadome"),
                ],
            },
            Missing {
                key: "geetest".to_owned(),
                listings: vec![listed(Watchlist::AntibotDetector, "geetest")],
            },
            Missing {
                key: "kasada".to_owned(),
                listings: vec![listed(Watchlist::WebAppAnalyzer, "Kasada")],
            },
        ]
    );
}

/// A covered vendor is never missing, under its own name or a name the
/// coverage file gives for it, in either list's spelling; technologies
/// outside webappanalyzer's Security category, Security technologies that
/// protect from no bots (HSTS, a bug bounty platform) and Antibot-Detector's
/// fingerprinting techniques are no vendors.
#[rstest]
#[case::covered_by_name("cloudflare")]
#[case::covered_by_a_listed_as_name("cloudflarebotmanagement")]
#[case::covered_in_another_spelling("recaptcha")]
#[case::outside_the_security_category("jquery")]
#[case::security_but_no_bots("hsts")]
#[case::a_bug_bounty_platform("hackerone")]
#[case::a_fingerprinting_technique("canvasfingerprint")]
fn no_covered_vendor_and_no_other_listing_is_missing(#[case] absent: &str) {
    let keys: Vec<String> = missing_from_the_fixtures()
        .into_iter()
        .map(|vendor| vendor.key)
        .collect();

    assert!(!keys.iter().any(|key| key == absent), "{keys:?}");
}

/// A webappanalyzer Security technology is a bot-protection vendor when its
/// name or description says bot, CAPTCHA or Turing test, in any case and
/// spelling.
#[rstest]
#[case::captcha_in_the_name("Yandex SmartCaptcha", None)]
#[case::antibot_in_the_name("AntiBot.Cloud", Some("A cloud service for websites."))]
#[case::bot_driven("Kasada", Some("Defends web applications against bot-driven abuse."))]
#[case::anti_bot("hCaptcha", Some("An anti-bot service."))]
#[case::bots_in_capitals("Acme Guard", Some("Stops BOTS at the edge."))]
#[case::turing_test(
    "Acme Check",
    Some("A public Turing test telling humans and machines apart.")
)]
fn a_technology_that_says_bot_captcha_or_turing_test_is_a_bot_protection_vendor(
    #[case] name: &str,
    #[case] description: Option<&str>,
) {
    assert!(describes_bot_protection(name, description));
}

/// A Security technology that says none of these is no bot-protection
/// vendor, nor is one whose word merely contains "bot".
#[rstest]
#[case::hsts("HSTS", Some("Tells browsers to reach a site over HTTPS only."))]
#[case::robots_txt("HackerOne", Some("Its scanners read robots.txt first."))]
#[case::a_word_containing_bot("Abbott Shield", Some("A firewall for Botswana's banks."))]
#[case::no_description("Mollom", None)]
fn a_technology_that_says_none_of_these_is_no_bot_protection_vendor(
    #[case] name: &str,
    #[case] description: Option<&str>,
) {
    assert!(!describes_bot_protection(name, description));
}

/// Writes `files`, each a path and its text, under a new folder.
fn checkout(files: &[(&str, &str)]) -> tempfile::TempDir {
    let folder = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let path = folder.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    folder
}

/// A layout change upstream fails the job instead of finding nothing: a
/// webappanalyzer with no Security category, and either list naming no
/// vendor at all.
#[rstest]
#[case::no_security_category(
    Watchlist::WebAppAnalyzer,
    &[
        ("src/categories.json", r#"{ "1": { "name": "CMS" } }"#),
        ("src/technologies/a.json", r#"{ "Acme": { "cats": [1], "description": "Stops bots." } }"#),
    ]
)]
#[case::no_bot_vendor_in_the_security_category(
    Watchlist::WebAppAnalyzer,
    &[
        ("src/categories.json", r#"{ "16": { "name": "Security" } }"#),
        ("src/technologies/h.json", r#"{ "HSTS": { "cats": [16], "description": "HTTPS only." } }"#),
    ]
)]
#[case::no_antibot_or_captcha_detector(
    Watchlist::AntibotDetector,
    &[("detectors/index.json", r#"{ "antibot": { "detectors": [] }, "captcha": { "detectors": [] } }"#)]
)]
fn a_watchlist_that_names_no_vendor_fails(
    #[case] watchlist: Watchlist,
    #[case] files: &[(&str, &str)],
) {
    let folder = checkout(files);

    assert!(read_listed(&[(watchlist, folder.path())]).is_err());
}

/// A listed name with no letter or digit, which no key can compare, is
/// reported rather than dropped.
#[test]
fn a_listed_name_with_no_letter_or_digit_is_reported() {
    let listings = [
        listed(Watchlist::AntibotDetector, "datadome"),
        listed(Watchlist::AntibotDetector, "--"),
    ];

    assert_eq!(unkeyed(&listings), vec![&listings[1]]);
}

/// A name a watchlist lists reaches the issue as plain text: a backtick,
/// mention, link or tag in it never becomes live Markdown, in the title or
/// the body.
#[test]
fn a_listed_name_reaches_the_issue_as_plain_text() {
    let vendor = Missing {
        key: "evil".to_owned(),
        listings: vec![listed(
            Watchlist::AntibotDetector,
            "Evil` @everyone [x](https://e.test) <b>",
        )],
    };

    assert_eq!(
        (
            title(&vendor),
            body(&vendor).lines().nth(2).map(str::to_owned)
        ),
        (
            "Block detection: no rule covers Evil everyone xhttpse.test b, which a watchlist lists"
                .to_owned(),
            Some(
                "- Scrapfly's Antibot-Detector (NPOSL-3.0) lists `Evil everyone xhttpse.test b`."
                    .to_owned()
            ),
        )
    );
}

/// A name past 64 characters is cut, so no listing fills an issue.
#[test]
fn a_long_listed_name_is_cut() {
    let vendor = Missing {
        key: "long".to_owned(),
        listings: vec![listed(Watchlist::WebAppAnalyzer, &"Bot".repeat(30))],
    };

    assert_eq!(
        title(&vendor),
        format!(
            "Block detection: no rule covers {}…, which a watchlist lists",
            "Bot".repeat(21) + "B"
        )
    );
}

/// A vendor no rule covers yet that the lists spell differently is missing
/// once, keyed by the name the coverage file folds its spellings into.
#[test]
fn an_uncovered_vendor_the_lists_spell_differently_is_missing_once() {
    let coverage = Coverage::parse(
        "vendor = []\n\n[[uncovered]]\nname = \"Akamai Bot Manager\"\nlisted_as = [\"akamai\"]\n",
    )
    .unwrap();
    let listings = [
        listed(Watchlist::WebAppAnalyzer, "Akamai Bot Manager"),
        listed(Watchlist::AntibotDetector, "akamai"),
    ];

    assert_eq!(
        missing(&listings, &coverage),
        vec![Missing {
            key: "akamaibotmanager".to_owned(),
            listings: listings.to_vec(),
        }]
    );
}
