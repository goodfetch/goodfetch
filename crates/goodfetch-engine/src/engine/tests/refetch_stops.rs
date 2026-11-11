//! The stops of the refetch without the site's stored data in "Needs you",
//! through the engine's public interface over the in-memory network (spec
//! #592's Seam 3; ADR 0052), on the paused `Clock`: each domain on which a
//! Run's refetch stopped, as the Wall allowance records it through the Run
//! (here the test's `test.refetch_stopped` call), is listed with the layer
//! of `wall.refetch_domains` that lists it, until no layer lists it any
//! more or `wall.denied_domains` denies it. A person dismisses the item,
//! removes the domain from that layer (or, when only `*` covers it,
//! replaces `*` with a list), or denies the domain with a reason; a domain
//! listed in project configuration is shown with its file, never removed
//! from the Kennel.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the Goodfetch home's settings files themselves"
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
use serde_json::{Value, json};

use super::consent_walls::{audit_entries, configure, opened, unrecordable, write};
use super::disk_view::DiskView;
use super::needs_you::{Seen, listed};
use super::run::call;
use super::{Client, attached_on, engine_with, per_major};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::engine::Engine;

const MB: u64 = 1_000_000;

/// The domain whose refetch stops.
const STOPPED: &str = "walled.test";

/// Records that a Run's refetch stopped on a domain, as the Wall allowance
/// does when the site refuses it.
const REFETCH_STOPPED: Operation = Operation {
    name: "test.refetch_stopped",
    description: "Records that a Run's refetch stopped on a domain.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

fn stops() -> Contribution {
    Contribution::new("test-refetch-stops").with_handler(Handler::new(
        &REFETCH_STOPPED,
        |params: Value, context: CallContext| async move {
            let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
            let run = context.runs().live(run_id).await.unwrap();
            run.record_refetch_stop(
                params["domain"].as_str().unwrap().to_owned(),
                params["cause"].as_str().unwrap().to_owned(),
            )
            .await
            .unwrap();
            Ok::<Value, CallError>(json!({}))
        },
    ))
}

/// A Collie engine with the test's call, whose governor samples the test
/// home's filesystem, which "Needs you" reads again at each sample.
fn stopping_engine(env: &TestEnv) -> Engine {
    let view = DiskView::default();
    view.mount(env.home(), 100 * MB, 50 * MB);
    engine_with(Product::Collie, Some(stops())).with_governor(view.system())
}

/// The machine's settings files, by name in the per-major folder.
fn file(env: &TestEnv, name: &str) -> String {
    per_major(env).join(name).display().to_string()
}

/// Starts the Run `stopped`, with `project` as its project configuration
/// when given, whose refetch stops on `STOPPED` at a Challenge.
async fn stopped(client: &mut Client, seen: &mut Seen, project: Option<Value>) {
    let mut start = json!({ "run_id": "stopped" });
    if let Some(project) = project {
        start["project"] = project;
    }
    seen.call(client, 2, "run.start", start).await;
    seen.call(
        client,
        3,
        "test.refetch_stopped",
        json!({ "run_id": "stopped", "domain": STOPPED, "cause": "challenge" }),
    )
    .await;
}

/// Each refetch stop item's ID, Run, details and actions.
fn stops_shown(items: &[Value]) -> Vec<Value> {
    items
        .iter()
        .filter(|item| item["kind"] == "wall_refetch_stopped")
        .map(|item| {
            json!({
                "id": item["id"],
                "run": item["run"],
                "details": item["details"],
                "actions": item["actions"],
            })
        })
        .collect()
}

/// A project configuration file listing `STOPPED` in
/// `wall.refetch_domains`, written in the test home, as a Client sends it.
fn project(env: &TestEnv) -> Value {
    let path = env.home().join("project").join("goodfetch.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "[settings]\nwall.domains = [\"walled.test\"]\nwall.refetch_domains = [\"walled.test\"]\n",
    )
    .unwrap();
    json!({
        "path": path.display().to_string(),
        "settings": { "wall": { "domains": [STOPPED], "refetch_domains": [STOPPED] } },
    })
}

/// The details and actions of the item a stop lists.
fn item(details: &Value, actions: &[&str]) -> Value {
    json!({
        "id": "wall_refetch_stopped:walled.test",
        "run": "stopped",
        "details": details,
        "actions": actions,
    })
}

const ANSWERS: [&str; 3] = [
    "needs_you.remove_refetch_domain",
    "needs_you.deny_wall_domain",
    "needs_you.dismiss",
];

/// A stop is listed with the layer of `wall.refetch_domains` that lists its
/// domain: the item offers to remove it there, or, when only `*` covers
/// it, says so; a domain from project configuration is shown with its file
/// and offers no removal. A domain no layer lists any more, or one
/// `wall.denied_domains` denies, is not listed.
///
/// Covers: story:12-27, operation:needs_you.list, setting:wall.refetch_domains
#[rstest]
#[case::listed_in_user_configuration(
    "wall.refetch_domains = [\"other.test\", \"walled.test\"]\n",
    "",
    false,
    Some(("user_configuration", USER_CONFIGURATION_FILE, false)),
)]
#[case::listed_in_the_operator_policy(
    "",
    "version = \"0.1\"\n\n[settings]\nwall.refetch_domains = [\"walled.test\"]\n",
    false,
    Some(("operator_policy", OPERATOR_POLICY_FILE, false)),
)]
#[case::covered_only_by_a_star("wall.refetch_domains = [\"*\"]\n", "", false, Some(("user_configuration", USER_CONFIGURATION_FILE, true)))]
#[case::listed_in_project_configuration("", "", true, Some(("project_configuration", "", false)))]
#[case::listed_nowhere_any_more("wall.refetch_domains = [\"other.test\"]\n", "", false, None)]
/// Covers: setting:wall.denied_domains
#[case::denied(
    "wall.refetch_domains = [\"walled.test\"]\n",
    "version = \"0.1\"\n\n[settings]\nwall.denied_domains = [\"walled.test\"]\n",
    false,
    None
)]
fn a_stopped_refetch_is_listed_with_the_layer_that_lists_its_domain(
    #[case] configuration: &str,
    #[case] policy: &str,
    #[case] in_project: bool,
    #[case] expected: Option<(&str, &str, bool)>,
) {
    let env = TestEnv::new();
    configure(&env, configuration);
    if !policy.is_empty() {
        write(&env, OPERATOR_POLICY_FILE, policy);
    }
    let project = in_project.then(|| project(&env));
    let project_file = project
        .as_ref()
        .map(|project| project["path"].as_str().unwrap().to_owned());

    let items = attached_on(&stopping_engine(&env), &env, |mut client| async move {
        opened(&mut client, "kennel").await;
        let mut seen = Seen::default();
        stopped(&mut client, &mut seen, project).await;
        // Two samples later, every source has read the stop.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let answer = seen
            .call(&mut client, 10, "needs_you.list", json!({}))
            .await;
        listed(&answer)
    });

    let expected: Vec<Value> = expected
        .into_iter()
        .map(|(layer, name, star)| {
            let path = project_file.clone().unwrap_or_else(|| file(&env, name));
            let actions: &[&str] = if layer == "project_configuration" {
                &ANSWERS[1..]
            } else {
                &ANSWERS
            };
            item(
                &json!({
                    "domain": STOPPED,
                    "cause": "challenge",
                    "layer": layer,
                    "file": path,
                    "only_star": star,
                }),
                actions,
            )
        })
        .collect();
    assert_eq!(stops_shown(&items), expected);
}

