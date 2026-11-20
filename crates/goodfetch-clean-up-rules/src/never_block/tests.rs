//! What the never-block list keeps loading that the Filter lists alone
//! would block, against a never-block list with no entry.

use std::sync::LazyLock;

use adblock::Engine;
use rstest::rstest;

use super::NeverBlock;
use crate::matching::{Choices, Request, ResourceKind, Verdict, judge};
use crate::rules::FilterLists;

static LISTS: LazyLock<FilterLists> =
    LazyLock::new(|| FilterLists::embedded().expect("the embedded Filter lists load"));

/// A script `url` requests from a news site's article, as every Filter
/// list on, by default, judges it with `never_block`.
fn judged(url: &str, never_block: &NeverBlock) -> Verdict {
    let request = Request {
        url,
        source_url: "https://news.example/article",
        kind: ResourceKind::Script,
        method: "GET",
    };
    let defaults = Choices {
        ads: true,
        trackers: true,
        annoyances: true,
        ..Choices::default()
    };
    judge(&request, never_block, Some(&LISTS), &defaults)
}

/// Without the never-block list, the Filter lists block challenge and
/// anti-bot requests, and Consent prompts' scripts: the entries are what
/// keeps them loading, so the Consent prompt runner always sees the prompt
/// (ADRs 0047 and 0048).
/// Covers: story:5-95, story:5-100
#[rstest]
#[case::datadome("https://js.datadome.co/tags.js")]
#[case::akamai_sensor("https://shop.example/akam/13/pixel_5f3c")]
#[case::perimeterx("https://client.perimeterx.net/PXabcdef/main.min.js")]
#[case::threatmetrix("https://h.online-metrix.net/fp/tags.js?org_id=1")]
#[case::didomis_prompt("https://sdk.privacy-center.org/loader/sdk-didomi.js")]
#[case::sourcepoints_message("https://cdn.example/unified/sourcepoint.js")]
#[case::a_sites_own_prompt_host("https://cmp.lemonde.fr/js/lemonde.min.js")]
fn the_lists_alone_block_what_the_never_block_list_keeps(#[case] url: &str) {
    let no_entry = NeverBlock {
        engine: Engine::default(),
    };

    let lists_alone = judged(url, &no_entry);
    let kept = judged(url, &NeverBlock::new());

    assert!(
        matches!(lists_alone, Verdict::Blocked(_)),
        "{lists_alone:?}"
    );
    assert_eq!(kept, Verdict::NeverBlocked);
}
