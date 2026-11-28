//! Personal-data notice shown once per run for fields marked with
//! `"x-goodfetch-personal": true` (ADR 0016; ticket 673).
#![expect(
    clippy::disallowed_methods,
    reason = "a test's own SQL reads, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::Path;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_test_client::{Attached, Hello, HomeLayout};
use goodfetch_test_support::{TestEnv, cli};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::FOXHOUND;
use crate::frontier::crawl_from;
use crate::on_the_calendar;
use crate::site::Site;

fn contact_schema(mark_personal: bool) -> Value {
    let mut email = json!({ "type": "string" });
    let mut phone = json!({ "type": "string" });
    if mark_personal {
        email["x-goodfetch-personal"] = json!(true);
        phone["x-goodfetch-personal"] = json!(true);
    }
    json!({
        "type": "object",
        "required": ["id", "email"],
        "properties": {
            "id": { "type": "string" },
            "email": email,
            "public_name": { "type": "string", "x-goodfetch-personal": false },
            "profile": {
                "type": "object",
                "properties": {
                    "phone": phone,
                },
            },
        },
    })
}

fn committed_count(env: &TestEnv, run_id: &str) -> i64 {
    let connection = Connection::open(HomeLayout::of(env.home()).run_file(run_id)).unwrap();
    connection
        .query_row("SELECT COUNT(*) FROM records", [], |row| row.get(0))
        .unwrap()
}

fn personal_audit_entries(env: &TestEnv) -> Vec<Value> {
    let exported = cli(env, FOXHOUND)
        .args(["audit", "export"])
        .run()
        .unwrap()
        .stdout;
    exported
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["kind"] == json!("personal_data_notice"))
        .collect()
}

fn run_contact_crawl(
    run_id: &str,
    mark_personal: bool,
) -> (i64, Vec<Value>, Vec<String>, Vec<Value>) {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    let report = crawl_from(
        &mut engine,
        &json!({
            "labels": { "page": {} },
            "run_id": run_id,
            "settings": { "site_preferences": { "purpose": "ai-train" } },
            "seeds": [
                { "url": site.page("/a"), "label": "page" },
                { "url": site.page("/b"), "label": "page" },
            ],
            "datasets": [{
                "name": "contacts",
                "schema": contact_schema(mark_personal),
                "key": ["id"],
            }],
        }),
        &mut |inside, handle| {
            let records = if handle["crawl_request"]["url"] == site.page("/a") {
                vec![
                    json!({
                        "id": "1",
                        "email": "ada@example.test",
                        "public_name": "Ada",
                        "profile": { "phone": "+33102030405" },
                    }),
                    json!({
                        "id": "2",
                        "email": "grace@example.test",
                        "public_name": "Grace",
                        "profile": { "phone": "+33102030406" },
                    }),
                ]
            } else {
                vec![json!({
                    "id": "3",
                    "email": "lin@example.test",
                    "public_name": "Lin",
                    "profile": { "phone": "+33102030407" },
                })]
            };
            let pushed = inside.call(
                "crawl_request.push",
                &json!({
                    "run_id": handle["run_id"],
                    "crawl_request_id": handle["crawl_request"]["id"],
                    "lease_epoch": handle["lease_epoch"],
                    "dataset": "contacts",
                    "records": records,
                }),
            );
            assert!(pushed.get("result").is_some(), "{pushed}");
            json!({ "result": { "records": [] } })
        },
    );
    let (_, stderr) = engine.finish_with_stderr();
    let warnings: Vec<Value> = report["result"]["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|warning| {
            warning["code"]
                .as_str()
                .is_some_and(|code| !on_the_calendar(code))
        })
        .cloned()
        .collect();
    let stderr_notices: Vec<String> = stderr
        .lines()
        .filter(|line| line.contains("x-goodfetch-personal"))
        .map(str::to_owned)
        .collect();
    (
        committed_count(&env, run_id),
        warnings,
        stderr_notices,
        personal_audit_entries(&env),
    )
}