/// A person dismisses the item: it leaves the list, dismissed, and stays
/// hidden while the stop is listed.
///
/// Covers: story:12-27, operation:needs_you.dismiss, event:needs_you.changed
#[test]
fn a_dismissed_stop_leaves_the_list() {
    let env = TestEnv::new();
    configure(&env, "wall.refetch_domains = [\"walled.test\"]\n");

    let (dismissed, after, removed) =
        attached_on(&stopping_engine(&env), &env, |mut client| async move {
            opened(&mut client, "kennel").await;
            let mut seen = Seen::default();
            stopped(&mut client, &mut seen, None).await;
            seen.until(&mut client, |items| items.len() == 1).await;
            let dismissed = seen
                .call(
                    &mut client,
                    10,
                    "needs_you.dismiss",
                    json!({ "item": "wall_refetch_stopped:walled.test" }),
                )
                .await;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let after = seen
                .call(&mut client, 11, "needs_you.list", json!({}))
                .await;
            let removed: Vec<Value> = seen
                .changes
                .iter()
                .filter(|change| change["change"] == "removed")
                .map(|change| json!([change["id"], change["reason"]]))
                .collect();
            (dismissed["result"].clone(), listed(&after), removed)
        });

    assert_eq!(
        (dismissed, after.len(), removed),
        (
            json!({ "dismissed": true }),
            0,
            vec![json!(["wall_refetch_stopped:walled.test", "dismissed"])],
        )
    );
}

