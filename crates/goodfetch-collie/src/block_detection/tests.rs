use goodfetch_protocol::block::BlockCause;
use rstest::rstest;

use super::{Document, Response, classify};

/// Each captured page of the Proving ground, with the status it was served
/// with, classifies to the Block cause its corpus records; a page no rule
/// decides is no block.
///
/// Covers: story:8-58, story:8-59
#[rstest]
#[case::corpus_interstitial(403, include_str!("fixtures/interstitial.html"), Some(BlockCause::Challenge))]
#[case::tier_move_challenge(403, include_str!("fixtures/protection-challenge.html"), Some(BlockCause::Challenge))]
#[case::vendor_script_only(200, include_str!("fixtures/vendor-script.html"), None)]
#[case::ordinary_article(200, include_str!("fixtures/article.html"), None)]
#[case::not_found(404, include_str!("fixtures/not-found.html"), None)]
#[case::interstitial_words_served_as_content(200, include_str!("fixtures/interstitial.html"), None)]
fn a_captured_page_classifies_to_its_block_cause(
    #[case] status: u16,
    #[case] markup: &str,
    #[case] expected: Option<BlockCause>,
) {
    let classification = classify(&Response { status }, &Document { markup });

    assert_eq!(
        classification.map(|classification| classification.cause),
        expected
    );
}

#[test]
fn a_classification_names_the_rule_its_source_and_license_and_what_it_matched() {
    let classification = classify(
        &Response { status: 403 },
        &Document {
            markup: include_str!("fixtures/interstitial.html"),
        },
    )
    .expect("the interstitial is a challenge");

    let evidence = classification.evidence;
    assert_eq!(
        (
            evidence.rule,
            evidence.source,
            evidence.license,
            evidence.status,
            evidence.markers
        ),
        (
            "interstitial_challenge",
            "Goodfetch, from the Proving ground's block-detection corpus",
            "MIT OR Apache-2.0",
            403,
            vec!["just a moment", "javascript and cookies"]
        )
    );
}