/// A run whose Dataset schema marks fields with `"x-goodfetch-personal": true`
/// shows ADR 0016's notice once, however many results carry those fields,
/// names the fields without blocking or delaying the run, and writes one
/// `personal_data_notice` Audit log entry; a run with no field marked personal
/// shows no notice and writes no Audit log entry (ticket 673).
///
/// Covers: story:4-78, operation:crawl.start, operation:crawl_request.push
#[test]
fn a_run_whose_schema_marks_personal_fields_shows_the_notice_once_and_records_one_audit_entry() {
    let (marked_records, marked_warnings, marked_stderr, marked_audit) =
        run_contact_crawl("personal", true);
    let (plain_records, plain_warnings, plain_stderr, plain_audit) =
        run_contact_crawl("plain", false);

    assert_eq!(marked_records, 3);
    assert_eq!(marked_warnings.len(), 1, "{marked_warnings:?}");
    assert_eq!(marked_warnings[0]["code"], json!("personal_data_notice"));
    let warning_message = marked_warnings[0]["message"].as_str().unwrap();
    assert!(
        warning_message.contains("email")
            && warning_message.contains("profile.phone")
            && !warning_message.contains("public_name"),
        "{warning_message}"
    );
    assert_eq!(marked_stderr.len(), 1, "{marked_stderr:?}");
    assert!(
        marked_stderr[0].contains("email") && marked_stderr[0].contains("profile.phone"),
        "{marked_stderr:?}"
    );
    assert_eq!(marked_audit.len(), 1, "{marked_audit:?}");
    assert_eq!(
        (
            &marked_audit[0]["kind"],
            &marked_audit[0]["details"]["fields"],
            &marked_audit[0]["details"]["purpose"],
            &marked_audit[0]["details"]["datasets"],
        ),
        (
            &json!("personal_data_notice"),
            &json!(["email", "profile.phone"]),
            &json!("ai-train"),
            &json!([{ "dataset": "contacts", "fields": ["email", "profile.phone"] }]),
        )
    );

    assert_eq!(plain_records, 3);
    assert!(plain_warnings.is_empty(), "{plain_warnings:?}");
    assert!(plain_stderr.is_empty(), "{plain_stderr:?}");
    assert!(plain_audit.is_empty(), "{plain_audit:?}");
}

/// Recursively searches every regular file under `root` for `needle` bytes,
/// returning the paths that contain it.
fn files_containing(root: &Path, needle: &[u8]) -> Vec<String> {
    let mut hits = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return hits;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            hits.extend(files_containing(&path, needle));
        } else if path.is_file()
            && let Ok(bytes) = std::fs::read(&path)
            && bytes.windows(needle.len()).any(|window| window == needle)
        {
            hits.push(path.display().to_string());
        }
    }
    hits
}

fn audit_entries_of_kind(env: &TestEnv, kind: &str) -> Vec<Value> {
    let exported = cli(env, FOXHOUND)
        .args(["audit", "export"])
        .run()
        .unwrap()
        .stdout;
    exported
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["kind"] == json!(kind))
        .collect()
}

