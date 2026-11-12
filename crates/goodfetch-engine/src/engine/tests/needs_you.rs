//! The storage entries of "Needs you" through the engine's public interface
//! over the in-memory network (spec #592's Seam 3; ADR 0122; pitfall O03),
//! on the paused `Clock`, the governor reading the filesystems through a
//! view the test sets: a filesystem at `low`, then `critical`, is listed
//! with its folder, the Run the critical level stops is listed beside it,
//! and each leaves the list with `needs_you.changed` once it is resolved:
//! the level once space is freed, the Run once it runs again or a person
//! dismisses it. A subscription that cannot resume gets every item in its
//! `events.resync`. A Run a Limit stopped is listed, offering a person's
//! `usage.raise` (and `run.resume` for a Recipe's Run), until a person
//! raises the Limit from the Kennel (ADRs 0031 and 0041).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::disk_view::DiskView;
use super::events::credit;
use super::run::{call, run_folder};
use super::storage::configure;
use super::usage::{charging, install_month, monthly_install_limit};
use super::{Client, attached_on, engine, engine_with, hello};
use crate::engine::Engine;

const MB: u64 = 1_000_000;

/// The levels the tests set: `low` below 2 MB, `critical` below 1 MB.
const LEVELS: &str = "storage.low_free = 2000000\nstorage.critical_free = 1000000\n";

/// The call that holds the tests' subscription.
pub(super) const SUBSCRIPTION: i64 = 1;

fn viewed(view: &DiskView) -> Engine {
    engine(Product::Collie).with_governor(view.system())
}

/// Opens the Connection and subscribes to every event; answers the engine
/// instance its `hello` named.
async fn opened(client: &mut Client) -> Value {
    client.send(&[hello("collie", "0.1")]).await;
    let opened = client.next().await.unwrap();
    client
        .send(&[
            json!({ "jsonrpc": "2.0", "id": SUBSCRIPTION, "method": "events.subscribe", "params": {} }),
            credit(SUBSCRIPTION, 10_000),
        ])
        .await;
    opened["result"]["engine_instance"].clone()
}

/// "Needs you" as its changes on the subscription leave it: each item by
/// ID, with every change seen so far.
#[derive(Default)]
pub(super) struct Seen {
    pub(super) items: BTreeMap<String, Value>,
    pub(super) changes: Vec<Value>,
}

impl Seen {
    /// Reads the subscription's events until `done` holds of the items,
    /// failing the test when a minute of the paused clock passes first.
    pub(super) async fn until(
        &mut self,
        client: &mut Client,
        done: impl Fn(&BTreeMap<String, Value>) -> bool,
    ) {
        while !done(&self.items) {
            let message = tokio::time::timeout(Duration::from_secs(60), client.next())
                .await
                .unwrap_or_else(|_| panic!("\"Needs you\" never got there: {:?}", self.items))
                .expect("the engine closed");
            self.take(&message);
        }
    }

    /// Calls `method` with `params` as call `id`, taking the subscription's
    /// events that arrive meanwhile; answers the reply.
    pub(super) async fn call(
        &mut self,
        client: &mut Client,
        id: i64,
        method: &str,
        params: Value,
    ) -> Value {
        client
            .send(&[json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })])
            .await;
        loop {
            let message = client.next().await.expect("the engine closed");
            if message["id"] == json!(id) {
                return message;
            }
            self.take(&message);
        }
    }

    /// Takes the changes `message` carries, if it is the subscription's.
    fn take(&mut self, message: &Value) {
        if message["method"] != "$/events" || message["params"]["id"] != json!(SUBSCRIPTION) {
            return;
        }
        {
            for event in message["params"]["events"].as_array().unwrap() {
                if event["event"] != "needs_you.changed" {
                    continue;
                }
                let change = &event["data"];
                match change["change"].as_str() {
                    Some("listed") => {
                        let item = change["item"].clone();
                        self.items
                            .insert(item["id"].as_str().unwrap().to_owned(), item);
                    }
                    _ => {
                        self.items.remove(change["id"].as_str().unwrap());
                    }
                }
                self.changes.push(change.clone());
            }
        }
    }
}

/// Each item's kind with what tells it apart: its level and folder for a
/// filesystem, its Run, folder and actions for a stopped Run.
fn shown(items: &[Value]) -> Vec<Value> {
    items
        .iter()
        .map(|item| {
            json!({
                "kind": item["kind"],
                "run": item["run"],
                "details": item["details"],
                "actions": item["actions"],
            })
        })
        .collect()
}

fn of_kind<'a>(items: &'a BTreeMap<String, Value>, kind: &str) -> Vec<&'a Value> {
    items.values().filter(|item| item["kind"] == kind).collect()
}

pub(super) fn listed(result: &Value) -> Vec<Value> {
    result["result"]["items"].as_array().unwrap().clone()
}

