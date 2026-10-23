//! The comparison, in process, over hand-written copies of both watchlists
//! in their own layouts and a coverage file (the `watchlist` fixture of
//! `xtask`'s integration tests).
//!
//! Covers: story:8-61

use std::path::{Path, PathBuf};

use super::{
    Listed, Missing, Watchlist, describes_bot_protection, missing, read_covered, read_listed,
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/fixtures/watchlist")
}

fn listed(watchlist: Watchlist, name: &str) -> Listed {
    Listed {
        watchlist,
        name: name.to_owned(),
    }
}

fn missing_from_the_fixture() -> Vec<Missing> {
    let root = fixture();
    let listed = read_listed(&[
        (Watchlist::WebAppAnalyzer, &root.join("webappanalyzer")),
        (Watchlist::AntibotDetector, &root.join("antibot-detector")),
    ])
    .unwrap();
    missing(&listed, &read_covered(&root).unwrap())
}

/// Each vendor either list names that no rule covers is missing once, with
/// every listing of it: `DataDome`, listed by both, is one vendor.
#[test]
fn each_listed_vendor_no_rule_covers_is_missing_once() {
    assert_eq!(
        missing_from_the_fixture(),
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
#[test]
fn no_covered_vendor_and_no_other_listing_is_missing() {
    let keys: Vec<String> = missing_from_the_fixture()
        .into_iter()
        .map(|vendor| vendor.key)
        .collect();

    for absent in [
        "cloudflare",
        "cloudflarebotmanagement",
        "recaptcha",
        "jquery",
        "hsts",
        "hackerone",
        "canvasfingerprint",
    ] {
        assert!(
            !keys.iter().any(|key| key == absent),
            "{absent} in {keys:?}"
        );
    }
}

/// A webappanalyzer Security technology is a bot-protection vendor when its
/// name or description says bot, CAPTCHA or Turing test, in any case and
/// spelling; a word that merely contains "bot" does not count.
#[test]
fn a_technology_is_a_bot_protection_vendor_when_its_name_or_description_says_so() {
    for (name, description) in [
        ("Yandex SmartCaptcha", None),
        ("AntiBot.Cloud", Some("A cloud service for websites.")),
        (
            "Kasada",
            Some("Defends web applications against bot-driven abuse."),
        ),
        ("hCaptcha", Some("An anti-bot service.")),
        ("Acme Guard", Some("Stops BOTS at the edge.")),
        (
            "Acme Check",
            Some("A public Turing test telling humans and machines apart."),
        ),
    ] {
        assert!(
            describes_bot_protection(name, description),
            "{name}: {description:?}"
        );
    }
    for (name, description) in [
        (
            "HSTS",
            Some("Tells browsers to reach a site over HTTPS only."),
        ),
        ("HackerOne", Some("Its scanners read robots.txt first.")),
        ("Abbott Shield", Some("A firewall for Botswana's banks.")),
        ("Mollom", None),
    ] {
        assert!(
            !describes_bot_protection(name, description),
            "{name}: {description:?}"
        );
    }
}
