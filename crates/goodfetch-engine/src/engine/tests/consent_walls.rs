//! The consent walls of "Needs you" through the engine's public interface
//! over the in-memory network (spec #592's Seam 3; ADRs 0048 and 0052), on
//! the paused `Clock`: each domain whose consent wall a Run met, as the
//! page crate records it through the Run (here the test's `test.consent_wall`
//! call), is listed with the offer to add it to `consent.domains` for the
//! next runs, a domain in `wall.denied_domains` too, one already in
//! `consent.domains` not. A person's answer, `needs_you.allow_consent_domain`
//! from the Kennel, writes the domain, or `*`, in user configuration in one
//! call, records it in the Audit log with the warning the person was shown,
//! and the item leaves the list, resolved.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the Goodfetch home's settings files and read its Audit log themselves"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::home::{OPERATOR_POLICY_FILE, USER_CONFIGURATION_FILE};
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use super::disk_view::DiskView;
use super::events::credit;
use super::needs_you::{SUBSCRIPTION, Seen, listed};
use super::run::call;
use super::{Client, attached_on, engine_with, per_major};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::engine::Engine;
use crate::home::Home;

const MB: u64 = 1_000_000;

/// The two domains whose consent walls the tests' Run meets.
const WALLED: [&str; 2] = ["walled.test", "other.test"];

/// Records that a Run met a consent wall on a domain, as the page crate
/// does when its Consent prompt runner finds one.
const CONSENT_WALL: Operation = Operation {
    name: "test.consent_wall",
    description: "Records that a Run met a consent wall on a domain.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

fn walls() -> Contribution {
    Contribution::new("test-consent-walls").with_handler(Handler::new(
        &CONSENT_WALL,
        |params: Value, context: CallContext| async move {
            let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
            let run = context.runs().live(run_id).await.unwrap();
            run.record_consent_wall(params["domain"].as_str().unwrap().to_owned())
                .await
                .unwrap();
            Ok::<Value, CallError>(json!({}))
        },
    ))
}

/// A Collie engine with the test's call, whose governor samples the test
/// home's filesystem, which "Needs you" reads again at each sample.
fn walled_engine(env: &TestEnv) -> Engine {
    let view = DiskView::default();
    view.mount(env.home(), 100 * MB, 50 * MB);
    engine_with(Product::Collie, Some(walls())).with_governor(view.system())
}

/// Writes `text` as `file` of the test home's per-major folder.
pub(super) fn write(env: &TestEnv, file: &str, text: &str) {
    let folder = per_major(env);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(file), text).unwrap();
}

/// Writes user configuration with a short sample interval and `settings`.
pub(super) fn configure(env: &TestEnv, settings: &str) {
    write(
        env,
        USER_CONFIGURATION_FILE,
        &format!("version = \"0.1\"\n\n[settings]\ngovernor.sample_interval = 250\n{settings}"),
    );
}

/// Opens a Connection from `surface` and subscribes to every event.
pub(super) async fn opened(client: &mut Client, surface: &str) {
    client
        .send(&[json!({
            "jsonrpc": "2.0", "id": 0, "method": "hello",
            "params": { "protocol_version": "0.1", "surface": surface, "needs": "collie", "product_version": "1.4.0" },
        })])
        .await;
    client.next().await.unwrap();
    client
        .send(&[
            json!({ "jsonrpc": "2.0", "id": SUBSCRIPTION, "method": "events.subscribe", "params": {} }),
            credit(SUBSCRIPTION, 10_000),
        ])
        .await;
}

/// Starts the Run `walled`, which meets a consent wall on each of `WALLED`.
async fn met_walls(client: &mut Client, seen: &mut Seen) {
    seen.call(client, 2, "run.start", json!({ "run_id": "walled" }))
        .await;
    for (id, domain) in (3..).zip(WALLED) {
        seen.call(
            client,
            id,
            "test.consent_wall",
            json!({ "run_id": "walled", "domain": domain }),
        )
        .await;
    }
}

/// Each consent wall item's domain, Run and actions.
fn walls_shown(items: &[Value]) -> Vec<Value> {
    items
        .iter()
        .filter(|item| item["kind"] == "consent_wall")
        .map(|item| {
            json!({
                "domain": item["details"]["domain"],
                "run": item["run"],
                "actions": item["actions"],
            })
        })
        .collect()
}

fn offered(domain: &str) -> Value {
    json!({
        "domain": domain,
        "run": "walled",
        "actions": ["needs_you.allow_consent_domain", "needs_you.dismiss"],
    })
}

/// The Audit log's entries, in order.
pub(super) fn audit_entries(env: &TestEnv) -> Vec<Value> {
    let home = Home::new(env.home().to_path_buf());
    if !home.audit_log().exists() {
        return Vec::new();
    }
    let connection = Connection::open(home.audit_log()).unwrap();
    let mut statement = connection
        .prepare("SELECT entry FROM entries ORDER BY sequence")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|entry| serde_json::from_str(&entry.unwrap()).unwrap())
        .collect()
}

