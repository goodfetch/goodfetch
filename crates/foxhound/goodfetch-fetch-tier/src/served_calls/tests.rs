//! Covers: story:6-259

use goodfetch_protocol::foxhound::fetch::FetchTier;

use super::{Row, Service, Verdict, verdict};

/// The tiers, cheapest first, as each case lists what they serve.
const TIERS: [FetchTier; 5] = [
    FetchTier::Http,
    FetchTier::DiscoveredApi,
    FetchTier::BrowserFetch,
    FetchTier::LightBrowser,
    FetchTier::Browser,
];

use Service::{
    Escalates as E, FromMarkup as M, FromReplayedCalls as R, OverTheSameTier as O, Served as S,
    WhenAlreadyTrue as W,
};

/// Each row on each tier, as spec #582's table writes it, and what one of
/// its calls leads to there under `auto` and under a forced tier: a call a
/// tier cannot serve exactly escalates under `auto` and fails with
/// `browser_required` when forced; one it serves is served either way.
#[rstest::rstest]
#[case::read(Row::Read, Some("page.formats"), [S, S, S, S, S])]
#[case::question(Row::Question, None, [M, M, M, M, S])]
#[case::state_wait(Row::StateWait, Some("locator.wait_for"), [W, W, W, W, S])]
#[case::network_wait(Row::NetworkWait, Some("page.wait_for_response"), [E, R, E, E, S])]
#[case::script(Row::Script, Some("page.evaluate"), [E, E, E, E, S])]
#[case::layout(Row::Layout, None, [E, E, E, E, S])]
#[case::gesture(Row::Gesture, Some("locator.click"), [E, E, E, E, S])]
#[case::navigation(Row::Navigation, Some("page.goto"), [O, O, O, E, S])]
#[case::request(Row::Request, Some("page.request"), [S, S, S, S, S])]
#[case::lifecycle(Row::Lifecycle, Some("page.close"), [S, S, S, S, S])]
fn each_row_on_each_tier_is_served_exactly_or_escalates_or_is_refused_when_forced(
    #[case] row: Row,
    #[case] call: Option<&str>,
    #[case] expected: [Service; 5],
) {
    assert_eq!(TIERS.map(|tier| row.on(tier)), expected);
    if let Some(call) = call {
        let expected = |forced: bool| {
            expected.map(|service| match service {
                E if forced => Some(Verdict::BrowserRequired),
                E => Some(Verdict::Escalate),
                service => Some(Verdict::Serve(service)),
            })
        };
        assert_eq!(
            (
                Row::of(call),
                TIERS.map(|tier| verdict(call, tier, false)),
                TIERS.map(|tier| verdict(call, tier, true)),
            ),
            (Some(row), expected(false), expected(true)),
        );
    }
}

/// Every call of the catalog's page entries has a row, so none is served
/// or escalated by default.
#[test]
fn every_call_of_the_catalogs_page_entries_has_a_row() {
    let without: Vec<&str> = goodfetch_protocol::page::ENTRIES
        .operations
        .iter()
        .map(|operation| operation.name)
        .filter(|name| Row::of(name).is_none())
        .collect();
    assert_eq!(without, Vec::<&str>::new());
}

/// A call on no page has no row, so no table answers it.
#[test]
fn a_call_on_no_page_has_no_row() {
    assert_eq!(
        (
            Row::of("run.start"),
            verdict("run.start", FetchTier::Http, true)
        ),
        (None, None)
    );
}

/// On a page with no document, every row that reads, waits on or acts on
/// the document fails with `response_not_document`; the page's network,
/// navigating, requests and closing read none.
#[test]
fn the_rows_that_read_the_document_are_those_a_page_with_none_refuses() {
    let rows = [
        Row::Read,
        Row::Question,
        Row::StateWait,
        Row::NetworkWait,
        Row::Script,
        Row::Layout,
        Row::Gesture,
        Row::Navigation,
        Row::Request,
        Row::Lifecycle,
    ];
    assert_eq!(
        rows.map(Row::reads_document),
        [
            true, true, true, false, true, true, true, false, false, false
        ]
    );
}
