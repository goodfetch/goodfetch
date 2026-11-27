//! A downloaded version of the Clean-up rules (ADR 0047): its Filter lists'
//! texts compiled on the engine's own machine, judged as the embedded ones
//! are; and the cookie sections of a list Goodfetch did not build, dropped
//! by either marker form before it is compiled.

use goodfetch_clean_up_rules::cosmetic::PageRules;
use goodfetch_clean_up_rules::list::List;
use goodfetch_clean_up_rules::matching::{Choices, Request, ResourceKind, Verdict, judge};
use goodfetch_clean_up_rules::never_block::NeverBlock;
use goodfetch_clean_up_rules::removal::{self, HostCache, Rules};
use goodfetch_clean_up_rules::rules::{FilterLists, ListTexts, without_cookie_sections};
use goodfetch_counted_work::{CountedWork, Row};
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

/// Compiled lists (as `upstream` compiles) keep cookie sections as
/// removal-only rules: their network rules never block, their cosmetic rules
/// never hide on the live page, their scriptlets are dropped, and their
/// cosmetic rules and exceptions act during extraction-side removal
/// (ADR 0047, issue 2379).
/// Covers: story:5-97, story:5-98, story:5-100
#[rstest]
#[case::easylist_to_marker(
    "[Adblock Plus 2.0]\n/banner-ad.\n! *** easylist:easylist_cookie/easylist_cookie_general_block.txt ***\n/cookie-banner.js\n! *** easylist:easylist_cookie/easylist_cookie_general_hide.txt ***\n##.cookie-leftover\nnews.example##+js(set-cookie, cookieconsent_status, allow)\nkept.example#@#.cookie-leftover\n! *** easylist:fanboy-addon/fanboy_social_general_block.txt ***\n/share-buttons.\n"
)]
#[case::fanboy_marker(
    "[Adblock Plus 2.0]\n/banner-ad.\n! easylist_cookie_general_block.txt\n/cookie-banner.js\n! easylist_cookie_general_hide.txt\n##.cookie-leftover\nnews.example##+js(set-cookie, cookieconsent_status, allow)\nkept.example#@#.cookie-leftover\n! fanboy_social_general_block.txt\n/share-buttons.\n"
)]
fn compiled_cookie_sections_are_removal_only_and_never_block_or_live_hide(#[case] text: &str) {
    let lists = FilterLists::compile(&ListTexts {
        ads: "",
        trackers: "",
        annoyances: text,
        unbreak: "",
    });
    let choices = every_list_on();
    let blocked_live = judged(
        &lists,
        "https://news.example/banner-ad.png",
        ResourceKind::Image,
    );
    let blocked_after_cookie = judged(
        &lists,
        "https://news.example/share-buttons.js",
        ResourceKind::Script,
    );
    let allowed_cookie_net = judged(
        &lists,
        "https://news.example/cookie-banner.js",
        ResourceKind::Script,
    );
    let live_rules = PageRules::of(&lists, &choices, "https://news.example/article")
        .css_rules(&["cookie-leftover"], &[]);

    let work = CountedWork::default();
    let page = r#"<!DOCTYPE html><html><body><main><p>Article.</p></main><div class="cookie-leftover">Cookie notice.</div></body></html>"#;
    #[expect(
        clippy::disallowed_methods,
        reason = "ADR 0138: a test's own parse, on no page's path"
    )]
    let doc_news = work.parse_html(Row::PageParses, page);
    let removed_news = removal::remove(
        &doc_news,
        "https://news.example/article",
        Rules {
            lists: &lists,
            version: "upstream",
            choices: &choices,
            hosts: &HostCache::new(0),
        },
        &work,
    );
    #[expect(
        clippy::disallowed_methods,
        reason = "ADR 0138: a test's own parse, on no page's path"
    )]
    let doc_kept = work.parse_html(Row::PageParses, page);
    let removed_kept = removal::remove(
        &doc_kept,
        "https://kept.example/article",
        Rules {
            lists: &lists,
            version: "upstream",
            choices: &choices,
            hosts: &HostCache::new(0),
        },
        &work,
    );

    assert_eq!(
        (
            blocked_live,
            blocked_after_cookie,
            allowed_cookie_net,
            live_rules.is_empty(),
            doc_news.html().to_string().contains("Cookie notice."),
            removed_news
                .removed
                .iter()
                .map(|element| (element.rule.as_str(), element.list))
                .collect::<Vec<_>>(),
            doc_kept.html().to_string().contains("Cookie notice."),
            removed_kept.removed.len(),
        ),
        (
            Verdict::Blocked(List::Annoyances),
            Verdict::Blocked(List::Annoyances),
            Verdict::Allowed,
            true,
            false,
            vec![(".cookie-leftover", Some(List::Annoyances))],
            true,
            0,
        )
    );
}