/// Makes the Audit log one no engine can write: a folder stands where its
/// store goes.
pub(super) fn unrecordable(home: &std::path::Path) {
    let store = Home::new(home.to_path_buf()).audit_log();
    for path in [
        store.clone(),
        store.with_extension("sqlite-wal"),
        store.with_extension("sqlite-shm"),
    ] {
        if path.is_file() {
            std::fs::remove_file(&path).unwrap();
        }
    }
    std::fs::create_dir_all(&store).unwrap();
}

/// Each domain whose consent wall a Run met is offered for `consent.domains`
/// unless that already covers it, a domain in `wall.denied_domains` too: a
/// denial takes away only the Wall allowance (ADR 0052).
///
/// Covers: story:12-26, operation:needs_you.list, setting:consent.domains, setting:wall.denied_domains
#[rstest]
#[case::each_domain_that_showed_one("", "", &["other.test", "walled.test"])]
#[case::a_domain_denied_its_walls_still(
    "",
    "version = \"0.1\"\n\n[settings]\nwall.denied_domains = [\"walled.test\"]\n",
    &["other.test", "walled.test"],
)]
#[case::not_one_consent_domains_lists("consent.domains = [\"walled.test\"]\n", "", &["other.test"])]
#[case::none_under_an_explicit_star("consent.domains = [\"*\"]\n", "", &[])]
fn a_domain_that_showed_a_consent_wall_is_offered_for_consent_domains(
    #[case] configuration: &str,
    #[case] policy: &str,
    #[case] expected: &[&str],
) {
    let env = TestEnv::new();
    configure(&env, configuration);
    if !policy.is_empty() {
        write(&env, OPERATOR_POLICY_FILE, policy);
    }

    let items = attached_on(&walled_engine(&env), &env, |mut client| async move {
        opened(&mut client, "kennel").await;
        let mut seen = Seen::default();
        met_walls(&mut client, &mut seen).await;
        // Two samples later, every source has read the walls.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let answer = seen
            .call(&mut client, 10, "needs_you.list", json!({}))
            .await;
        listed(&answer)
    });

    assert_eq!(
        walls_shown(&items),
        expected
            .iter()
            .map(|domain| offered(domain))
            .collect::<Vec<_>>()
    );
}

/// Meets the walls, then answers `walled.test`, again, then `*`; answers
/// the three answers, `config.explain` of `consent.domains`, the list left
/// and each removal with its reason.
async fn answered(mut client: Client) -> ([Value; 3], Value, Vec<Value>, Vec<Value>) {
    opened(&mut client, "kennel").await;
    let mut seen = Seen::default();
    met_walls(&mut client, &mut seen).await;
    seen.until(&mut client, |items| items.len() == 2).await;
    let allow = |domain: &str| json!({ "domain": domain });
    let first = seen
        .call(
            &mut client,
            10,
            "needs_you.allow_consent_domain",
            allow("walled.test"),
        )
        .await;
    seen.until(&mut client, |items| items.len() == 1).await;
    let again = seen
        .call(
            &mut client,
            11,
            "needs_you.allow_consent_domain",
            allow("walled.test"),
        )
        .await;
    let star = seen
        .call(
            &mut client,
            12,
            "needs_you.allow_consent_domain",
            allow("*"),
        )
        .await;
    seen.until(&mut client, std::collections::BTreeMap::is_empty)
        .await;
    let explained = call(
        &mut client,
        13,
        "config.explain",
        json!({ "keys": ["consent.domains"] }),
    )
    .await;
    let after = seen
        .call(&mut client, 14, "needs_you.list", json!({}))
        .await;
    let removed: Vec<Value> = seen
        .changes
        .iter()
        .filter(|change| change["change"] == "removed")
        .map(|change| json!([change["id"], change["reason"]]))
        .collect();
    (
        [first, again, star].map(|answer| answer["result"].clone()),
        explained,
        listed(&after),
        removed,
    )
}