/// An Export's metadata lists the sorted personal columns declared with
/// `"x-goodfetch-personal": true` in its Dataset's JSON Schema; a Dataset with
/// no marked personal fields lists none (ADR 0016; ticket 694).
///
/// Covers: story:4-120, operation:run.export
#[test]
fn an_export_metadata_lists_the_personal_columns_from_the_schema() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    let report = crawl_from(
        &mut engine,
        &json!({
            "labels": { "page": {} },
            "run_id": "export-personal",
            "seeds": [{ "url": site.page("/a"), "label": "page" }],
            "datasets": [
                {
                    "name": "contacts",
                    "schema": contact_schema(true),
                    "key": ["id"],
                },
                {
                    "name": "notes",
                    "schema": {
                        "type": "object",
                        "properties": { "body": { "type": "string" } },
                    },
                },
            ],
        }),
        &mut |inside, handle| {
            let pushed = inside.call(
                "crawl_request.push",
                &json!({
                    "run_id": handle["run_id"],
                    "crawl_request_id": handle["crawl_request"]["id"],
                    "lease_epoch": handle["lease_epoch"],
                    "dataset": "contacts",
                    "records": [{
                        "id": "1",
                        "email": "ada@example.test",
                        "public_name": "Ada",
                        "profile": { "phone": "+33102030405" },
                    }],
                }),
            );
            assert!(pushed.get("result").is_some(), "{pushed}");
            let pushed_note = inside.call(
                "crawl_request.push",
                &json!({
                    "run_id": handle["run_id"],
                    "crawl_request_id": handle["crawl_request"]["id"],
                    "lease_epoch": handle["lease_epoch"],
                    "dataset": "notes",
                    "records": [{ "body": "hello" }],
                }),
            );
            assert!(pushed_note.get("result").is_some(), "{pushed_note}");
            json!({ "result": { "records": [] } })
        },
    );
    assert_eq!(report["result"]["lifecycle"], json!("finished"), "{report}");

    let contacts_export = engine.ask(
        "export-contacts",
        "run.export",
        &json!({ "run_id": "export-personal", "dataset": "contacts", "format": "csv" }),
    );
    let notes_export = engine.ask(
        "export-notes",
        "run.export",
        &json!({ "run_id": "export-personal", "dataset": "notes", "format": "csv" }),
    );
    assert!(engine.finished());

    assert_eq!(
        contacts_export["result"]["personal_columns"],
        json!(["email", "profile.phone"]),
        "{contacts_export}"
    );
    assert!(
        notes_export["result"].get("personal_columns").is_none()
            || notes_export["result"]["personal_columns"] == json!([]),
        "{notes_export}"
    );
}

fn crawl_with_canaries(person: &mut Attached, site: &Site, ada_canary: &str, bob_keep: &str) {
    let report = crawl_from(
        person,
        &json!({
            "labels": { "page": {} },
            "run_id": "erase-run",
            "settings": { "cache": { "mode": "replay" } },
            "seeds": [
                { "url": site.page(&format!("/user?email={ada_canary}")), "label": "page" },
                { "url": site.page(&format!("/user?email={bob_keep}")), "label": "page" },
            ],
            "datasets": [{
                "name": "contacts",
                "schema": contact_schema(true),
                "key": ["id"],
            }],
        }),
        &mut |inside, handle| {
            let url = handle["crawl_request"]["url"].as_str().unwrap_or_default();
            let (id, email, name) = if url.contains(ada_canary) {
                ("1", ada_canary, "Ada")
            } else {
                ("2", bob_keep, "Bob")
            };
            let pushed = inside.call(
                "crawl_request.push",
                &json!({
                    "run_id": handle["run_id"],
                    "crawl_request_id": handle["crawl_request"]["id"],
                    "lease_epoch": handle["lease_epoch"],
                    "dataset": "contacts",
                    "records": [{
                        "id": id,
                        "email": email,
                        "public_name": name,
                    }],
                }),
            );
            assert!(pushed.get("result").is_some(), "{pushed}");
            json!({ "result": { "records": [] } })
        },
    );
    assert_eq!(report["result"]["lifecycle"], json!("finished"), "{report}");

    let checkpoint = person.ask("ckpt", "run.checkpoint", &json!({ "run_id": "erase-run" }));
    assert!(checkpoint.get("result").is_some(), "{checkpoint}");
    let export = person.ask(
        "export",
        "run.export",
        &json!({ "run_id": "erase-run", "dataset": "contacts", "format": "jsonl" }),
    );
    assert!(export.get("result").is_some(), "{export}");
}

