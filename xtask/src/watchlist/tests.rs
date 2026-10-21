//! The comparison, in process, over hand-written copies of both watchlists
//! in their own layouts and a coverage file (the `watchlist` fixture of
//! `xtask`'s integration tests).
//!
//! Covers: story:8-61

use std::path::{Path, PathBuf};

use super::{Listed, Missing, Watchlist, missing, read_covered, read_listed};

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
/// outside webappanalyzer's Security category and Antibot-Detector's
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
        "canvasfingerprint",
    ] {
        assert!(
            !keys.iter().any(|key| key == absent),
            "{absent} in {keys:?}"
        );
    }
}