/// A filesystem crossing `storage.low_free`, then `storage.critical_free`,
/// is listed with its folder at each level, the Run the critical level
/// stops is listed with the folder that filled, and a subscription that
/// cannot resume gets both in its resync. Freeing space takes the level
/// off the list, and the Run's resume takes it off too, each with a change
/// that says it is resolved.
///
/// Covers: story:12-25, story:12-32, operation:needs_you.list, event:needs_you.changed, event:events.resync
#[test]
fn the_storage_entries_appear_with_their_folder_and_leave_once_resolved() {
    let env = TestEnv::new();
    configure(&env, LEVELS);
    let view = DiskView::default();
    let disk = view.mount(env.home(), 100 * MB, 50 * MB);
    let filling = view.clone();

    let (at_low, at_critical, resynced, freed, resumed, changes) = attached_on(
        &viewed(&view),
        &env,
        |mut client| async move {
            let instance = opened(&mut client).await;
            call(&mut client, 2, "run.start", json!({ "run_id": "filled" })).await;
            let mut seen = Seen::default();

            filling.set_free(disk, MB * 3 / 2);
            seen.until(&mut client, |items| {
                of_kind(items, "storage_level")
                    .iter()
                    .any(|item| item["details"]["level"] == "low")
            })
            .await;
            let at_low = seen.call(&mut client, 3, "needs_you.list", json!({})).await;

            filling.set_free(disk, MB / 2);
            seen.until(&mut client, |items| {
                !of_kind(items, "run_stopped_by_storage").is_empty()
                    && of_kind(items, "storage_level")
                        .iter()
                        .any(|item| item["details"]["level"] == "critical")
            })
            .await;
            let at_critical = seen.call(&mut client, 4, "needs_you.list", json!({})).await;
            client
                .send(&[
                    json!({ "jsonrpc": "2.0", "id": 5, "method": "events.subscribe", "params": { "after": { "engine_instance": instance, "position": 1_000_000 } } }),
                    credit(5, 1),
                ])
                .await;
            let resynced = loop {
                let message = client.next().await.unwrap();
                if message["method"] == "$/events" && message["params"]["id"] == json!(5) {
                    break message["params"]["events"][0].clone();
                }
            };

            filling.set_free(disk, 50 * MB);
            seen.until(&mut client, |items| {
                of_kind(items, "storage_level").is_empty()
            })
            .await;
            let freed = seen.call(&mut client, 6, "needs_you.list", json!({})).await;

            seen.call(&mut client, 7, "run.resume", json!({ "run_id": "filled" }))
                .await;
            seen.until(&mut client, BTreeMap::is_empty).await;
            let resumed = seen.call(&mut client, 8, "needs_you.list", json!({})).await;
            (at_low, at_critical, resynced, freed, resumed, seen.changes)
        },
    );

    let home = env.home().display().to_string();
    let folder = run_folder(&env, "filled").display().to_string();
    let level = |level: &str| json!({ "kind": "storage_level", "run": null, "details": { "level": level, "folder": home }, "actions": [] });
    let stopped = json!({
        "kind": "run_stopped_by_storage",
        "run": "filled",
        "details": { "folder": folder },
        "actions": ["needs_you.dismiss"],
    });
    let mut critical = shown(&listed(&at_critical));
    critical.sort_by_key(|item| item["kind"].to_string());
    let removed: Vec<(Value, Value)> = changes
        .iter()
        .filter(|change| change["change"] == "removed")
        .map(|change| (change["id"].clone(), change["reason"].clone()))
        .collect();
    assert_eq!(
        (
            shown(&listed(&at_low)),
            critical,
            resynced["event"].clone(),
            resynced["data"]["needs_you"]
                .as_array()
                .map(|items| shown(items)),
            shown(&listed(&freed)),
            listed(&resumed),
            removed
                .iter()
                .map(|(_, reason)| reason.clone())
                .collect::<Vec<_>>(),
        ),
        (
            vec![level("low")],
            vec![stopped.clone(), level("critical")],
            json!("events.resync"),
            Some(shown(&listed(&at_critical))),
            vec![stopped],
            Vec::<Value>::new(),
            vec![json!("resolved"), json!("resolved")],
        ),
        "{changes:?}"
    );
}

