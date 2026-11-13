//! "Needs you" at its module interface (spec #592's Seam 3), with real
//! SQLite and `Clock::paused`: the items of every source passed in as a
//! value, merged into one list the longest waiting first and read a page
//! at a time; an item a source resolves leaving it with
//! `needs_you.changed`; and a dismissal, which only a source that allows it
//! takes, kept in the Run index so that the item stays hidden after the
//! engine starts again.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059)"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{EngineInstance, ProductVersion};
use goodfetch_protocol::needs_you::{
    NeedsYouChange, NeedsYouItem, NeedsYouList, NeedsYouListParams, NeedsYouRemoval, PAGE,
};
use goodfetch_protocol::run::PointInTime;
use rstest::rstest;
use serde_json::{Map, json};
use tokio::sync::watch;

use super::consent::ConsentDomains;
use super::refetch::coverage;
use super::{Answering, Changes, NeedsYou, NeedsYouSource, Primitives, SourceContext, SourceError};
use crate::clock::Clock;
use crate::events::{Events, EventsOptions, Subscription};
use crate::governor::StorageSample;
use crate::home::Home;
use crate::known_certificates::KnownCertificates;
use crate::ledger::Ledger;
use crate::run_index::RunIndex;
use crate::settings::{Settings, SettingsFiles};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// `consent.domains` keys its entries as every host is keyed: however a
/// person writes a domain, it covers the wall a Run recorded under its key.
#[rstest]
#[case::written_as_recorded(json!(["news.example"]), "news.example", true)]
#[case::in_capitals_and_fully_qualified(json!(["News.Example."]), "news.example", true)]
#[case::an_international_name(json!(["Bücher.de"]), "xn--bcher-kva.de", true)]
#[case::every_domain(json!(["*"]), "xn--bcher-kva.de", true)]
#[case::another_domain(json!(["news.example"]), "shop.example", false)]
fn consent_domains_cover_a_domain_however_it_is_written(
    #[case] domains: serde_json::Value,
    #[case] domain: &str,
    #[case] covered: bool,
) {
    assert_eq!(ConsentDomains::of(&domains).covers(domain), covered);
}

/// `wall.refetch_domains` lists a domain however a person wrote it, so
/// "Needs you" finds the entry to remove.
#[test]
fn a_refetch_domain_written_as_an_international_name_covers_its_key() {
    assert_eq!(
        coverage(&["Bücher.de".to_owned()], "xn--bcher-kva.de"),
        Some(false)
    );
}

/// A source the test fills: what it holds now, which a person may dismiss
/// when `dismissable`.
struct Held {
    items: watch::Receiver<Vec<NeedsYouItem>>,
    dismissable: bool,
}

impl Primitives for Held {
    fn dismissable(&self) -> bool {
        self.dismissable
    }

    fn items<'a>(
        &'a self,
        _: &'a SourceContext,
    ) -> Answering<'a, Result<Vec<NeedsYouItem>, SourceError>> {
        let items = self.items.borrow().clone();
        Box::pin(async move { Ok(items) })
    }

    fn changes(&self, _: &SourceContext) -> Changes {
        Changes::of(self.items.clone())
    }
}

/// A source holding `items`, and what changes them.
fn held(
    items: Vec<NeedsYouItem>,
    dismissable: bool,
) -> (NeedsYouSource, watch::Sender<Vec<NeedsYouItem>>) {
    let (changing, items) = watch::channel(items);
    (NeedsYouSource::new(Held { items, dismissable }), changing)
}

fn item(id: &str, kind: &str, since: &str) -> NeedsYouItem {
    NeedsYouItem {
        id: id.to_owned(),
        kind: kind.to_owned(),
        since: PointInTime::from_rfc3339(since.to_owned()),
        run: None,
        schedule: None,
        summary: format!("{id} waits"),
        actions: Vec::new(),
        details: Map::new(),
    }
}

/// One engine's "Needs you" on the Run index at `index`, its changes on a
/// subscription opened before it starts.
struct Started {
    needs_you: NeedsYou,
    changes: Subscription,
    owner: TaskOwner,
}

async fn started(index: &Path, sources: Vec<NeedsYouSource>) -> Started {
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let events = Events::start(
        EventsOptions::default(),
        EngineInstance::new("test"),
        &owner,
        goodfetch_counted_work::CountedWork::default(),
    );
    let changes = events.subscribe(None, false).await.unwrap();
    let context = SourceContext {
        clock: Clock::paused(UNIX_EPOCH + Duration::from_hours(500_000)),
        storage: watch::channel(Arc::new(StorageSample::default())).1,
        run_index: RunIndex::of(
            index.to_owned(),
            &ProductVersion::parse("1.0.0").unwrap(),
            None,
            Arc::new(owner.child()),
        ),
        ledger: Ledger::at(index.with_file_name("usage.sqlite"), None),
        settings: Settings::new(
            SettingsFiles::of(
                &Home::new(index.parent().unwrap().to_owned()),
                &ProductVersion::parse("1.0.0").unwrap(),
            ),
            Product::Collie,
            owner.child(),
        ),
        known_certificates: KnownCertificates::of(
            &Home::new(index.parent().unwrap().to_owned()),
            None,
            Arc::new(owner.child()),
        ),
        owner: Arc::new(owner.child()),
    };
    let needs_you = NeedsYou::start(&sources, context, events, &owner);
    Started {
        needs_you,
        changes,
        owner,
    }
}

