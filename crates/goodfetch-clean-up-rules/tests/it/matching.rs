//! `judge` on the embedded Filter lists: the never-block list first, then
//! each list that is on, then `browser.block_resources` (ADRs 0047, 0117).

use std::sync::LazyLock;

use goodfetch_clean_up_rules::list::List;
use goodfetch_clean_up_rules::matching::{Choices, Request, ResourceKind, Verdict, judge};
use goodfetch_clean_up_rules::never_block::NeverBlock;
use goodfetch_clean_up_rules::rules::{EmbeddedList, FilterLists, LoadError};
use rstest::rstest;
use serde::Deserialize;

static LISTS: LazyLock<FilterLists> =
    LazyLock::new(|| FilterLists::embedded().expect("the embedded Filter lists load"));

/// Each embedded list loaded on its own, put together in another order
/// than the embedded lists'.
static LOADED_ON_THEIR_OWN: LazyLock<FilterLists> = LazyLock::new(|| {
    let loaded = [List::Annoyances, List::Ads, List::Trackers]
        .map(|list| EmbeddedList::load(list).expect("the embedded Filter list loads"));
    FilterLists::of_embedded(loaded).expect("one of each list")
});

/// The never-block list's file, which the build compiles into the list.
#[derive(Deserialize)]
struct NeverBlockFile {
    entry: Vec<NeverBlockEntry>,
}

/// One entry of it, by the vendor it is for and the request it must keep
/// loading.
#[derive(Deserialize)]
struct NeverBlockEntry {
    vendor: String,
    example: String,
}

static NEVER_BLOCK_ENTRIES: LazyLock<Vec<NeverBlockEntry>> = LazyLock::new(|| {
    toml::from_str::<NeverBlockFile>(include_str!("../../rules/never-block.toml"))
        .expect("the never-block list is well formed")
        .entry
});

/// Every Filter list on, as by default, and nothing else.
fn defaults() -> Choices {
    Choices {
        ads: true,
        trackers: true,
        annoyances: true,
        ..Choices::default()
    }
}

/// Every Filter list on and every resource kind `browser.block_resources`
/// takes blocked.
fn everything_blocked() -> Choices {
    Choices {
        block_resources: vec![
            ResourceKind::Image,
            ResourceKind::Media,
            ResourceKind::Font,
            ResourceKind::Stylesheet,
        ],
        ..defaults()
    }
}

fn request(url: &str, kind: ResourceKind) -> Request<'_> {
    Request {
        url,
        source_url: "https://news.example/article",
        kind,
        method: "GET",
    }
}

fn judged(url: &str, kind: ResourceKind, choices: &Choices) -> Verdict {
    judge(
        &request(url, kind),
        &NeverBlock::new(),
        Some(&LISTS),
        choices,
    )
}

/// Covers: story:5-94
#[rstest]
#[case::an_ad_script(
    "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js",
    ResourceKind::Script,
    List::Ads
)]
#[case::an_ad_image(
    "https://news.example/ads/banner-728x90.gif",
    ResourceKind::Image,
    List::Ads
)]
#[case::a_tracker(
    "https://www.google-analytics.com/collect?v=1",
    ResourceKind::Ping,
    List::Trackers
)]
#[case::an_annoyance(
    "https://static.addtoany.com/menu/page.js",
    ResourceKind::Script,
    List::Annoyances
)]
fn a_filter_list_blocks_its_ads_trackers_and_annoyances(
    #[case] url: &str,
    #[case] kind: ResourceKind,
    #[case] list: List,
) {
    assert_eq!(judged(url, kind, &defaults()), Verdict::Blocked(list));
}

/// Each embedded list loads on its own, so that the engine loads them side
/// by side (#2688), and put together in any order they block what each
/// blocks among the embedded lists.
/// Covers: story:5-94
#[rstest]
#[case::an_ad_script(
    "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js",
    ResourceKind::Script,
    List::Ads
)]
#[case::a_tracker(
    "https://www.google-analytics.com/collect?v=1",
    ResourceKind::Ping,
    List::Trackers
)]
#[case::an_annoyance(
    "https://static.addtoany.com/menu/page.js",
    ResourceKind::Script,
    List::Annoyances
)]
fn lists_loaded_on_their_own_block_as_the_embedded_lists_do(
    #[case] url: &str,
    #[case] kind: ResourceKind,
    #[case] list: List,
) {
    let verdict = judge(
        &request(url, kind),
        &NeverBlock::new(),
        Some(&LOADED_ON_THEIR_OWN),
        &defaults(),
    );

    assert_eq!(verdict, Verdict::Blocked(list));
}

