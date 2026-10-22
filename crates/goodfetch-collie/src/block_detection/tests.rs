use std::collections::BTreeSet;

use goodfetch_protocol::block::BlockCause;
use rstest::rstest;

use super::rules::{HeaderMarker, RULES, Source};
use super::{Document, Response, classify};

/// The headers the corpus's interstitial is served with.
const INTERSTITIAL_HEADERS: &[(&str, &str)] =
    &[("cf-mitigated", "challenge"), ("cache-control", "no-store")];

/// Each captured page of the Proving ground, with the status and headers it
/// was served with, classifies to the Block cause its corpus records by the
/// rule written for it; a page no rule decides, a vendor's script or widget
/// on an ordinary page among them, is no block. Stories 8-58 and 8-59 are
/// declared by the same corpus met by a Session in the engine, which raising
/// `blocked` brings (#711); until then they are excepted
/// (`features/exceptions.toml`), since a unit test proves no story.
#[rstest]
#[case::challenge_header(403, INTERSTITIAL_HEADERS, include_str!("fixtures/interstitial.html"), Some((BlockCause::Challenge, "cloudflare_challenge")))]
#[case::interstitial_without_its_header(403, &[], include_str!("fixtures/interstitial.html"), Some((BlockCause::Challenge, "interstitial_challenge")))]
#[case::tier_move_challenge(403, &[], include_str!("fixtures/protection-challenge.html"), Some((BlockCause::Challenge, "interstitial_challenge")))]
#[case::press_and_hold(403, &[], include_str!("fixtures/press-and-hold.html"), Some((BlockCause::Captcha, "press_and_hold")))]
#[case::captcha(200, &[], include_str!("fixtures/captcha.html"), Some((BlockCause::Captcha, "captcha_widget")))]
#[case::cloudflare_block(403, &[("cache-control", "no-store")], include_str!("fixtures/cloudflare-block.html"), Some((BlockCause::IpBlock, "cloudflare_block")))]
#[case::incapsula_block(403, &[], include_str!("fixtures/incapsula-block.html"), Some((BlockCause::IpBlock, "incapsula_block")))]
#[case::sucuri_block(403, &[], include_str!("fixtures/sucuri-block.html"), Some((BlockCause::IpBlock, "sucuri_block")))]
#[case::ip_block(403, &[], include_str!("fixtures/ip-block.html"), Some((BlockCause::IpBlock, "ip_block_notice")))]
#[case::rate_limit(429, &[("retry-after", "120")], include_str!("fixtures/rate-limit.html"), Some((BlockCause::RateLimit, "too_many_requests")))]
#[case::vendor_script_only(200, &[], include_str!("fixtures/vendor-script.html"), None)]
#[case::captcha_widget_on_a_contact_form(200, &[], "<form><div class=\"g-recaptcha\"></div><button>Send</button></form>", None)]
#[case::captcha_widget_on_a_login_form(200, &[], "<h1>Sign in</h1><form action=\"/login\" method=\"post\"><input type=\"email\" name=\"email\"><input type=\"password\" name=\"password\"><p>Verify you are human</p><div class=\"g-recaptcha\"></div><button>Sign in</button></form>", None)]
#[case::ordinary_article(200, &[], include_str!("fixtures/article.html"), None)]
#[case::not_found(404, &[], include_str!("fixtures/not-found.html"), None)]
#[case::interstitial_words_served_as_content(200, &[], include_str!("fixtures/interstitial.html"), None)]
#[case::firewall_words_served_as_content(200, &[], include_str!("fixtures/sucuri-block.html"), None)]
fn a_captured_page_classifies_to_its_block_cause(
    #[case] status: u16,
    #[case] headers: &[(&str, &str)],
    #[case] markup: &str,
    #[case] expected: Option<(BlockCause, &str)>,
) {
    let classification = classify(&Response { status, headers }, &Document { markup });

    assert_eq!(
        classification.map(|classification| (classification.cause, classification.evidence.rule)),
        expected
    );
}

/// A classification's evidence names the rule, where it comes from, the
/// license it is used under and what it matched.
///
/// Covers: story:8-60
#[rstest]
#[case::goodfetchs_own(
    403,
    &[],
    include_str!("fixtures/interstitial.html"),
    ("interstitial_challenge", "Goodfetch, from the Proving ground's block-detection corpus", "MIT OR Apache-2.0", 403, vec![], vec!["just a moment", "javascript and cookies"])
)]
#[case::seeded_from_a_header(
    403,
    INTERSTITIAL_HEADERS,
    include_str!("fixtures/interstitial.html"),
    ("cloudflare_challenge", "Spider", "MIT", 403, vec![HeaderMarker { name: "cf-mitigated", value: "challenge" }], vec![])
)]
#[case::seeded_from_a_page(
    403,
    &[],
    include_str!("fixtures/incapsula-block.html"),
    ("incapsula_block", "wafw00f", "BSD-3-Clause", 403, vec![], vec!["request unsuccessful", "incapsula incident id"])
)]
fn a_classification_names_the_rule_its_source_and_license_and_what_it_matched(
    #[case] status: u16,
    #[case] headers: &[(&str, &str)],
    #[case] markup: &str,
    #[case] expected: (&str, &str, &str, u16, Vec<HeaderMarker>, Vec<&str>),
) {
    let classification =
        classify(&Response { status, headers }, &Document { markup }).expect("the page is a block");

    let evidence = classification.evidence;
    assert_eq!(
        (
            evidence.rule,
            evidence.source,
            evidence.license,
            evidence.status,
            evidence.headers,
            evidence.markers
        ),
        expected
    );
}

/// The outside sources the rules name, with their licenses, are exactly the
/// inputs `third-party.toml` declares for the notice inventory: no rule's
/// source is missing from it, and it lists no source no rule uses.
///
/// Covers: story:8-60
#[test]
fn the_notice_inventory_declares_exactly_the_rules_outside_sources() {
    let declared: toml::Table =
        toml::from_str(include_str!("third-party.toml")).expect("third-party.toml is TOML");

    let named: BTreeSet<(&str, &str)> = RULES
        .iter()
        .filter_map(|rule| match rule.source {
            Source::Seeded(outside) => Some((outside.name, outside.license)),
            Source::Goodfetch => None,
        })
        .collect();

    let inputs: BTreeSet<(&str, &str)> = declared["input"]
        .as_array()
        .expect("inputs are an array")
        .iter()
        .map(|input| {
            (
                input["name"].as_str().expect("an input's name"),
                input["license"].as_str().expect("an input's license"),
            )
        })
        .collect();
    assert_eq!(named, inputs);
}

/// Every vendor `vendors.toml` counts as covered, which the watchlist job
/// opens no issue for, names at least one rule, and every rule it names
/// exists, so no vendor is counted covered without a rule.
///
/// Covers: story:8-61
#[test]
fn each_covered_vendor_names_rules_that_exist() {
    let file: toml::Table =
        toml::from_str(include_str!("vendors.toml")).expect("vendors.toml is TOML");
    let rules: BTreeSet<&str> = RULES.iter().map(|rule| rule.name).collect();

    for vendor in file["vendor"].as_array().expect("vendors are an array") {
        let name = vendor["name"].as_str().expect("a vendor's name");
        let named: Vec<&str> = vendor["rules"]
            .as_array()
            .expect("a vendor's rules")
            .iter()
            .map(|rule| rule.as_str().expect("a rule's name"))
            .collect();
        assert!(!named.is_empty(), "{name} names no rule");
        for rule in named {
            assert!(
                rules.contains(rule),
                "{name} names {rule}, which is no rule"
            );
        }
    }
}