fn assert_non_person_surfaces_refused(env: &TestEnv, ada_canary: &str) {
    for surface in [
        Surface::Mcp,
        Surface::SdkTypescript,
        Surface::SdkPython,
        Surface::Rust,
    ] {
        let mut client = Attached::start(env, FOXHOUND);
        let opened = client.open(
            &Hello::needing(Product::Foxhound)
                .surface(surface)
                .id("hello"),
        );
        assert_eq!(opened["result"]["product"], json!("foxhound"), "{opened}");
        let refused = client.ask(
            "erase",
            "run.erase",
            &json!({ "run_id": "erase-run", "value": ada_canary }),
        );
        assert!(client.finished());
        assert_eq!(
            refused["error"]["data"]["code"],
            json!("person_required"),
            "{surface:?}: {refused}"
        );
    }
}

/// `run.erase` for a value removes every matching Record, Checkpoint field,
/// kept Export and Response cache entry of the run so no file under the run
/// folder or Response cache holds the value, keeps unrelated Records intact,
/// writes an Audit log entry without repeating the erased value, and is
/// refused with `person_required` on every non-person surface (ADR 0016;
/// ticket 694).
///
/// Covers: story:4-120, operation:run.erase
#[test]
fn run_erase_removes_matching_records_checkpoint_and_response_cache_and_writes_audit_entry() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let ada_canary = "ada-canary-4120@example.test";
    let bob_keep = "bob-keep-4120@example.test";

    let mut person = crate::opened_by(
        Attached::start(&env, FOXHOUND),
        Hello::needing(Product::Foxhound).surface(Surface::Cli),
    )
    .settings_by_default(&site.settings());
    crawl_with_canaries(&mut person, &site, ada_canary, bob_keep);

    let layout = HomeLayout::of(env.home());
    let run_folder = layout.run_folder("erase-run");
    let cache_folder = layout
        .per_major()
        .join(goodfetch_protocol::home::RESPONSE_CACHE);
    assert!(
        !files_containing(&run_folder, ada_canary.as_bytes()).is_empty(),
        "expected canary in run folder before erase"
    );
    assert!(
        !files_containing(&cache_folder, ada_canary.as_bytes()).is_empty(),
        "expected canary in Response cache before erase"
    );

    assert_non_person_surfaces_refused(&env, ada_canary);

    // Person surface erases across run state, Checkpoint, Exports and Response cache.
    let erased = person.ask(
        "erase",
        "run.erase",
        &json!({ "run_id": "erase-run", "value": ada_canary }),
    );
    let page = person.ask(
        "page",
        "dataset.page",
        &json!({ "run_id": "erase-run", "dataset": "contacts" }),
    );
    let searched = person.ask(
        "search",
        "dataset.search",
        &json!({ "run_id": "erase-run", "dataset": "contacts", "text": "ada-canary-4120" }),
    );
    assert!(person.finished());

    let result = &erased["result"];
    assert!(
        result["records"].as_u64().unwrap_or(0) >= 1
            && result["checkpoint_entries"].as_u64().unwrap_or(0) >= 1
            && result["response_cache_entries"].as_u64().unwrap_or(0) >= 1,
        "{erased}"
    );
    let remaining_records = page["result"]["records"].as_array().unwrap();
    assert_eq!(remaining_records.len(), 1, "{page}");
    assert_eq!(remaining_records[0]["value"]["email"], json!(bob_keep));
    assert_eq!(
        searched["result"]["records"].as_array().unwrap().len(),
        0,
        "{searched}"
    );
    assert_eq!(
        files_containing(&run_folder, ada_canary.as_bytes()),
        Vec::<String>::new(),
        "canary remained in run folder after run.erase"
    );
    assert_eq!(
        files_containing(&cache_folder, ada_canary.as_bytes()),
        Vec::<String>::new(),
        "canary remained in Response cache after run.erase"
    );

    // Kennel (person surface) also succeeds idempotently.
    let mut kennel = Attached::start(&env, FOXHOUND);
    let opened = kennel.open(
        &Hello::needing(Product::Foxhound)
            .surface(Surface::Kennel)
            .id("hello"),
    );
    assert_eq!(opened["result"]["product"], json!("foxhound"), "{opened}");
    let again = kennel.ask(
        "erase-again",
        "run.erase",
        &json!({ "run_id": "erase-run", "value": ada_canary }),
    );
    assert!(kennel.finished());
    assert_eq!(again["result"]["records"], json!(0), "{again}");

    let audit = audit_entries_of_kind(&env, "run_data_erased");
    assert_eq!(audit.len(), 2, "{audit:?}");
    assert_eq!(audit[0]["details"]["operation"], json!("run.erase"));
    assert_eq!(audit[0]["details"]["by_value"], json!(true));
    assert_eq!(audit[0]["run_id"], json!("erase-run"));
    assert!(
        !audit
            .iter()
            .any(|entry| entry.to_string().contains(ada_canary)),
        "Audit log must not repeat the erased personal value: {audit:?}"
    );
}