impl Started {
    /// The next `count` changes of the list.
    async fn changes(&mut self, count: usize) -> Vec<NeedsYouChange> {
        let mut changes = Vec::new();
        while changes.len() < count {
            let taken: Vec<crate::events::Envelope> =
                self.changes.next(count - changes.len()).await.unwrap();
            changes.extend(taken.into_iter().map(|envelope| {
                assert_eq!(envelope.event, "needs_you.changed");
                serde_json::from_str::<NeedsYouChange>(envelope.data.json()).unwrap()
            }));
        }
        changes
    }

    async fn list(&self, params: NeedsYouListParams) -> NeedsYouList {
        self.needs_you.list(params).await.unwrap()
    }

    async fn ids(&self) -> Vec<String> {
        self.list(NeedsYouListParams::default())
            .await
            .items
            .into_iter()
            .map(|item| item.id)
            .collect()
    }

    async fn stop(self) {
        drop(self.needs_you);
        self.owner.stop().await;
    }
}

fn index() -> (tempfile::TempDir, PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("run-index.sqlite");
    (folder, path)
}

fn listed(change: &NeedsYouChange) -> &str {
    match change {
        NeedsYouChange::Listed { item } => &item.id,
        NeedsYouChange::Removed { .. } => panic!("expected an item listed, got {change:?}"),
    }
}

/// Every source's items make one list, the longest waiting first and by ID
/// among items waiting since the same time, which `kinds` narrows to the
/// kinds it names.
///
/// Covers: story:12-23
#[test]
fn every_sources_items_make_one_list_the_longest_waiting_first() {
    let (_folder, path) = index();
    goodfetch_test_support::runtime().block_on(async {
        let (first, _keep_first) = held(
            vec![
                item("b", "storage_level", "2026-10-24T10:00:00Z"),
                item("d", "storage_level", "2026-10-24T12:00:00Z"),
            ],
            false,
        );
        let (second, _keep_second) = held(
            vec![
                item("c", "run_stopped_by_storage", "2026-10-24T10:00:00Z"),
                item("a", "run_stopped_by_storage", "2026-10-24T11:00:00Z"),
            ],
            true,
        );
        let mut started = started(&path, vec![first, second]).await;
        started.changes(4).await;

        let every = started.ids().await;
        let stopped = started
            .list(NeedsYouListParams {
                kinds: Some(vec!["run_stopped_by_storage".to_owned()]),
                after: None,
            })
            .await;

        assert_eq!(
            (
                every,
                stopped
                    .items
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
            ),
            (
                vec![
                    "b".to_owned(),
                    "c".to_owned(),
                    "a".to_owned(),
                    "d".to_owned()
                ],
                vec!["c", "a"],
            )
        );
        started.stop().await;
    });
}

/// The list orders items by the time their `since` names, not by its text:
/// a source that writes fractions of a second or another offset lists its
/// items where their time falls, and the cursor a page ends at compares as
/// a time too.
///
/// Covers: story:12-23
#[test]
fn items_are_ordered_by_the_time_their_since_names_not_its_text() {
    let (_folder, path) = index();
    goodfetch_test_support::runtime().block_on(async {
        let (first, _keep_first) = held(
            vec![
                item("whole", "storage_level", "2026-10-24T10:00:00Z"),
                item("later_fraction", "storage_level", "2026-10-24T10:00:00.5Z"),
            ],
            false,
        );
        let (second, _keep_second) = held(
            vec![
                item(
                    "offset",
                    "run_stopped_by_storage",
                    "2026-10-24T11:30:00+02:00",
                ),
                item(
                    "a_moment_before",
                    "run_stopped_by_storage",
                    "2026-10-24T09:59:59.999Z",
                ),
            ],
            true,
        );
        let mut started = started(&path, vec![first, second]).await;
        started.changes(4).await;

        let every = started.ids().await;
        let after_whole = started
            .list(NeedsYouListParams {
                kinds: None,
                after: Some("2026-10-24T10:00:00Z whole".to_owned()),
            })
            .await;

        assert_eq!(
            (
                every,
                after_whole
                    .items
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
            ),
            (
                vec![
                    "offset".to_owned(),
                    "a_moment_before".to_owned(),
                    "whole".to_owned(),
                    "later_fraction".to_owned(),
                ],
                vec!["later_fraction"],
            )
        );
        started.stop().await;
    });
}