/// Answers `method` with `params` from the Kennel once the stop is
/// listed, then waits for the list to empty; answers the answer and each
/// removal with its reason.
async fn answered(mut client: Client, method: String, params: Value) -> (Value, Vec<Value>) {
    opened(&mut client, "kennel").await;
    let mut seen = Seen::default();
    stopped(&mut client, &mut seen, None).await;
    seen.until(&mut client, |items| items.len() == 1).await;
    let answer = seen.call(&mut client, 10, &method, params).await;
    if answer.get("result").is_some() {
        seen.until(&mut client, std::collections::BTreeMap::is_empty)
            .await;
    }
    let removed = seen
        .changes
        .iter()
        .filter(|change| change["change"] == "removed")
        .map(|change| json!([change["id"], change["reason"]]))
        .collect();
    (answer, removed)
}

/// Removal edits the layer that lists the domain, every other entry kept:
/// user configuration or the Operator policy; when only `*` covers it, the
/// list the person gives replaces `*`. The answer goes to the Audit log
/// and the item leaves the list, resolved.
///
/// Covers: story:12-27, operation:needs_you.remove_refetch_domain, setting:wall.refetch_domains
#[rstest]
#[case::from_user_configuration(
    "wall.refetch_domains = [\"other.test\", \"walled.test\"]\n",
    "",
    json!(null),
    (USER_CONFIGURATION_FILE, "user_configuration", json!(["other.test"])),
)]
#[case::from_the_operator_policy(
    "",
    "version = \"0.1\"\n\n[settings]\nwall.refetch_domains = [\"walled.test\", \"other.test\"]\n",
    json!(null),
    (OPERATOR_POLICY_FILE, "operator_policy", json!(["other.test"])),
)]
#[case::a_star_replaced_by_a_list(
    "wall.refetch_domains = [\"*\"]\n",
    "",
    json!(["other.test", "third.test"]),
    (USER_CONFIGURATION_FILE, "user_configuration", json!(["other.test", "third.test"])),
)]
fn removal_edits_the_layer_that_lists_the_domain(
    #[case] configuration: &str,
    #[case] policy: &str,
    #[case] instead: Value,
    #[case] expected: (&str, &str, Value),
) {
    let env = TestEnv::new();
    configure(&env, configuration);
    if !policy.is_empty() {
        write(&env, OPERATOR_POLICY_FILE, policy);
    }
    let mut params = json!({ "domain": STOPPED });
    if !instead.is_null() {
        params["instead_of_star"] = instead.clone();
    }

    let (answer, removed) = attached_on(&stopping_engine(&env), &env, |client| {
        answered(client, "needs_you.remove_refetch_domain".to_owned(), params)
    });

    let (name, layer, domains) = expected;
    let written = std::fs::read_to_string(file(&env, name)).unwrap();
    let entries = audit_entries(&env);
    assert_eq!(
        (
            answer["result"].clone(),
            written.contains(&format!("refetch_domains = {domains}").replace(',', ", ")),
            entries
                .iter()
                .map(|entry| json!({
                    "kind": entry["kind"],
                    "surface": entry["surface"],
                    "layer": entry["layer"],
                    "details": entry["details"],
                }))
                .collect::<Vec<_>>(),
            removed,
        ),
        (
            json!({
                "domain": STOPPED,
                "removed": true,
                "layer": layer,
                "domains": domains,
            }),
            true,
            vec![json!({
                "kind": "refetch_domain_removed",
                "surface": "kennel",
                "layer": layer,
                "details": { "domain": STOPPED, "domains": domains },
            })],
            vec![json!(["wall_refetch_stopped:walled.test", "resolved"])],
        ),
        "{written}"
    );
}

/// Denial writes the domain in the Operator policy's
/// `wall.denied_domains`, creating the file when there is none, and the
/// Audit log records it with its reason; the item leaves the list.
///
/// Covers: story:12-27, operation:needs_you.deny_wall_domain, setting:wall.denied_domains
#[test]
fn denial_writes_wall_denied_domains_with_its_reason() {
    let env = TestEnv::new();
    configure(&env, "wall.refetch_domains = [\"walled.test\"]\n");
    let reason = "The site's owner objected in writing.";

    let (answer, removed) = attached_on(&stopping_engine(&env), &env, |client| {
        answered(
            client,
            "needs_you.deny_wall_domain".to_owned(),
            json!({ "domain": STOPPED, "reason": reason }),
        )
    });

    let written = std::fs::read_to_string(file(&env, OPERATOR_POLICY_FILE)).unwrap();
    let entries = audit_entries(&env);
    assert_eq!(
        (
            answer["result"].clone(),
            written.contains("denied_domains = [\"walled.test\"]"),
            entries
                .iter()
                .map(|entry| json!({
                    "kind": entry["kind"],
                    "layer": entry["layer"],
                    "details": entry["details"],
                }))
                .collect::<Vec<_>>(),
            removed,
        ),
        (
            json!({ "domain": STOPPED, "added": true, "reason": reason }),
            true,
            vec![json!({
                "kind": "wall_domain_denied",
                "layer": "operator_policy",
                "details": { "domain": STOPPED, "reason": reason },
            })],
            vec![json!(["wall_refetch_stopped:walled.test", "resolved"])],
        ),
        "{written}"
    );
}

