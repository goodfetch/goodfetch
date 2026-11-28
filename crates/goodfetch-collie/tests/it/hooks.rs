//! Settings-named Hook cases (ADRs 0033, 0088 and 0104; issue 954):
//! process-Plugin Hooks configured via `hooks` run in layer order (Operator
//! policy -> user configuration -> project configuration -> run layer) around
//! every main-frame navigation of a Collie Run (`before_navigation`,
//! `after_navigation`, `page_ready`), receive `{ page, url, run }` with the
//! single catalog `snake_case` stage name, can set a cookie or block a
//! resource on every page (an Agent's included), refuse `hook_duplicate` and
//! Foxhound-only `labels`/`datasets` (`not_in_product`), and cannot mutate a
//! logged-in Session's requests.
#![expect(
    clippy::disallowed_methods,
    reason = "integration tests lay out Plugin bundles and read trace files on their own thread"
)]

use std::fs;
use std::path::Path;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::home::{RUN_TRACE_PLUGIN_CALLS, RUN_TRACE_PROVENANCE};
use goodfetch_protocol::plugin::HookStageName;
use goodfetch_test_client::{Attached, Hello, HomeLayout};
use goodfetch_test_support::{ProvingGround, TestEnv, Tool, browser_cache, tools};
use serde_json::{Value, json};

use crate::COLLIE;
use crate::page::{opened, started_with};
use crate::plugin_host::hello;

const SEED: u64 = 954;

/// Installs a scripted Hook Plugin named `name` into `home`'s Plugin store and
/// allows it in Operator policy.
fn install_hook_plugin(home: &Path, name: &str, stages: &[&str], answer: &Value) {
    let policy = HomeLayout::of(home).operator_policy();
    fs::create_dir_all(policy.parent().unwrap()).unwrap();
    let written = fs::read_to_string(&policy).unwrap_or_else(|_| "version = \"0.1\"\n".to_owned());
    fs::write(
        &policy,
        format!("{written}\n[settings.plugins.{name}]\nallow = true\n"),
    )
    .unwrap();
    let bundle = home.join("plugins").join(name).join("1.0.0");
    fs::create_dir_all(&bundle).unwrap();
    let scripted = tools()
        .unwrap()
        .path(Tool::ScriptedPlugin)
        .unwrap()
        .to_owned();
    fs::copy(scripted, bundle.join("plugin")).unwrap();
    let mut hello_result = hello(&json!([{ "kind": "hook", "capabilities": [] }]));
    hello_result["name"] = json!(name);
    hello_result["hook_stages"] = json!(stages);
    let script = json!({
        "hello": hello_result,
        "answers": { "hook.call": answer },
    });
    fs::write(bundle.join("script.json"), script.to_string()).unwrap();
    let manifest = json!({
        "manifest_version": "0.1",
        "name": name,
        "version": "1.0.0",
        "kinds": ["hook"],
        "hook_stages": stages,
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
        "network_level": "none",
    });
    fs::write(bundle.join("goodfetch-plugin.json"), manifest.to_string()).unwrap();
}

