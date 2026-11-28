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

use goodfetch_test_client::HomeLayout;
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
