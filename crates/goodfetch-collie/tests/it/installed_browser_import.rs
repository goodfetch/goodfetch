//! Installed Chromium-family browser imports (`source: { kind: "installed_browser" }`
//! and CLI `identity import --browser chrome|edge|brave`, ADR 0056; ticket #738)
//! against Collie's executable with prepared Profile directories, a real
//! pipe-speaking scripted Chrome executable, and the Proving ground.
#![expect(
    clippy::too_many_lines,
    reason = "end-to-end Session source scenarios verify pipe launch, CDP reading, DBSC exclusion, family warning, and Proving ground reuse together"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "synchronous CLI integration test prepares Profile directory fixtures and scripted Chrome binaries on disk"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use goodfetch_test_support::{
    Canaries, Output, ProvingGround, SecretKind, TestEnv, browser_cache, cli,
};
use serde_json::{Value, json};

use crate::COLLIE;
use crate::page::started_with;
use crate::sessions::{page_in, session_of};

const SEED: u64 = 738;

/// Prepares a closed, non-default Chromium Profile directory with `fr-FR`
/// locale metadata and an unreadable/corrupt `Cookies` sentinel file so any
/// attempt by Collie to read or decrypt cookies directly would fail.
fn prepare_profile_dir(root: &Path, profile_subdir: &str) -> PathBuf {
    let profile_dir = root.join(profile_subdir);
    std::fs::create_dir_all(&profile_dir).unwrap();
    std::fs::write(root.join("Last Version"), "136.0.7103.48\n").unwrap();
    std::fs::write(
        root.join("Local State"),
        json!({
            "user_experience_metrics": { "stability": { "stats_version": "136.0.7103.48" } },
            "intl": { "app_locale": "fr-FR" }
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        profile_dir.join("Preferences"),
        json!({ "intl": { "accept_languages": "fr-FR,fr;q=0.9,en;q=0.5" } }).to_string(),
    )
    .unwrap();
    // Corrupt non-SQLite sentinel file with 0000 permissions: proves Collie
    // never opens or decrypts browser cookie files.
    let cookies_path = profile_dir.join("Cookies");
    std::fs::write(&cookies_path, b"NOT_A_SQLITE_DB_NEVER_DECRYPT_ME").unwrap();
    let mut perms = std::fs::metadata(&cookies_path).unwrap().permissions();
    perms.set_mode(0o000);
    std::fs::set_permissions(&cookies_path, perms).unwrap();
    root.to_path_buf()
}

struct ScriptedChromeSpec<'a> {
    product: &'a str,
    domain: &'a str,
    origin: &'a str,
    source_port: u16,
    cookie_value: &'a str,
    http_only_value: &'a str,
    dbsc_cookie_value: &'a str,
    local_storage_value: &'a str,
    idb_value: &'a str,
    dbsc_supported: bool,
    argv_log: &'a Path,
}

/// Writes an executable Python 3 scripted Chrome binary that speaks Chromium's
/// NUL-delimited CDP over `--remote-debugging-pipe` (fd 3 read, fd 4 write),
/// logs its argv, and serves cookies, `DOMStorage`, `IndexedDB`, and
/// `Network.enableDeviceBoundSessions`.
fn write_scripted_chrome(path: &Path, spec: &ScriptedChromeSpec<'_>) {
    let config = json!({
        "product": spec.product,
        "domain": spec.domain,
        "origin": spec.origin,
        "sourcePort": spec.source_port,
        "cookieValue": spec.cookie_value,
        "httpOnlyValue": spec.http_only_value,
        "dbscCookieValue": spec.dbsc_cookie_value,
        "localStorageValue": spec.local_storage_value,
        "idbValue": spec.idb_value,
        "dbscSupported": spec.dbsc_supported,
        "argvLog": spec.argv_log.display().to_string(),
    });
    let config_path = path.with_extension("json");
    std::fs::write(&config_path, config.to_string()).unwrap();

    let script = format!(
        r#"#!/usr/bin/env python3
import json, os, sys

with open({config_path:?}, "r", encoding="utf-8") as f:
    cfg = json.load(f)

with open(cfg["argvLog"], "w", encoding="utf-8") as f:
    json.dump(sys.argv[1:], f)

def send(obj):
    data = (json.dumps(obj) + "\0").encode("utf-8")
    os.write(4, data)

buf = b""
while True:
    chunk = os.read(3, 65536)
    if not chunk:
        break
    buf += chunk
    while b"\0" in buf:
        raw, buf = buf.split(b"\0", 1)
        if not raw:
            continue
        msg = json.loads(raw.decode("utf-8"))
        cmd_id = msg["id"]
        method = msg["method"]
        params = msg.get("params", {{}})
        sid = msg.get("sessionId")

        def reply(result):
            out = {{"id": cmd_id, "result": result}}
            if sid is not None:
                out["sessionId"] = sid
            send(out)

        def fail(code, message):
            out = {{"id": cmd_id, "error": {{"code": code, "message": message}}}}
            if sid is not None:
                out["sessionId"] = sid
            send(out)

        if method == "Browser.getVersion":
            reply({{
                "protocolVersion": "1.3",
                "product": cfg["product"],
                "revision": "@123456",
                "userAgent": "Mozilla/5.0 Chrome/136.0.7103.48",
                "jsVersion": "13.6",
            }})
        elif method == "Target.createTarget":
            reply({{"targetId": "reader-target-1"}})
        elif method == "Target.attachToTarget":
            reply({{"sessionId": "reader-session-1"}})
        elif method == "Target.closeTarget":
            reply({{"success": True}})
        elif method == "Network.enableDeviceBoundSessions":
            if not cfg["dbscSupported"]:
                fail(-32601, "'Network.enableDeviceBoundSessions' wasn't found")
            else:
                send({{
                    "method": "Network.deviceBoundSessionsAdded",
                    "sessionId": sid,
                    "params": {{
                        "sessions": [{{
                            "key": {{"site": cfg["origin"], "id": "dbsc-session-1"}},
                            "refreshUrl": cfg["origin"] + "/refresh",
                            "inclusionRules": {{
                                "origin": cfg["origin"],
                                "includeSite": True,
                                "urlRules": [],
                            }},
                            "cookieCravings": [{{
                                "name": "__Secure-1PSIDTS",
                                "domain": cfg["domain"],
                                "path": "/",
                                "secure": True,
                                "httpOnly": True,
                            }}],
                            "expiryDate": 1900000000.0,
                            "allowedRefreshInitiators": [],
                        }}]
                    }},
                }})
                reply({{}})
        elif method == "Storage.getCookies":
            reply({{
                "cookies": [
                    {{
                        "name": "session_token",
                        "value": cfg["cookieValue"],
                        "domain": cfg["domain"],
                        "path": "/",
                        "expires": 1900000000.0,
                        "size": 24,
                        "httpOnly": False,
                        "secure": False,
                        "session": False,
                        "sameSite": "Lax",
                        "priority": "Medium",
                        "sourceScheme": "NonSecure",
                        "sourcePort": cfg["sourcePort"],
                    }},
                    {{
                        "name": "auth_http_only",
                        "value": cfg["httpOnlyValue"],
                        "domain": cfg["domain"],
                        "path": "/",
                        "expires": 1900000000.0,
                        "size": 24,
                        "httpOnly": True,
                        "secure": False,
                        "session": False,
                        "sameSite": "Lax",
                        "priority": "Medium",
                        "sourceScheme": "NonSecure",
                        "sourcePort": cfg["sourcePort"],
                    }},
                    {{
                        "name": "__Secure-1PSIDTS",
                        "value": cfg["dbscCookieValue"],
                        "domain": cfg["domain"],
                        "path": "/",
                        "expires": 1900000000.0,
                        "size": 24,
                        "httpOnly": True,
                        "secure": True,
                        "session": False,
                        "sameSite": "Lax",
                        "priority": "Medium",
                        "sourceScheme": "Secure",
                        "sourcePort": 443,
                    }},
                    {{
                        "name": "unrelated",
                        "value": "ignore-me",
                        "domain": "unrelated.invalid",
                        "path": "/",
                        "expires": 1900000000.0,
                        "size": 12,
                        "httpOnly": False,
                        "secure": False,
                        "session": False,
                        "sameSite": "Lax",
                        "priority": "Medium",
                        "sourceScheme": "NonSecure",
                        "sourcePort": 80,
                    }},
                ]
            }})
        elif method in ("DOMStorage.enable", "IndexedDB.enable"):
            reply({{}})
        elif method == "DOMStorage.getDOMStorageItems":
            origin = params.get("storageId", {{}}).get("securityOrigin", "")
            if origin == cfg["origin"]:
                reply({{"entries": [["imported_ls", cfg["localStorageValue"]]]}})
            else:
                reply({{"entries": []}})
        elif method == "IndexedDB.requestDatabaseNames":
            origin = params.get("securityOrigin", "")
            if origin == cfg["origin"]:
                reply({{"databaseNames": ["app_auth"]}})
            else:
                reply({{"databaseNames": []}})
        elif method == "IndexedDB.requestDatabase":
            reply({{
                "databaseWithObjectStores": {{
                    "name": "app_auth",
                    "version": 1,
                    "objectStores": [{{
                        "name": "tokens",
                        "keyPath": {{"type": "string", "string": "id"}},
                        "autoIncrement": False,
                        "indexes": [],
                    }}],
                }}
            }})
        elif method == "IndexedDB.requestData":
            reply({{
                "objectStoreDataEntries": [{{
                    "key": {{"type": "string", "value": "primary"}},
                    "primaryKey": {{"type": "string", "value": "primary"}},
                    "value": {{"type": "object", "value": {{"id": "primary", "token": cfg["idbValue"]}}}},
                }}],
                "hasMore": False,
            }})
        elif method == "Browser.close":
            reply({{}})
            sys.exit(0)
        else:
            reply({{}})
"#,
        config_path = config_path.display().to_string()
    );
    std::fs::write(path, script).unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

/// Run-only `source: { kind: "installed_browser" }` on Collie's executable
/// launches the scripted Chrome on a closed non-default Profile directory over
/// a pipe (never decrypting `Cookies`), carries cookies, `localStorage`, and
/// `IndexedDB` into a real headless Chrome Session on the Proving ground with
/// `standard`'s `fr-FR` locale, leaves out device-bound session cookies with a
/// warning, warns on a different source browser family (`edge`), and leaves the
/// Saved identities store empty.
///
/// Covers: story:5-74, story:5-75
#[test]
fn installed_browser_run_only_import_carries_cookies_storage_idb_and_excludes_dbsc() {
    let env = TestEnv::new();
    let work = tempfile::tempdir().unwrap();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();

    let host = "scope.test";
    let base_url = proving_ground.url(host, "/");
    let parsed_url = url::Url::parse(&base_url).unwrap();
    let source_port = parsed_url.port_or_known_default().unwrap();
    let origin = parsed_url.origin().ascii_serialization();
    let mut canaries = Canaries::new(env.seed());
    let cookie_val = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let httponly_val = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let dbsc_val = "dbsc-ephemeral-never-imported";
    let ls_val = "ls-from-installed-chrome";
    let idb_val = "idb-from-installed-chrome";

    let profile_root = prepare_profile_dir(&work.path().join("chrome-work-profile"), "Default");
    let argv_log = work.path().join("chrome-argv.json");
    let scripted_chrome = work.path().join("scripted-chrome.py");
    write_scripted_chrome(
        &scripted_chrome,
        &ScriptedChromeSpec {
            product: "Chrome/136.0.7103.48",
            domain: host,
            origin: &origin,
            source_port,
            cookie_value: &cookie_val,
            http_only_value: &httponly_val,
            dbsc_cookie_value: dbsc_val,
            local_storage_value: ls_val,
            idb_value: idb_val,
            dbsc_supported: true,
            argv_log: &argv_log,
        },
    );

    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "ib-run-only",
        &serde_json::Map::new(),
        |_| {},
    );
    let session = session_of(&engine.ask(
        2,
        "run.new_session",
        &json!({
            "run_id": "ib-run-only",
            "source": {
                "kind": "installed_browser",
                "browser": "chrome",
                "path": profile_root.display().to_string(),
                "executable": scripted_chrome.display().to_string(),
                "domains": ["scope.test"],
            },
        }),
    ));

    // Verify the scripted Chrome was launched on the closed non-default
    // directory over `--remote-debugging-pipe`, never `--remote-debugging-port`.
    let argv: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(&argv_log).unwrap()).unwrap();
    assert!(
        argv.iter().any(|a| a == "--remote-debugging-pipe"),
        "must launch over debugging pipe: {argv:?}"
    );
    assert!(
        argv.iter()
            .any(|a| a == &format!("--user-data-dir={}", profile_root.display())),
        "must pass closed non-default profile dir: {argv:?}"
    );
    assert!(
        !argv
            .iter()
            .any(|a| a.starts_with("--remote-debugging-port")),
        "must never open a debugging port: {argv:?}"
    );

    let page = page_in(&mut engine, 3, &session);
    let url = proving_ground.url(host, "/?ib=1");
    let nav = engine.ask(
        4,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": url }),
    );
    assert_eq!(
        nav["result"]["response"]["status"], 200,
        "goto failed: {nav}"
    );

    // Read localStorage + IndexedDB carried into the page.
    let probe = engine.ask(
        5,
        "page.evaluate",
        &json!({
            "page_id": page.page_id,
            "function": r#"async () => {
                const ls = localStorage.getItem("imported_ls");
                const idb = await new Promise((resolve) => {
                    const req = indexedDB.open("app_auth", 1);
                    req.onerror = () => resolve(null);
                    req.onsuccess = () => {
                        const db = req.result;
                        if (!db.objectStoreNames.contains("tokens")) { db.close(); resolve(null); return; }
                        const tx = db.transaction("tokens", "readonly");
                        const get = tx.objectStore("tokens").get("primary");
                        get.onsuccess = () => { const v = get.result; db.close(); resolve(v); };
                        get.onerror = () => { db.close(); resolve(null); };
                    };
                });
                return { ls, idb };
            }"#,
        }),
    );
    assert!(probe.get("error").is_none(), "evaluate failed: {probe}");
    let val = &probe["result"]["value"];
    assert_eq!(val["ls"], json!(ls_val), "localStorage must be carried");
    assert_eq!(
        val["idb"]["token"],
        json!(idb_val),
        "IndexedDB must be carried: {val}"
    );

    // Check headers and cookies received by Proving ground.
    let recorded = proving_ground.received_requests().unwrap();
    let hit = recorded
        .of_the_case()
        .find(|r| r.target == "/?ib=1")
        .expect("proving ground received /?ib=1");
    let header = |name: &str| -> Option<String> {
        hit.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    };
    let sent_cookies = header("cookie").unwrap_or_default();
    assert!(
        sent_cookies.contains(&cookie_val),
        "imported login cookie must be sent: {sent_cookies:?}"
    );
    assert!(
        sent_cookies.contains(&httponly_val),
        "imported HttpOnly cookie must be sent: {sent_cookies:?}"
    );
    assert!(
        !sent_cookies.contains(dbsc_val),
        "device-bound session cookie must be left out: {sent_cookies:?}"
    );
    let accept_lang = header("accept-language").unwrap_or_default();
    assert!(
        accept_lang.starts_with("fr-FR"),
        "source browser locale must be presented: {accept_lang:?}"
    );

    // Open an Edge-sourced Session in the same Run to verify the
    // different-browser-family warning.
    let edge_argv = work.path().join("edge-argv.json");
    let scripted_edge = work.path().join("scripted-edge.py");
    write_scripted_chrome(
        &scripted_edge,
        &ScriptedChromeSpec {
            product: "Edg/136.0.3240.50",
            domain: host,
            origin: &origin,
            source_port,
            cookie_value: &cookie_val,
            http_only_value: &httponly_val,
            dbsc_cookie_value: dbsc_val,
            local_storage_value: ls_val,
            idb_value: idb_val,
            dbsc_supported: true,
            argv_log: &edge_argv,
        },
    );
    let _edge_session = session_of(&engine.ask(
        6,
        "run.new_session",
        &json!({
            "run_id": "ib-run-only",
            "source": {
                "kind": "installed_browser",
                "browser": "edge",
                "path": profile_root.display().to_string(),
                "executable": scripted_edge.display().to_string(),
                "domains": ["scope.test"],
            },
        }),
    ));

    let finished = engine.ask(7, "run.finish", &json!({ "run_id": "ib-run-only" }));
    assert!(finished.get("error").is_none(), "finish failed: {finished}");
    let report = engine.ask(8, "run.report", &json!({ "run_id": "ib-run-only" }));
    let warnings = report["result"]["warnings"]
        .as_array()
        .expect("report warnings array");
    let dbsc_warn = warnings
        .iter()
        .find(|w| w["code"] == json!("device_bound_session_excluded"))
        .expect("device_bound_session_excluded warning must be recorded");
    assert!(
        dbsc_warn["message"]
            .as_str()
            .unwrap()
            .contains("works only in its own browser"),
        "DBSC warning must explain login works only in its own browser: {dbsc_warn}"
    );
    // Chrome import must NOT warn about family mismatch; Edge import MUST.
    let family_warns: Vec<&Value> = warnings
        .iter()
        .filter(|w| w["code"] == json!("imported_identity_family_differs"))
        .collect();
    assert_eq!(
        family_warns.len(),
        1,
        "only the Edge import should raise imported_identity_family_differs: {warnings:?}"
    );

    // Confirm run-only import left Saved identities empty.
    let listed = engine.ask(9, "identity.list", &json!({}));
    assert_eq!(listed["result"]["identities"], json!([]));

    let (_, exited) = engine.finish();
    assert!(exited);

    // Canary oracle: zero plaintext cookie leaks on disk.
    let hits = canaries.leaks(&[Output::Path(env.home())]).unwrap();
    assert!(
        hits.is_empty(),
        "run-only installed_browser import leaked canaries on disk: {hits:?}"
    );
}