/// Lists put together without one of each name the list that is missing.
/// Covers: story:5-94
#[test]
fn lists_put_together_without_one_name_the_missing_list() {
    let loaded = [List::Ads, List::Ads, List::Trackers]
        .map(|list| EmbeddedList::load(list).expect("the embedded Filter list loads"));

    let missing = FilterLists::of_embedded(loaded).err();

    assert!(
        matches!(missing, Some(LoadError::Missing { list: "annoyances" })),
        "{missing:?}"
    );
}

/// Covers: story:5-94
#[rstest]
#[case::its_document("https://news.example/article", ResourceKind::Document)]
#[case::its_script("https://news.example/static/site.js", ResourceKind::Script)]
#[case::its_image("https://news.example/images/photo.jpg", ResourceKind::Image)]
fn the_sites_own_content_goes_on(#[case] url: &str, #[case] kind: ResourceKind) {
    assert_eq!(judged(url, kind, &defaults()), Verdict::Allowed);
}

/// Covers: story:5-94
#[test]
fn a_list_turned_off_blocks_nothing() {
    let url = "https://www.google-analytics.com/collect?v=1";
    let trackers_off = Choices {
        trackers: false,
        ..defaults()
    };
    assert_eq!(
        judged(url, ResourceKind::Ping, &trackers_off),
        Verdict::Allowed
    );
    assert_eq!(
        judge(
            &request(url, ResourceKind::Ping),
            &NeverBlock::new(),
            None,
            &Choices::default()
        ),
        Verdict::Allowed
    );
}

/// A redirect rule whose stand-in the alias table names answers with it:
/// EasyPrivacy's `||static.chartbeat.com/js/chartbeat.js$redirect=chartbeat.js`
/// gives DuckDuckGo's Chartbeat surrogate, which defines `pSUPERFLY`.
/// Covers: story:5-94
#[test]
fn a_redirect_rule_answers_with_its_stand_in() {
    let verdict = judged(
        "https://static.chartbeat.com/js/chartbeat.js",
        ResourceKind::Script,
        &defaults(),
    );
    let Verdict::StandIn { mime, body, .. } = verdict else {
        panic!("expected a stand-in, got {verdict:?}");
    };
    assert_eq!(mime, "application/javascript");
    let script = data_encoding::BASE64
        .decode(body.as_bytes())
        .expect("the body is Base64");
    assert!(String::from_utf8_lossy(&script).contains("window.pSUPERFLY = noopProxy"));
}

/// Covers: story:5-94
#[test]
fn a_sites_own_script_passes_the_lists_with_first_party_scripts() {
    let url = "https://news.example/js/adobe-analytics.js";
    let own = Request {
        source_url: "https://www.news.example/article",
        ..request(url, ResourceKind::Script)
    };
    let first_party = Choices {
        first_party_scripts: true,
        ..defaults()
    };
    assert!(matches!(
        judge(&own, &NeverBlock::new(), Some(&LISTS), &defaults()),
        Verdict::Blocked(_)
    ));
    assert_eq!(
        judge(&own, &NeverBlock::new(), Some(&LISTS), &first_party),
        Verdict::Allowed
    );
}

/// Each never-block entry keeps its example loading over every Filter list
/// and over `browser.block_resources`, as a script, an image, a report or
/// a frame.
/// Covers: story:5-95
#[rstest]
#[case::as_a_script(ResourceKind::Script)]
#[case::as_an_image(ResourceKind::Image)]
#[case::as_a_report(ResourceKind::Xhr)]
#[case::as_a_frame(ResourceKind::Subdocument)]
fn every_never_block_entry_loads_over_every_list_and_block_resources(#[case] kind: ResourceKind) {
    let choices = everything_blocked();

    let not_kept: Vec<(&str, &str, Verdict)> = NEVER_BLOCK_ENTRIES
        .iter()
        .map(|entry| {
            (
                entry.vendor.as_str(),
                entry.example.as_str(),
                judged(&entry.example, kind, &choices),
            )
        })
        .filter(|(_, _, verdict)| *verdict != Verdict::NeverBlocked)
        .collect();

    assert!(not_kept.is_empty(), "{not_kept:?}");
}

/// Covers: story:5-96
#[rstest]
#[case::image("https://news.example/images/photo.jpg", ResourceKind::Image)]
#[case::media("https://news.example/video/clip.mp4", ResourceKind::Media)]
#[case::font("https://news.example/fonts/body.woff2", ResourceKind::Font)]
#[case::stylesheet("https://news.example/css/site.css", ResourceKind::Stylesheet)]
fn block_resources_blocks_its_kinds_only(#[case] url: &str, #[case] kind: ResourceKind) {
    let only_this = Choices {
        block_resources: vec![kind],
        ..Choices::default()
    };
    assert_eq!(judged(url, kind, &only_this), Verdict::ResourceBlocked);
    assert_eq!(
        judged(
            "https://news.example/static/site.js",
            ResourceKind::Script,
            &only_this
        ),
        Verdict::Allowed
    );
}