fn crawl_contacts_with_nested_phone_and_rejected(
    person: &mut Attached,
    site: &Site,
    ada_email: &str,
    ada_phone: &str,
    rejected_email: &str,
) {
    let report = crawl_from(
        person,
        &json!({
            "labels": { "page": {} },
            "run_id": "erase-key-run",
            "seeds": [{ "url": site.page("/a"), "label": "page" }],
            "datasets": [{
                "name": "contacts",
                "schema": {
                    "type": "object",
                    "required": ["id"],
                    "properties": {
                        "id": { "type": "string" },
                        "email": { "type": "string", "x-goodfetch-personal": true },
                        "public_name": { "type": "string" },
                        "profile": {
                            "type": "object",
                            "properties": {
                                "phone": { "type": "string", "x-goodfetch-personal": true },
                                "city": { "type": "string" },
                            },
                        },
                    },
                },
                "key": ["id"],
            }],
        }),
        &mut |inside, handle| {
            let pushed = inside.call(
                "crawl_request.push",
                &json!({
                    "run_id": handle["run_id"],
                    "crawl_request_id": handle["crawl_request"]["id"],
                    "lease_epoch": handle["lease_epoch"],
                    "dataset": "contacts",
                    "records": [
                        {
                            "id": "1",
                            "email": ada_email,
                            "public_name": "Ada",
                            "profile": { "phone": ada_phone, "city": "Paris" },
                        },
                        {
                            "id": "2",
                            "public_name": "NoContact",
                            "profile": { "city": "Lyon" },
                        },
                        {
                            "email": rejected_email,
                            "profile": { "phone": "+33999999991" },
                        },
                    ],
                }),
            );
            assert_eq!(
                pushed["error"]["data"]["code"],
                json!("record_invalid"),
                "{pushed}"
            );
            json!({ "result": { "records": [] } })
        },
    );
    assert_eq!(report["result"]["lifecycle"], json!("finished"), "{report}");
}

fn assert_label_refuses_dataset_retention(person: &mut Attached, site: &Site) {
    let label_refused = person.ask(
        "label-retention",
        "crawl.start",
        &json!({
            "labels": {
                "page": {
                    "settings": { "datasets": { "contacts": { "retention": 10_000 } } }
                }
            },
            "run_id": "retention-on-label",
            "seeds": [{ "url": site.page("/a"), "label": "page" }],
            "datasets": [{ "name": "contacts" }],
        }),
    );
    assert_eq!(
        (
            &label_refused["error"]["data"]["code"],
            &label_refused["error"]["data"]["key"]
        ),
        (
            &json!("setting_not_applicable"),
            &json!("datasets.contacts.retention")
        ),
        "{label_refused}"
    );
}

