//! A downloaded version of the Clean-up rules (ADR 0047): its Filter lists'
//! texts compiled on the engine's own machine, judged as the embedded ones
//! are; and the cookie sections of a list Goodfetch did not build, dropped
//! by either marker form before it is compiled.

use goodfetch_clean_up_rules::list::List;
use goodfetch_clean_up_rules::matching::{Choices, Request, ResourceKind, Verdict, judge};
use goodfetch_clean_up_rules::never_block::NeverBlock;
use goodfetch_clean_up_rules::rules::{FilterLists, ListTexts, without_cookie_sections};
use rstest::rstest;

fn every_list_on() -> Choices {
    Choices {
        ads: true,
        trackers: true,
        annoyances: true,
        ..Choices::default()
    }
}

fn judged(lists: &FilterLists, url: &str, kind: ResourceKind) -> Verdict {
    judge(
        &Request {
            url,
            source_url: "https://news.example/article",
            kind,
            method: "GET",
        },
        &NeverBlock::new(),
        Some(lists),
        &every_list_on(),
    )
}

/// Covers: story:5-97
#[rstest]
#[case::its_ad(
    "https://news.example/img/banner.gif",
    ResourceKind::Image,
    Verdict::Blocked(List::Ads)
)]
#[case::its_tracker(
    "https://news.example/js/beacon.js",
    ResourceKind::Script,
    Verdict::Blocked(List::Trackers)
)]
#[case::an_exception_of_its_unbreak_list(
    "https://news.example/js/beacon.js?keep=1",
    ResourceKind::Script,
    Verdict::Allowed
)]
#[case::what_none_of_its_lists_names(
    "https://news.example/img/photo.gif",
    ResourceKind::Image,
    Verdict::Allowed
)]
fn compiled_lists_block_what_their_texts_name(
    #[case] url: &str,
    #[case] kind: ResourceKind,
    #[case] verdict: Verdict,
) {
    let lists = FilterLists::compile(&ListTexts {
        ads: "[Adblock Plus 2.0]\n/img/banner.gif\n",
        trackers: "[Adblock Plus 2.0]\n/js/beacon.js\n",
        annoyances: "[Adblock Plus 2.0]\n",
        unbreak: "[Adblock Plus 2.0]\n@@/js/beacon.js?keep=1\n",
    });

    assert_eq!(judged(&lists, url, kind), verdict);
}

/// Covers: story:5-97
#[rstest]
#[case::easylist_to_s_marker(
    "! *** easylist:easylist/easylist_general_block.txt ***\n/banner-ad.\n! *** easylist:easylist_cookie/easylist_cookie_general_block.txt ***\n/cookie-banner.js\n! *** easylist:fanboy-addon/fanboy_social_general_block.txt ***\n/share-buttons.\n",
    "! *** easylist:easylist/easylist_general_block.txt ***\n/banner-ad.\n! *** easylist:fanboy-addon/fanboy_social_general_block.txt ***\n/share-buttons.\n"
)]
#[case::fanboy_co_nz_s_marker(
    "! fanboy_annoyance_general_block.txt\n/newsletter-popup.\n! easylist_cookie_general_hide.txt\n##.cookie-notice\n! fanboy_social_general_block.txt\n/share-buttons.\n",
    "! fanboy_annoyance_general_block.txt\n/newsletter-popup.\n! fanboy_social_general_block.txt\n/share-buttons.\n"
)]
#[case::a_list_without_sections(
    "[Adblock Plus 2.0]\n/banner-ad.\n",
    "[Adblock Plus 2.0]\n/banner-ad.\n"
)]
fn a_lists_cookie_sections_are_dropped_by_either_marker(#[case] text: &str, #[case] kept: &str) {
    assert_eq!(without_cookie_sections(text), kept);
}