/// An answer no person may give is refused, every file left as it was and
/// nothing recorded: from an agent surface, a removal when only `*` covers
/// the domain and no list replaces it, a denial without a reason, and a
/// write to a managed Operator policy.
///
/// Covers: story:12-27
#[rstest]
/// Covers: error:person_required, operation:needs_you.deny_wall_domain
#[case::from_an_agent_surface(
    "sdk_typescript",
    "wall.refetch_domains = [\"walled.test\"]\n",
    "",
    "needs_you.deny_wall_domain",
    json!({ "domain": STOPPED, "reason": "objected" }),
    "person_required"
)]
/// Covers: operation:needs_you.remove_refetch_domain
#[case::a_star_with_no_list(
    "kennel",
    "wall.refetch_domains = [\"*\"]\n",
    "",
    "needs_you.remove_refetch_domain",
    json!({ "domain": STOPPED }),
    "invalid_params"
)]
/// Covers: operation:needs_you.deny_wall_domain
#[case::a_denial_without_a_reason(
    "kennel",
    "",
    "",
    "needs_you.deny_wall_domain",
    json!({ "domain": STOPPED, "reason": "  " }),
    "invalid_params"
)]
/// Covers: error:setting_locked, operation:needs_you.deny_wall_domain
#[case::a_managed_operator_policy(
    "kennel",
    "",
    "version = \"0.1\"\n\n[managed]\n",
    "needs_you.deny_wall_domain",
    json!({ "domain": STOPPED, "reason": "objected" }),
    "setting_locked"
)]
fn an_answer_no_person_may_give_is_refused_and_writes_nothing(
    #[case] surface: &str,
    #[case] configuration: &str,
    #[case] policy: &str,
    #[case] method: &str,
    #[case] params: Value,
    #[case] code: &str,
) {
    let env = TestEnv::new();
    configure(&env, configuration);
    if !policy.is_empty() {
        write(&env, OPERATOR_POLICY_FILE, policy);
    }
    let files = || {
        [USER_CONFIGURATION_FILE, OPERATOR_POLICY_FILE]
            .map(|name| std::fs::read_to_string(file(&env, name)).ok())
    };
    let before = files();
    let surface = surface.to_owned();
    let method = method.to_owned();

    let answer = attached_on(&stopping_engine(&env), &env, |mut client| async move {
        opened(&mut client, &surface).await;
        call(&mut client, 2, &method, params).await
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
            files() == before,
            audit_entries(&env).len()
        ),
        (Some(code), true, 0),
        "{answer}"
    );
}

/// A removal the Audit log cannot record is written all the same, since
/// the person gave it, and answers `internal_error` saying the entry is
/// missing rather than succeeding unrecorded.
///
/// Covers: operation:needs_you.remove_refetch_domain
#[test]
fn a_removal_the_audit_log_cannot_record_is_written_and_says_so() {
    let env = TestEnv::new();
    configure(
        &env,
        "wall.refetch_domains = [\"other.test\", \"walled.test\"]\n",
    );
    let home = env.home().to_path_buf();

    let answer = attached_on(&stopping_engine(&env), &env, |mut client| async move {
        opened(&mut client, "kennel").await;
        let mut seen = Seen::default();
        stopped(&mut client, &mut seen, None).await;
        seen.until(&mut client, |items| items.len() == 1).await;
        unrecordable(&home);
        seen.call(
            &mut client,
            10,
            "needs_you.remove_refetch_domain",
            json!({ "domain": STOPPED }),
        )
        .await
    });

    assert_eq!(
        (
            &answer["error"]["code"],
            std::fs::read_to_string(file(&env, USER_CONFIGURATION_FILE))
                .unwrap()
                .contains("refetch_domains = [\"other.test\"]"),
        ),
        (&json!(-32603), true),
        "{answer}"
    );
}
