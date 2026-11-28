//! End-to-end Saved identities against real headless Chrome and the Proving
//! ground, plus the `identity list|import|delete|export` CLI commands (ADRs
//! 0015, 0035 and 0056; ticket #736).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::too_many_lines,
    reason = "end-to-end Proving ground scenario walks login, save, canary scan, and headless reuse in one flow"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "synchronous CLI integration test reads and writes its temp files directly"
)]

use goodfetch_test_support::{
    Canaries, Output, ProvingGround, SecretKind, TestEnv, browser_cache, cli,
};
use serde_json::{Value, json};

use crate::COLLIE;
use crate::page::started_with;
use crate::sessions::{page_in, session_of};

const SEED: u64 = 736;
const FAKE_PASSPHRASE: &str = "gf-fake-cli-export-passphrase-0001";

/// Log in against the Proving ground in a real headless Chrome Session, save
/// the Saved identity, scan Goodfetch home with the canary oracle, then reuse
/// the Saved identity headless in a new Run and confirm the Proving ground
/// observes the identical UA, Accept-Language, and login cookie.
///
/// Covers: story:5-63, operation:session.save_identity, operation:identity.list
#[cfg(unix)]
#[test]
fn real_browser_save_and_reuse_preserves_login_cookie_ua_and_language_with_zero_disk_leaks() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut canaries = Canaries::new(env.seed());
    let login_mark = canaries.plant(SecretKind::TypedPassword).value().to_owned();

    // Run 1: open a Session with an explicit locale & timezone, set login cookie on Proving ground, save_identity.
    let mut settings1 = serde_json::Map::new();
    settings1.insert(
        "browser".to_owned(),
        json!({ "locale": "fr-FR", "timezone_id": "Europe/Paris" }),
    );
    let mut engine1 = started_with(
        &env,
        &cache,
        &proving_ground,
        "save-browser-1",
        &settings1,
        |_| {},
    );
    let s1 = session_of(&engine1.ask(2, "run.new_session", &json!({ "run_id": "save-browser-1" })));
    let p1 = page_in(&mut engine1, 3, &s1);
    let set_url = proving_ground.url("scope.test", "/storage?mark=proving-login-736");
    let nav1 = engine1.ask(
        4,
        "page.goto",
        &json!({ "page_id": p1.page_id, "url": set_url }),
    );
    assert_eq!(nav1["result"]["response"]["status"], 200, "{nav1}");
    let planted = engine1.ask(
        40,
        "page.evaluate",
        &json!({
            "page_id": p1.page_id,
            "function": "(tok) => { document.cookie = 'canary_sid=' + tok + '; Path=/; SameSite=Lax'; localStorage.setItem('canary_ls', tok); return true; }",
            "arg": login_mark,
        }),
    );
    assert_eq!(planted["result"]["value"], true, "{planted}");
    let check_url = proving_ground.url("scope.test", "/?pass=1");
    let nav1_check = engine1.ask(
        5,
        "page.goto",
        &json!({ "page_id": p1.page_id, "url": check_url }),
    );
    assert_eq!(
        nav1_check["result"]["response"]["status"], 200,
        "{nav1_check}"
    );

    let saved = engine1.ask(
        6,
        "session.save_identity",
        &json!({ "session_id": s1, "name": "proving-portal" }),
    );
    assert_eq!(saved["result"]["name"], "proving-portal", "{saved}");
    let listed = engine1.ask(7, "identity.list", &json!({}));
    assert_eq!(
        listed["result"]["identities"][0]["locale"], "fr-FR",
        "{listed}"
    );
    assert_eq!(
        listed["result"]["identities"][0]["timezone_id"], "Europe/Paris",
        "{listed}"
    );
    let _ = engine1.ask(8, "run.finish", &json!({ "run_id": "save-browser-1" }));
    assert!(engine1.finish().1);

    // Canary oracle: zero clear-text cookie anywhere in Goodfetch home!
    let leaks = canaries.leaks(&[Output::Path(env.home())]).unwrap();
    assert!(leaks.is_empty(), "Saved identity leaked on disk: {leaks:?}");

    // Run 2: reuse Saved identity in a new Run without passing locale/timezone manually.
    let mut settings2 = serde_json::Map::new();
    settings2.insert(
        "session".to_owned(),
        json!({ "source": { "kind": "saved_identity", "name": "proving-portal" } }),
    );
    let mut engine2 = started_with(
        &env,
        &cache,
        &proving_ground,
        "reuse-browser-2",
        &settings2,
        |_| {},
    );
    let s2 = session_of(&engine2.ask(
        2,
        "run.new_session",
        &json!({ "run_id": "reuse-browser-2" }),
    ));
    let p2 = page_in(&mut engine2, 3, &s2);
    let check_url2 = proving_ground.url("scope.test", "/?pass=2");
    let nav2 = engine2.ask(
        4,
        "page.goto",
        &json!({ "page_id": p2.page_id, "url": check_url2 }),
    );
    assert_eq!(nav2["result"]["response"]["status"], 200, "{nav2}");
    let tz = engine2.ask(
        5,
        "page.evaluate",
        &json!({
            "page_id": p2.page_id,
            "function": "() => Intl.DateTimeFormat().resolvedOptions().timeZone",
        }),
    );
    assert_eq!(tz["result"]["value"], "Europe/Paris", "{tz}");
    let _ = engine2.ask(6, "run.finish", &json!({ "run_id": "reuse-browser-2" }));
    assert!(engine2.finish().1);

    // Compare what Proving ground observed on pass 1 vs pass 2.
    let recorded = proving_ground.received_requests().unwrap();
    let pass1 = recorded
        .of_the_case()
        .find(|r| r.target == "/?pass=1")
        .expect("pass=1 request recorded");
    let pass2 = recorded
        .of_the_case()
        .find(|r| r.target == "/?pass=2")
        .expect("pass=2 request recorded");

    let header = |headers: &[(String, String)], name: &str| -> Option<String> {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    };
    assert_eq!(
        header(&pass1.headers, "user-agent"),
        header(&pass2.headers, "user-agent")
    );
    assert_eq!(
        header(&pass1.headers, "accept-language"),
        header(&pass2.headers, "accept-language")
    );
    assert!(
        header(&pass2.headers, "cookie")
            .as_deref()
            .is_some_and(|c| c.contains(&login_mark)),
        "expected reused Session to carry login cookie; got {:?}",
        header(&pass2.headers, "cookie")
    );
}