/// A list longer than a page is read a page at a time: each page's `next`
/// is where the next one starts, and the last page has none.
#[test]
fn a_long_list_is_read_a_page_at_a_time() {
    let (_folder, path) = index();
    goodfetch_test_support::runtime().block_on(async {
        let items = (0..=PAGE)
            .map(|count| {
                item(
                    &format!("item-{count:03}"),
                    "storage_level",
                    "2026-10-24T10:00:00Z",
                )
            })
            .collect();
        let (source, _keep) = held(items, false);
        let mut started = started(&path, vec![source]).await;
        started.changes(PAGE + 1).await;

        let first = started.list(NeedsYouListParams::default()).await;
        let second = started
            .list(NeedsYouListParams {
                kinds: None,
                after: first.next.clone(),
            })
            .await;

        assert_eq!(
            (
                first.items.len(),
                first.items.last().map(|item| item.id.clone()),
                second
                    .items
                    .iter()
                    .map(|item| item.id.clone())
                    .collect::<Vec<_>>(),
                second.next,
            ),
            (
                PAGE,
                Some(format!("item-{:03}", PAGE - 1)),
                vec![format!("item-{PAGE:03}")],
                None,
            )
        );
        started.stop().await;
    });
}

/// An item its source no longer holds, resolved by a person elsewhere or
/// by its Run, leaves the list with `needs_you.changed` saying so; one that
/// changes is listed again as it is now, waiting since it first appeared.
///
/// Covers: story:12-32
#[test]
fn an_item_its_source_resolves_leaves_the_list_and_one_that_changes_keeps_its_since() {
    let (_folder, path) = index();
    goodfetch_test_support::runtime().block_on(async {
        let low = item("disk", "storage_level", "2026-10-24T10:00:00Z");
        let (source, changing) = held(
            vec![
                low.clone(),
                item("run", "run_stopped_by_storage", "2026-10-24T09:00:00Z"),
            ],
            false,
        );
        let mut started = started(&path, vec![source]).await;
        started.changes(2).await;

        let critical = NeedsYouItem {
            since: PointInTime::from_rfc3339("2026-10-24T10:05:00Z".to_owned()),
            summary: "disk is critical".to_owned(),
            ..low.clone()
        };
        changing.send_replace(vec![
            critical,
            item("run", "run_stopped_by_storage", "2026-10-24T09:00:00Z"),
        ]);
        let changed = started.changes(1).await;
        changing.send_replace(vec![item(
            "run",
            "run_stopped_by_storage",
            "2026-10-24T09:00:00Z",
        )]);
        let resolved = started.changes(1).await;

        assert_eq!(
            (changed, resolved, started.ids().await),
            (
                vec![NeedsYouChange::Listed {
                    item: NeedsYouItem {
                        summary: "disk is critical".to_owned(),
                        ..low
                    }
                }],
                vec![NeedsYouChange::Removed {
                    id: "disk".to_owned(),
                    reason: NeedsYouRemoval::Resolved,
                }],
                vec!["run".to_owned()],
            )
        );
        started.stop().await;
    });
}

/// A person dismisses an item whose source allows it: it leaves the list
/// with `needs_you.changed`, offers `needs_you.dismiss` while listed, and
/// stays hidden once the engine starts again on the same Run index. An
/// item of a source that does not allow it, or one the list no longer
/// holds, is not dismissed.
///
/// Covers: story:12-32, format:run-index
#[test]
fn a_dismissed_item_stays_hidden_after_the_engine_starts_again() {
    let (_folder, path) = index();
    goodfetch_test_support::runtime().block_on(async {
        let stopped = item("stopped", "run_stopped_by_storage", "2026-10-24T09:00:00Z");
        let level = item("level", "storage_level", "2026-10-24T10:00:00Z");
        let (dismissable, _keep_dismissable) = held(vec![stopped.clone()], true);
        let (fixed, _keep_fixed) = held(vec![level.clone()], false);
        let mut first = started(&path, vec![dismissable, fixed]).await;
        first.changes(2).await;
        let offered: Vec<Vec<String>> = first
            .list(NeedsYouListParams::default())
            .await
            .items
            .into_iter()
            .map(|item| item.actions)
            .collect();

        let answers = (
            first.needs_you.dismiss("stopped".to_owned()).await.unwrap(),
            first.needs_you.dismiss("level".to_owned()).await.unwrap(),
            first.needs_you.dismiss("gone".to_owned()).await.unwrap(),
        );
        let dismissed = first.changes(1).await;
        first.stop().await;

        let (dismissable, _keep_dismissable) = held(vec![stopped], true);
        let (fixed, _keep_fixed) = held(vec![level], false);
        let mut again = started(&path, vec![dismissable, fixed]).await;
        let relisted = again.changes(1).await;

        assert_eq!(
            (
                offered,
                answers,
                dismissed,
                relisted.iter().map(listed).collect::<Vec<_>>(),
                again.ids().await,
            ),
            (
                vec![vec!["needs_you.dismiss".to_owned()], Vec::new()],
                (true, false, false),
                vec![NeedsYouChange::Removed {
                    id: "stopped".to_owned(),
                    reason: NeedsYouRemoval::Dismissed,
                }],
                vec!["level"],
                vec!["level".to_owned()],
            )
        );
        again.stop().await;
    });
}