/// A person dismisses the Run stopped by storage: it leaves the list, with
/// a change that says so, while the filesystem's level, which only freeing
/// space resolves, is not dismissed.
///
/// Covers: story:12-32, operation:needs_you.dismiss
#[test]
fn a_person_dismisses_a_stopped_run_but_not_a_storage_level() {
    let env = TestEnv::new();
    configure(&env, LEVELS);
    let view = DiskView::default();
    let disk = view.mount(env.home(), 100 * MB, 50 * MB);
    let filling = view.clone();

    let (answers, after, change) = attached_on(&viewed(&view), &env, |mut client| async move {
        opened(&mut client).await;
        call(&mut client, 2, "run.start", json!({ "run_id": "filled" })).await;
        let mut seen = Seen::default();
        filling.set_free(disk, MB / 2);
        seen.until(&mut client, |items| {
            !of_kind(items, "run_stopped_by_storage").is_empty()
                && !of_kind(items, "storage_level").is_empty()
        })
        .await;
        let stopped = of_kind(&seen.items, "run_stopped_by_storage")[0]["id"].clone();
        let level = of_kind(&seen.items, "storage_level")[0]["id"].clone();
        let dismissed = seen
            .call(
                &mut client,
                3,
                "needs_you.dismiss",
                json!({ "item": stopped }),
            )
            .await;
        let kept = seen
            .call(
                &mut client,
                4,
                "needs_you.dismiss",
                json!({ "item": level }),
            )
            .await;
        seen.until(&mut client, |items| {
            of_kind(items, "run_stopped_by_storage").is_empty()
        })
        .await;
        let after = seen
            .call(
                &mut client,
                5,
                "needs_you.list",
                json!({ "kinds": ["run_stopped_by_storage", "storage_level"] }),
            )
            .await;
        (
            (dismissed["result"].clone(), kept["result"].clone()),
            after,
            seen.changes.last().cloned(),
        )
    });

    assert_eq!(
        (
            answers,
            listed(&after)
                .iter()
                .map(|item| item["kind"].clone())
                .collect::<Vec<_>>(),
            change.map(|change| change["reason"].clone()),
        ),
        (
            (json!({ "dismissed": true }), json!({ "dismissed": false })),
            vec![json!("storage_level")],
            Some(json!("dismissed")),
        )
    );
}

/// A Run that `test.charge` stops on the install's monthly Limit of 2
/// requests, tagged with `tags`, on a disk with room: "Needs you" as it
/// lists the Run, then as a person's `usage.raise` from the Kennel leaves
/// it, with the reasons of the items removed meanwhile.
fn stopped_by_limit_then_raised(tags: Value) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let env = TestEnv::new();
    let view = DiskView::default();
    view.mount(env.home(), 1_000_000 * MB, 500_000 * MB);
    let engine = engine_with(Product::Collie, Some(charging())).with_governor(view.system());

    attached_on(&engine, &env, |mut client| async move {
        let mut opening = hello("collie", "0.1");
        opening["params"]["surface"] = json!("kennel");
        client.send(&[opening]).await;
        client.next().await.unwrap();
        client
            .send(&[
                json!({ "jsonrpc": "2.0", "id": SUBSCRIPTION, "method": "events.subscribe", "params": {} }),
                credit(SUBSCRIPTION, 10_000),
            ])
            .await;
        let mut seen = Seen::default();
        seen.call(&mut client, 2, "usage.set", monthly_install_limit(2))
            .await;
        seen.call(
            &mut client,
            3,
            "test.charge",
            json!({ "run_id": "capped", "amount": 3, "tags": tags }),
        )
        .await;
        seen.call(&mut client, 4, "run.finish", json!({ "run_id": "capped" }))
            .await;
        seen.until(&mut client, |items| {
            !of_kind(items, "run_stopped_by_limit").is_empty()
        })
        .await;
        let stopped = seen.call(&mut client, 5, "needs_you.list", json!({})).await;

        seen.call(&mut client, 6, "usage.raise", monthly_install_limit(10))
            .await;
        seen.until(&mut client, |items| {
            of_kind(items, "run_stopped_by_limit").is_empty()
        })
        .await;
        let raised = seen.call(&mut client, 7, "needs_you.list", json!({})).await;
        let removed = seen
            .changes
            .iter()
            .filter(|change| change["change"] == "removed")
            .map(|change| change["reason"].clone())
            .collect();
        (shown(&listed(&stopped)), listed(&raised), removed)
    })
}

/// An SDK script's Run stopped by a Limit is listed with the Limit as
/// `limit_reached` names it, what it allows now and the Product the Run
/// needs, offering the person's `usage.raise`; once a person raises the
/// Limit it leaves the list, resolved, to wait for its script.
///
/// Covers: story:12-100, story:12-32, operation:usage.raise
#[test]
fn a_run_stopped_by_a_limit_is_listed_until_a_person_raises_the_limit() {
    let listed = stopped_by_limit_then_raised(json!([]));

    assert_eq!(
        listed,
        (
            vec![json!({
                "kind": "run_stopped_by_limit",
                "run": "capped",
                "details": { "limit": install_month(), "amount": 2, "needs": "collie" },
                "actions": ["usage.raise", "needs_you.dismiss"],
            })],
            Vec::new(),
            vec![json!("resolved")],
        )
    );
}

/// A Recipe's Run stopped by a Limit names its Recipe and also offers
/// `run.resume`, which resumes it in the Daemon once the Limit is raised.
///
/// Covers: story:12-100
#[test]
fn a_recipes_run_stopped_by_a_limit_offers_its_resume_in_the_daemon() {
    let listed = stopped_by_limit_then_raised(json!(["goodfetch.recipe:nightly-prices"]));

    assert_eq!(
        listed.0,
        vec![json!({
            "kind": "run_stopped_by_limit",
            "run": "capped",
            "details": {
                "limit": install_month(),
                "amount": 2,
                "needs": "collie",
                "recipe": "nightly-prices",
            },
            "actions": ["usage.raise", "run.resume", "needs_you.dismiss"],
        })]
    );
}