/// CLI `identity import`, `identity list`, `identity export` (encrypted bundle
/// default and clear-text `--format storage-state` refused under `--json`),
/// and `identity delete`.
///
/// Covers: story:5-63, command:identity.import, command:identity.list, command:identity.export, command:identity.delete, error:person_required
#[test]
fn cli_identity_commands_round_trip_and_refuse_clear_text_under_json() {
    let env = TestEnv::new();
    let work = tempfile::tempdir().unwrap();
    let bundle_file = work.path().join("portal.age");

    let imported = cli(&env, COLLIE)
        .args([
            "identity",
            "import",
            "cli-portal",
            "--cookie-header",
            "sid=cli_cookie_value",
            "--domain",
            "cli.test",
            "--json",
        ])
        .run()
        .unwrap();
    assert_eq!(imported.exit, Some(0), "{imported}");
    let info: Value = serde_json::from_str(imported.stdout.trim()).unwrap();
    assert_eq!(info["name"], "cli-portal");

    let listed = cli(&env, COLLIE)
        .args(["identity", "list", "--json"])
        .run()
        .unwrap();
    assert_eq!(listed.exit, Some(0), "{listed}");
    let list_doc: Value = serde_json::from_str(listed.stdout.trim()).unwrap();
    assert_eq!(list_doc["identities"].as_array().unwrap().len(), 1);

    // Export encrypted bundle by default.
    let exported = cli(&env, COLLIE)
        .env("CLI_EXPORT_KEY", FAKE_PASSPHRASE)
        .args([
            "identity",
            "export",
            "cli-portal",
            "--passphrase",
            "env:CLI_EXPORT_KEY",
            "--out",
            bundle_file.to_str().unwrap(),
            "--json",
        ])
        .run()
        .unwrap();
    assert_eq!(exported.exit, Some(0), "{exported}");
    let bundle_bytes = std::fs::read_to_string(&bundle_file).unwrap();
    assert!(bundle_bytes.contains("BEGIN AGE ENCRYPTED FILE"));
    assert!(!bundle_bytes.contains("cli_cookie_value"));

    // Clear-text --format storage-state under --json is refused with person_required.
    let refused = cli(&env, COLLIE)
        .args([
            "identity",
            "export",
            "cli-portal",
            "--format",
            "storage-state",
            "--json",
        ])
        .run()
        .unwrap();
    assert_ne!(refused.exit, Some(0), "{refused}");
    let err_doc: Value = serde_json::from_str(refused.stdout.trim()).unwrap();
    assert_eq!(err_doc["error"]["code"], "person_required", "{err_doc}");

    // Delete removes the Saved identity.
    let deleted = cli(&env, COLLIE)
        .args(["identity", "delete", "cli-portal", "--json"])
        .run()
        .unwrap();
    assert_eq!(deleted.exit, Some(0), "{deleted}");
    let del_doc: Value = serde_json::from_str(deleted.stdout.trim()).unwrap();
    assert_eq!(del_doc["deleted"], true);
}