/// `run.erase` handles `key`-only erasure (collecting erased personal values
/// without rewriting JSON property names), dotted `key` paths
/// (`profile.phone`), `rejected_records`, and `<run>/bodies/` files, while
/// `datasets.<name>.retention` on a Label is refused with
/// `setting_not_applicable` (ADR 0016; ticket 694).
///
/// Covers: story:4-120, operation:run.erase, setting:datasets, error:setting_not_applicable
#[test]
fn run_erase_handles_key_only_dotted_key_rejected_records_and_bodies_and_label_refuses_retention() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let ada_email = "ada-keyonly-4120@example.test";
    let ada_phone = "+33102039999";
    let rejected_email = "rejected-4120@example.test";

    let mut person = crate::opened_by(
        Attached::start(&env, FOXHOUND),
        Hello::needing(Product::Foxhound).surface(Surface::Cli),
    )
    .settings_by_default(&site.settings());

    assert_label_refuses_dataset_retention(&mut person, &site);

    crawl_contacts_with_nested_phone_and_rejected(
        &mut person,
        &site,
        ada_email,
        ada_phone,
        rejected_email,
    );

    let run_folder = HomeLayout::of(env.home()).run_folder("erase-key-run");
    let bodies_dir = run_folder.join("bodies");
    let trace_dir = run_folder.join("trace");
    std::fs::create_dir_all(&bodies_dir).unwrap();
    std::fs::create_dir_all(&trace_dir).unwrap();
    let body_file = bodies_dir.join("body-canary");
    std::fs::write(
        &body_file,
        format!("<html>phone {ada_phone} email {ada_email}</html>"),
    )
    .unwrap();
    let trace_file = trace_dir.join("extra.jsonl");
    std::fs::write(
        &trace_file,
        format!("{{\"email\":\"{ada_email}\",\"profile\":{{\"phone\":\"{ada_phone}\",\"city\":\"Paris\"}}}}\n"),
    )
    .unwrap();

    // 1. Dotted key-only erase (`profile.phone`) erases record 1 + rejected record,
    //    scrubs `<run>/bodies/` and removes `profile.phone` without corrupting keys.
    let phone_erased = person.ask(
        "erase-phone",
        "run.erase",
        &json!({ "run_id": "erase-key-run", "key": "profile.phone" }),
    );
    let page_after = person.ask(
        "page-after",
        "dataset.page",
        &json!({ "run_id": "erase-key-run", "dataset": "contacts" }),
    );
    assert!(person.finished());

    assert_eq!(
        (
            &phone_erased["result"]["records"],
            &phone_erased["result"]["rejected_records"],
        ),
        (&json!(1), &json!(1)),
        "{phone_erased}"
    );
    let remaining = page_after["result"]["records"].as_array().unwrap();
    assert_eq!(remaining.len(), 1, "{page_after}");
    assert_eq!(remaining[0]["value"]["public_name"], json!("NoContact"));
    assert_eq!(
        files_containing(&run_folder, ada_phone.as_bytes()),
        Vec::<String>::new()
    );
    assert_eq!(
        files_containing(&run_folder, rejected_email.as_bytes()),
        Vec::<String>::new()
    );
    let body_after = std::fs::read_to_string(&body_file).unwrap();
    assert!(!body_after.contains(ada_phone) && body_after.contains("phone [erased]"));
    let trace_after: Value =
        serde_json::from_str(std::fs::read_to_string(&trace_file).unwrap().trim()).unwrap();
    assert_eq!(
        trace_after,
        json!({ "email": ada_email, "profile": { "city": "Paris" } })
    );

    // 2. Top-level key-only erase (`email`) removes `email` structurally from trace
    //    and scrubs its collected value from `<run>/bodies/` without rewriting keys.
    let mut person2 = crate::opened_by(
        Attached::start(&env, FOXHOUND),
        Hello::needing(Product::Foxhound).surface(Surface::Cli),
    );
    let email_erased = person2.ask(
        "erase-email",
        "run.erase",
        &json!({ "run_id": "erase-key-run", "key": "email" }),
    );
    assert!(person2.finished());
    assert!(email_erased.get("result").is_some(), "{email_erased}");
    let trace_final: Value =
        serde_json::from_str(std::fs::read_to_string(&trace_file).unwrap().trim()).unwrap();
    assert_eq!(trace_final, json!({ "profile": { "city": "Paris" } }));
    assert_eq!(
        files_containing(&run_folder, ada_email.as_bytes()),
        Vec::<String>::new()
    );
}
