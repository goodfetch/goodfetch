//! A kind of "Needs you" item added from outside the engine (spec #592;
//! ADR 0087): a source this crate's test passes in as a value of its own
//! contribution, of a kind neither the engine nor Collie names, lists its
//! item in `needs_you.list` beside the engine's own sources.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_engine::needs_you::{
    Answering, Changes, NeedsYouSource, Primitives, SourceContext, SourceError,
};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::{Composition, Contribution, Engine};
use goodfetch_protocol::needs_you::NeedsYouItem;
use goodfetch_protocol::point_in_time;
use goodfetch_test_support::TestEnv;
use serde_json::{Map, json};

use super::{Reply, crawl_on, dependencies};

/// The kind this test's source fills, which no catalog names.
const KIND: &str = "crawl_test_waiting";

/// A source of one item, which never changes.
struct Waiting;

impl Primitives for Waiting {
    fn dismissable(&self) -> bool {
        false
    }

    fn items<'a>(
        &'a self,
        _: &'a SourceContext,
    ) -> Answering<'a, Result<Vec<NeedsYouItem>, SourceError>> {
        Box::pin(async {
            Ok(vec![NeedsYouItem {
                id: format!("{KIND}:1"),
                kind: KIND.to_owned(),
                since: point_in_time::from_rfc3339("2026-10-24T08:00:00Z").unwrap(),
                run: None,
                schedule: None,
                summary: "A crawl waits for a person.".to_owned(),
                actions: Vec::new(),
                details: Map::new(),
            }])
        })
    }

    fn changes(&self, _: &SourceContext) -> Changes {
        Changes::none()
    }
}

/// Foxhound's engine, with this test's source of "Needs you" items.
fn with_source() -> Engine {
    let identity = super::engine().identity().clone();
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(goodfetch_frontier::contribution())
        .with(goodfetch_session_pool::contribution())
        .with(super::super::contribution(
            goodfetch_fetch_tier::FetchTiers::new(),
        ))
        .with(
            Contribution::new("goodfetch-crawl tests")
                .with_needs_you_source(NeedsYouSource::new(Waiting)),
        );
    Engine::new(identity, &composition).unwrap()
}

/// Covers: story:12-23, operation:needs_you.list
#[test]
fn a_kind_another_crate_passes_in_is_listed_without_the_engine_naming_it() {
    let env = TestEnv::new();

    let (listed, _) = crawl_on(
        &with_source(),
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call("needs_you.list", json!({}), &mut |_| Reply::Silent)
                .await
        },
    );

    let items = listed["result"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        items
            .iter()
            .map(|item| (item["kind"].clone(), item["summary"].clone()))
            .collect::<Vec<_>>(),
        vec![(json!(KIND), json!("A crawl waits for a person."))],
        "{listed}"
    );
}
