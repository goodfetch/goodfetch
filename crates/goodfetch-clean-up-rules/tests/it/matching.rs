//! `judge` on the embedded Filter lists: the never-block list first, then
//! each list that is on, then `browser.block_resources` (ADRs 0047, 0117).

use std::sync::LazyLock;

use goodfetch_clean_up_rules::list::List;
use goodfetch_clean_up_rules::matching::{Choices, Request, ResourceKind, Verdict, judge};
use goodfetch_clean_up_rules::never_block::{self, NeverBlock};
use goodfetch_clean_up_rules::rules::{EmbeddedList, FilterLists, LoadError};
use rstest::rstest;

static LISTS: LazyLock<FilterLists> =
    LazyLock::new(|| FilterLists::embedded().expect("the embedded Filter lists load"));

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
#[test]
fn lists_loaded_on_their_own_block_as_the_embedded_lists_do() {
    let loaded = [List::Annoyances, List::Ads, List::Trackers]
        .map(|list| EmbeddedList::load(list).expect("the embedded Filter list loads"));
    let lists = FilterLists::of_embedded(loaded).expect("one of each list");

    for (url, kind, list) in [
        (
            "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js",
            ResourceKind::Script,
            List::Ads,
        ),
        (
            "https://www.google-analytics.com/collect?v=1",
            ResourceKind::Ping,
            List::Trackers,
        ),
        (
            "https://static.addtoany.com/menu/page.js",
            ResourceKind::Script,
            List::Annoyances,
        ),
    ] {
        let verdict = judge(
            &request(url, kind),
            &NeverBlock::new(),
            Some(&lists),
            &defaults(),
        );
        assert_eq!(verdict, Verdict::Blocked(list), "{url}");
    }
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
#[test]
fn the_sites_own_content_goes_on() {
    for (url, kind) in [
        ("https://news.example/article", ResourceKind::Document),
        ("https://news.example/static/site.js", ResourceKind::Script),
        ("https://news.example/images/photo.jpg", ResourceKind::Image),
    ] {
        assert_eq!(judged(url, kind, &defaults()), Verdict::Allowed, "{url}");
    }
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
/// and over `browser.block_resources`, as an image, a script or a report.
/// Covers: story:5-95
#[test]
fn every_never_block_entry_loads_over_every_list_and_block_resources() {
    let choices = everything_blocked();
    for entry in never_block::entries() {
        for kind in [
            ResourceKind::Script,
            ResourceKind::Image,
            ResourceKind::Xhr,
            ResourceKind::Subdocument,
        ] {
            assert_eq!(
                judged(entry.example, kind, &choices),
                Verdict::NeverBlocked,
                "{} ({}) as {kind:?}",
                entry.vendor,
                entry.example
            );
        }
    }
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
    let lists_alone = judge(
        &request(url, ResourceKind::Script),
        &NeverBlock::empty(),
        Some(&LISTS),
        &defaults(),
    );
    assert!(
        matches!(lists_alone, Verdict::Blocked(_)),
        "{lists_alone:?}"
    );
    assert_eq!(
        judged(url, ResourceKind::Script, &defaults()),
        Verdict::NeverBlocked
    );
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