/// A person's answer from the Kennel writes the domain in user
/// configuration's `consent.domains` in one call, with no confirmation
/// asked, records it in the Audit log with the warning shown, and the item
/// leaves the list, resolved; the same answer again adds nothing and
/// records nothing. Writing `*` warns that it accepts every domain's wall
/// and that an Agent no longer asks first, and takes every other wall off.
/// The Kennel is a person surface (ADR 0141): its answer is admitted, as the
/// CLI's is, and the Audit log records it as the Kennel's.
///
/// Covers: story:12-26, operation:needs_you.allow_consent_domain, event:needs_you.changed, operation:config.explain
#[test]
fn a_persons_answer_writes_consent_domains_records_its_warning_and_resolves_the_item() {
    let env = TestEnv::new();
    configure(&env, "");

    let (answers, explained, after, removed) = attached_on(&walled_engine(&env), &env, answered);

    let [first, again, star] = answers;
    let warning = first["warning"].as_str().unwrap_or_default().to_owned();
    let star_warning = star["warning"].as_str().unwrap_or_default().to_owned();
    let entries = audit_entries(&env);
    let effective = explained["result"]["settings"]
        .as_array()
        .and_then(|settings| {
            settings
                .iter()
                .find(|setting| setting["key"] == "consent.domains")
        })
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(
        (
            (&first["domain"], &first["added"], &first["layer"]),
            (&again["added"], &star["added"]),
            (
                warning.contains("walled.test") && warning.contains("in your name"),
                star_warning.contains("every domain") && star_warning.contains("Agent"),
            ),
            (&effective["value"], &effective["layer"]),
            entries
                .iter()
                .map(|entry| json!({
                    "kind": entry["kind"],
                    "actor": entry["actor"],
                    "surface": entry["surface"],
                    "layer": entry["layer"],
                    "details": entry["details"],
                }))
                .collect::<Vec<_>>(),
            after.len(),
            removed,
        ),
        (
            (
                &json!("walled.test"),
                &json!(true),
                &json!("user_configuration")
            ),
            (&json!(false), &json!(true)),
            (true, true),
            (&json!(["walled.test", "*"]), &json!("user_configuration")),
            vec![
                json!({
                    "kind": "consent_domain_allowed",
                    "actor": { "kind": "person", "name": "os_user" },
                    "surface": "kennel",
                    "layer": "user_configuration",
                    "details": { "domain": "walled.test", "warning": warning },
                }),
                json!({
                    "kind": "consent_domain_allowed",
                    "actor": { "kind": "person", "name": "os_user" },
                    "surface": "kennel",
                    "layer": "user_configuration",
                    "details": { "domain": "*", "warning": star_warning },
                }),
            ],
            0,
            vec![
                json!(["consent_wall:walled.test", "resolved"]),
                json!(["consent_wall:other.test", "resolved"]),
            ],
        )
    );
}

/// The answer is a person's: an agent surface is refused with
/// `person_required`, text naming no domain with `invalid_params`, and an
/// Operator policy that locks `consent.domains` with `setting_locked`,
/// user configuration left as it was each time and nothing recorded.
///
/// Covers: story:12-26, operation:needs_you.allow_consent_domain
#[rstest]
/// Covers: error:person_required
#[case::from_an_agent_surface("sdk_typescript", "walled.test", "", "person_required")]
#[case::naming_no_domain("kennel", "https://walled.test/", "", "invalid_params")]
/// Covers: error:setting_locked
#[case::under_a_lock(
    "kennel",
    "walled.test",
    "version = \"0.1\"\n\n[lock.consent]\ndomains = []\n",
    "setting_locked"
)]
fn an_answer_no_person_may_give_is_refused_and_writes_nothing(
    #[case] surface: &str,
    #[case] domain: &str,
    #[case] policy: &str,
    #[case] code: &str,
) {
    let env = TestEnv::new();
    configure(&env, "");
    if !policy.is_empty() {
        write(&env, OPERATOR_POLICY_FILE, policy);
    }
    let file = per_major(&env).join(USER_CONFIGURATION_FILE);
    let before = std::fs::read_to_string(&file).unwrap();
    let surface = surface.to_owned();
    let domain = domain.to_owned();

    let answer = attached_on(&walled_engine(&env), &env, |mut client| async move {
        opened(&mut client, &surface).await;
        call(
            &mut client,
            2,
            "needs_you.allow_consent_domain",
            json!({ "domain": domain }),
        )
        .await
    });

    let refused = answer["error"]["data"]["code"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            (answer["error"]["code"] == json!(-32602)).then(|| "invalid_params".to_owned())
        });
    assert_eq!(
        (
            refused.as_deref(),
            std::fs::read_to_string(&file).unwrap() == before,
            audit_entries(&env).len(),
        ),
        (Some(code), true, 0),
        "{answer}"
    );
}

/// An answer the Audit log cannot record is written all the same, since
/// the person gave it, and answers `internal_error` saying the entry is
/// missing rather than succeeding unrecorded.
///
/// Covers: operation:needs_you.allow_consent_domain
#[test]
fn an_answer_the_audit_log_cannot_record_is_written_and_says_so() {
    let env = TestEnv::new();
    configure(&env, "");
    let file = per_major(&env).join(USER_CONFIGURATION_FILE);
    let home = env.home().to_path_buf();

    let answer = attached_on(&walled_engine(&env), &env, |mut client| async move {
        opened(&mut client, "kennel").await;
        unrecordable(&home);
        call(
            &mut client,
            2,
            "needs_you.allow_consent_domain",
            json!({ "domain": "walled.test" }),
        )
        .await
    });

    assert_eq!(
        (
            &answer["error"]["code"],
            std::fs::read_to_string(&file)
                .unwrap()
                .contains("[settings.consent]\ndomains = [\"walled.test\"]"),
        ),
        (&json!(-32603), true),
        "{answer}"
    );
}