/// CLI `identity import --browser chrome|edge|brave` imports into the encrypted
/// Saved identity store, passes `--profile-directory`, surfaces DBSC and
/// family-mismatch warnings, handles browsers lacking
/// `Network.enableDeviceBoundSessions`, and refuses locked or default OS
/// directories with `profile_directory_in_use`.
///
/// Covers: story:5-74, story:5-75
#[test]
fn cli_identity_import_supports_chromium_family_warnings_probe_fallback_and_refusals() {
    let env = TestEnv::new();
    let work = tempfile::tempdir().unwrap();
    let mut canaries = Canaries::new(env.seed());
    let cookie_val = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let httponly_val = canaries.plant(SecretKind::TypedPassword).value().to_owned();

    let profile_root = prepare_profile_dir(&work.path().join("cli-chrome-profile"), "Profile 1");
    let argv_log = work.path().join("cli-chrome-argv.json");
    let scripted_chrome = work.path().join("cli-scripted-chrome.py");
    write_scripted_chrome(
        &scripted_chrome,
        &ScriptedChromeSpec {
            product: "Chrome/136.0.7103.48",
            domain: "shop.example",
            origin: "https://shop.example",
            source_port: 443,
            cookie_value: &cookie_val,
            http_only_value: &httponly_val,
            dbsc_cookie_value: "dbsc-cli-excluded",
            local_storage_value: "cli-ls-value",
            idb_value: "cli-idb-value",
            dbsc_supported: true,
            argv_log: &argv_log,
        },
    );

    // 1. CLI `identity import` for Chrome into Saved identities.
    let res = cli(&env, COLLIE)
        .args([
            "identity",
            "import",
            "work-chrome",
            "--browser",
            "chrome",
            "--profile-dir",
            profile_root.to_str().unwrap(),
            "--chromium-profile",
            "Profile 1",
            "--executable",
            scripted_chrome.to_str().unwrap(),
            "--domain",
            "shop.example",
            "--json",
        ])
        .run()
        .unwrap();
    assert_eq!(res.exit, Some(0), "chrome import failed: {res}");
    let doc: Value = serde_json::from_str(res.stdout.trim()).unwrap();
    assert_eq!(doc["name"], json!("work-chrome"));
    assert_eq!(doc["access_mode"], json!("standard"));
    assert_eq!(doc["browser_family"], json!("chrome"));
    assert_eq!(doc["locale"], json!("fr-FR"));
    assert_eq!(doc["machine_bound"], json!(true));
    assert_eq!(doc["source_kind"], json!("installed_browser"));
    assert_eq!(doc["cookie_count"], json!(2));
    assert_eq!(doc["origin_count"], json!(1));
    let warnings = doc["warnings"].as_array().expect("warnings array");
    assert!(
        warnings.iter().any(|w| w
            .as_str()
            .unwrap()
            .contains("works only in its own browser")),
        "missing DBSC warning: {warnings:?}"
    );
    assert!(
        !warnings.iter().any(|w| w
            .as_str()
            .unwrap()
            .contains("differs from the source browser")),
        "Chrome on Chrome must not warn about family mismatch: {warnings:?}"
    );

    let argv: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(&argv_log).unwrap()).unwrap();
    assert!(
        argv.iter().any(|a| a == "--profile-directory=Profile 1"),
        "expected --profile-directory=Profile 1 in {argv:?}"
    );

    // 2. CLI `identity import` for Brave (human output) warns on both DBSC and
    // browser family mismatch.
    let res_brave = cli(&env, COLLIE)
        .args([
            "identity",
            "import",
            "work-brave",
            "--browser",
            "brave",
            "--profile-dir",
            profile_root.to_str().unwrap(),
            "--chromium-profile",
            "Profile 1",
            "--executable",
            scripted_chrome.to_str().unwrap(),
            "--domain",
            "shop.example",
        ])
        .run()
        .unwrap();
    assert_eq!(res_brave.exit, Some(0), "brave import failed: {res_brave}");
    assert!(
        res_brave.stderr.contains("works only in its own browser"),
        "human stderr must include DBSC warning: {}",
        res_brave.stderr
    );
    assert!(
        res_brave.stderr.contains("differs from the source browser"),
        "human stderr must include family mismatch warning: {}",
        res_brave.stderr
    );

    // 3. Scripted browser lacking `Network.enableDeviceBoundSessions` warns and
    // still completes the import.
    let no_dbsc_argv = work.path().join("no-dbsc-argv.json");
    let scripted_no_dbsc = work.path().join("scripted-no-dbsc.py");
    write_scripted_chrome(
        &scripted_no_dbsc,
        &ScriptedChromeSpec {
            product: "Chrome/130.0.0.0",
            domain: "shop.example",
            origin: "https://shop.example",
            source_port: 443,
            cookie_value: &cookie_val,
            http_only_value: &httponly_val,
            dbsc_cookie_value: "kept-when-probe-unavailable",
            local_storage_value: "cli-ls-value",
            idb_value: "cli-idb-value",
            dbsc_supported: false,
            argv_log: &no_dbsc_argv,
        },
    );
    let res_probe = cli(&env, COLLIE)
        .args([
            "identity",
            "import",
            "old-chrome",
            "--browser",
            "chrome",
            "--profile-dir",
            profile_root.to_str().unwrap(),
            "--chromium-profile",
            "Profile 1",
            "--executable",
            scripted_no_dbsc.to_str().unwrap(),
            "--domain",
            "shop.example",
            "--json",
        ])
        .run()
        .unwrap();
    assert_eq!(
        res_probe.exit,
        Some(0),
        "probe fallback import failed: {res_probe}"
    );
    let probe_doc: Value = serde_json::from_str(res_probe.stdout.trim()).unwrap();
    assert_eq!(probe_doc["cookie_count"], json!(3));
    assert!(
        probe_doc["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("enableDeviceBoundSessions")),
        "expected probe-unavailable warning: {probe_doc}"
    );

    // 4. Locked profile directory is refused with `profile_directory_in_use`.
    std::fs::write(profile_root.join("SingletonLock"), b"locked").unwrap();
    let res_locked = cli(&env, COLLIE)
        .args([
            "identity",
            "import",
            "locked-chrome",
            "--browser",
            "chrome",
            "--profile-dir",
            profile_root.to_str().unwrap(),
            "--executable",
            scripted_chrome.to_str().unwrap(),
            "--domain",
            "shop.example",
            "--json",
        ])
        .run()
        .unwrap();
    assert_ne!(res_locked.exit, Some(0));
    let locked_err: Value = serde_json::from_str(res_locked.stdout.trim()).unwrap();
    assert_eq!(
        locked_err["error"]["code"],
        json!("profile_directory_in_use")
    );

    // 5. Default OS user data directory is refused with `profile_directory_in_use`.
    let fake_home = work.path().join("fake-home");
    let default_chrome_dir = if cfg!(target_os = "macos") {
        fake_home.join("Library/Application Support/Google/Chrome")
    } else {
        fake_home.join(".config/google-chrome")
    };
    std::fs::create_dir_all(&default_chrome_dir).unwrap();
    let res_default = cli(&env, COLLIE)
        .env("HOME", fake_home.to_str().unwrap())
        .args([
            "identity",
            "import",
            "default-chrome",
            "--browser",
            "chrome",
            "--executable",
            scripted_chrome.to_str().unwrap(),
            "--domain",
            "shop.example",
            "--json",
        ])
        .run()
        .unwrap();
    assert_ne!(res_default.exit, Some(0));
    let default_err: Value = serde_json::from_str(res_default.stdout.trim()).unwrap();
    assert_eq!(
        default_err["error"]["code"],
        json!("profile_directory_in_use")
    );

    // Canary oracle: zero plaintext cookie leaks in Goodfetch home.
    let hits = canaries.leaks(&[Output::Path(env.home())]).unwrap();
    assert!(
        hits.is_empty(),
        "Saved identity import leaked canaries on disk: {hits:?}"
    );
}