fn read_jsonl(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn setup_layered_hooks(home: &Path, target_url: &str) {
    let stages = ["before_navigation", "after_navigation", "page_ready"];
    install_hook_plugin(
        home,
        "policy-cookie-hook",
        &stages,
        &json!({
            "call_back": {
                "method": "page.set_cookie",
                "params": {
                    "name": "operator_tag",
                    "value": "from-hook",
                    "url": target_url,
                },
                "then": { "result": {} }
            }
        }),
    );
    install_hook_plugin(
        home,
        "user-block-hook",
        &stages,
        &json!({
            "call_back": {
                "method": "page.block_url",
                "params": {
                    "url": "**/blocked-by-hook.png",
                },
                "then": { "result": {} }
            }
        }),
    );
    install_hook_plugin(home, "run-stage-hook", &stages, &json!({ "result": {} }));

    let policy_path = HomeLayout::of(home).operator_policy();
    let policy_text = fs::read_to_string(&policy_path).unwrap();
    fs::write(
        &policy_path,
        format!(
            "{policy_text}\n[[settings.hooks]]\nname = \"policy-cookie\"\nstage = \"before_navigation\"\nplugin = \"policy-cookie-hook\"\n"
        ),
    )
    .unwrap();

    let user_config = HomeLayout::of(home).user_configuration();
    fs::create_dir_all(user_config.parent().unwrap()).unwrap();
    fs::write(
        &user_config,
        "version = \"0.1\"\n\n[[settings.hooks]]\nname = \"user-block\"\nstage = \"before_navigation\"\nplugin = \"user-block-hook\"\n",
    )
    .unwrap();
}

/// Process-Plugin Hooks from Operator policy, user configuration and run
/// layers run in that exact order across `before_navigation`,
/// `after_navigation` and `page_ready` on a Collie Run navigation; a
/// `before_navigation` Hook sets a cookie on the page before the request is
/// sent and blocks a resource, and the Run trace records the single catalog
/// `snake_case` stage name everywhere.
///
/// Covers: story:7-63, story:7-69, story:7-110
#[test]
fn settings_named_hooks_run_in_layer_order_and_set_cookie_around_navigation() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let target_url = proving_ground.url("shop.test", "/");
    setup_layered_hooks(env.home(), &target_url);

    let cache = browser_cache(&env).unwrap();
    let run_settings = json!({
        "hooks": [
            { "name": "run-after", "stage": "after_navigation", "plugin": "run-stage-hook" },
            { "name": "run-ready", "stage": "page_ready", "plugin": "run-stage-hook" },
        ]
    });
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "hooks-run",
        run_settings.as_object().unwrap(),
        |_| {},
    );
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "hooks-run" }))).page_id;
    let navigated = engine.ask(
        3,
        "page.goto",
        &json!({ "page_id": page, "url": target_url }),
    );
    assert!(navigated.get("result").is_some(), "{navigated}");

    let cookies = engine.ask(
        4,
        "page.evaluate",
        &json!({ "page_id": page, "function": "() => document.cookie" }),
    );
    assert!(
        cookies["result"]["value"]
            .as_str()
            .unwrap_or_default()
            .contains("operator_tag=from-hook"),
        "expected cookie set by before_navigation Hook, got: {cookies}"
    );

    let blocked_url = proving_ground.url("shop.test", "/blocked-by-hook.png");
    let fetch_blocked = engine.ask(
        5,
        "page.evaluate",
        &json!({
            "page_id": page,
            "function": "(u) => fetch(u).then(() => 'ok').catch(() => 'blocked')",
            "arg": blocked_url,
        }),
    );
    assert_eq!(
        fetch_blocked["result"]["value"].as_str(),
        Some("blocked"),
        "expected subresource blocked by user Hook: {fetch_blocked}"
    );

    let _ = engine.ask(6, "run.finish", &json!({ "run_id": "hooks-run" }));
    let _ = engine.finish();

    let provenance_path = HomeLayout::of(env.home()).run_trace("hooks-run", RUN_TRACE_PROVENANCE);
    let hook_events: Vec<(String, String)> = read_jsonl(&provenance_path)
        .into_iter()
        .filter(|entry| entry["action"] == "hook_call")
        .map(|entry| {
            (
                entry["hook"].as_str().unwrap().to_owned(),
                entry["stage"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        hook_events,
        vec![
            (
                "policy-cookie".to_owned(),
                HookStageName::BEFORE_NAVIGATION.as_str().to_owned()
            ),
            (
                "user-block".to_owned(),
                HookStageName::BEFORE_NAVIGATION.as_str().to_owned()
            ),
            (
                "run-after".to_owned(),
                HookStageName::AFTER_NAVIGATION.as_str().to_owned()
            ),
            (
                "run-ready".to_owned(),
                HookStageName::PAGE_READY.as_str().to_owned()
            ),
        ]
    );

    let calls_path = HomeLayout::of(env.home()).run_trace("hooks-run", RUN_TRACE_PLUGIN_CALLS);
    let calls = read_jsonl(&calls_path);
    assert_eq!(calls.len(), 4, "{calls:?}");
    assert!(calls.iter().all(|entry| entry["method"] == "hook.call"));
}

/// Opening a Collie Run whose `hooks` repeats an Operator policy Hook name
/// fails with `hook_duplicate`, and setting Foxhound's `labels` fails with
/// `not_in_product`.
///
/// Covers: story:7-63, story:7-110
#[test]
fn settings_named_hooks_refuse_duplicates_and_foxhound_fields_on_collie() {
    let env = TestEnv::new();
    let policy_path = HomeLayout::of(env.home()).operator_policy();
    fs::create_dir_all(policy_path.parent().unwrap()).unwrap();
    fs::write(
        &policy_path,
        "version = \"0.1\"\n\n[[settings.hooks]]\nname = \"dup\"\nstage = \"before_navigation\"\nplugin = \"p1\"\n",
    )
    .unwrap();

    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&Hello::needing(Product::Collie).message());
    let _ = engine.next();

    let dup = engine.ask(
        2,
        "run.start",
        &json!({
            "run_id": "dup-run",
            "settings": {
                "hooks": [{ "name": "dup", "stage": "page_ready", "plugin": "p2" }]
            }
        }),
    );
    assert_eq!(
        dup["error"]["data"]["code"].as_str(),
        Some("hook_duplicate"),
        "{dup}"
    );
    assert_eq!(dup["error"]["data"]["hook"].as_str(), Some("dup"), "{dup}");
    assert_eq!(
        dup["error"]["data"]["stage"].as_str(),
        Some("page_ready"),
        "{dup}"
    );

    let fox = engine.ask(
        3,
        "run.start",
        &json!({
            "run_id": "fox-run",
            "settings": {
                "hooks": [{
                    "name": "fox-only",
                    "stage": "page_ready",
                    "plugin": "p2",
                    "labels": ["listing"]
                }]
            }
        }),
    );
    assert_eq!(
        fox["error"]["data"]["code"].as_str(),
        Some("not_in_product"),
        "{fox}"
    );
    let _ = engine.finish();
}
