//! The model manifest's retirement check (ADR 0065), run with the daily
//! test-health job: from 30 days before the retirement its provider
//! announced, a model `test-support/fixtures/models.toml` pins opens one
//! `needs-triage` issue through a recorded stand-in of GitHub's interface,
//! and each later run updates it; a model further off opens none, and a
//! manifest outside the format is refused.

use serde_json::{Value, json};

use crate::github::{GitHub, Request};
use crate::workspace::{Outcome, Workspace};

const ISSUES: &str = "/repos/goodfetch/goodfetch/issues";
const MANIFEST: &str = "test-support/fixtures/models.toml";
const TODAY: &str = "2026-11-24";

/// A manifest whose small model of both formats, `openai/gpt-9-nano`,
/// retires on `retires`, and whose larger ones announce no retirement.
fn manifest(retires: &str) -> String {
    let small = format!(
        "small = {{ model = \"openai/gpt-9-nano\", retires = {retires}, source = \"https://developers.openai.com/api/docs/deprecations\" }}"
    );
    format!(
        "[openai-compatible]\n{small}\nlarger = {{ model = \"openai/gpt-9.2\", source = \"https://developers.openai.com/api/docs/deprecations\" }}\n\n\
         [anthropic-messages]\n{small}\nlarger = {{ model = \"openai/gpt-9.1\", source = \"https://developers.openai.com/api/docs/deprecations\" }}\n\n\
         [gemini]\nwaits = \"ADR 0076: the gateway does not serve Gemini's native format\"\n"
    )
}

/// A workspace holding `manifest`, committed: the check reads its history
/// for the commit that pinned a model.
fn workspace(manifest: &str) -> (Workspace, String) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(MANIFEST, manifest.as_bytes());
    let commit = workspace.commit_all();
    (workspace, commit)
}

/// GitHub holding `existing` as the `needs-triage` issues, with `more`
/// routes, and no merged pull request for `commit`.
fn github(commit: &str, existing: &str, more: &[(String, &str)]) -> GitHub {
    let mut routes = vec![
        (
            format!("{ISSUES}?labels=needs-triage&state=all&per_page=100&page=1"),
            existing,
        ),
        (format!("POST {ISSUES}"), r#"{ "number": 41, "id": 9001 }"#),
        (format!("PATCH {ISSUES}/7"), r#"{ "number": 7 }"#),
    ];
    routes.extend(more.iter().cloned());
    if !more.iter().any(|(path, _)| path.contains("/pulls")) {
        routes.push((
            format!("/repos/goodfetch/goodfetch/commits/{commit}/pulls"),
            "[]",
        ));
    }
    let routes: Vec<(&str, &str)> = routes
        .iter()
        .map(|(path, body)| (path.as_str(), *body))
        .collect();
    GitHub::serving(&routes)
}

fn check(workspace: &Workspace, github: &GitHub) -> Outcome {
    workspace.xtask(&[
        "models",
        "check",
        "--today",
        TODAY,
        "--repository",
        "goodfetch/goodfetch",
        "--api-url",
        github.url(),
    ])
}

const MARKER: &str = "<!-- goodfetch-results: needs-triage model-retirement openai/gpt-9-nano -->";

/// Each write's method and path, with its JSON fields but the body's text.
fn writes(writes: &[Request]) -> Vec<(String, String, Value)> {
    writes
        .iter()
        .map(|write| {
            let mut fields: Value = serde_json::from_str(&write.body).unwrap();
            if let Some(fields) = fields.as_object_mut() {
                fields.remove("body");
            }
            (write.method.clone(), write.path.clone(), fields)
        })
        .collect()
}

fn text(write: &Request) -> String {
    let fields: Value = serde_json::from_str(&write.body).unwrap();
    fields["body"].as_str().unwrap().to_owned()
}

#[test]
fn a_model_retiring_in_30_days_opens_one_issue_for_both_formats_that_pin_it() {
    let (workspace, commit) = workspace(&manifest("2026-12-24"));
    let github = github(&commit, "[]", &[]);

    let outcome = check(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let written = github.writes();
    assert_eq!(
        writes(&written),
        [(
            "POST".to_owned(),
            ISSUES.to_owned(),
            json!({
                "title": "Model retirement: openai/gpt-9-nano retires on 2026-12-24",
                "labels": ["needs-triage"],
            })
        )]
    );
    let body = text(&written[0]);
    assert_eq!(body.lines().next(), Some(MARKER));
    assert!(
        body.contains(
            "pins as the small model of the OpenAI-compatible format and of the Anthropic Messages format, retires on 2026-12-24, in 30 days"
        ),
        "{body}"
    );
    assert!(
        body.contains("https://developers.openai.com/api/docs/deprecations"),
        "{body}"
    );
    assert!(outcome.stdout.contains("opened #41"), "{}", outcome.stdout);
}

#[test]
fn a_model_retiring_in_31_days_opens_none() {
    let (workspace, commit) = workspace(&manifest("2026-12-25"));
    let github = github(&commit, "[]", &[]);

    let outcome = check(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(github.writes(), []);
    assert!(
        outcome
            .stdout
            .contains("No pinned model retires within 30 days of 2026-11-24"),
        "{}",
        outcome.stdout
    );
}

#[test]
fn a_second_run_updates_the_same_issue() {
    let (workspace, commit) = workspace(&manifest("2026-12-20"));
    let existing = json!([{
        "number": 7,
        "state": "open",
        "closed_at": null,
        "body": format!("{MARKER}\nretires on 2026-12-20, in 27 days"),
    }])
    .to_string();
    let github = github(&commit, &existing, &[]);

    let outcome = check(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let written = github.writes();
    assert_eq!(
        writes(&written),
        [("PATCH".to_owned(), format!("{ISSUES}/7"), json!({}))]
    );
    assert!(
        text(&written[0]).contains("in 26 days"),
        "{}",
        text(&written[0])
    );
}

/// The commit that pinned the model is ticket 411's, of spec 327, in
/// milestone 2 (ADR 0136).
#[test]
fn the_issue_of_a_model_a_ticket_pinned_is_a_sub_issue_of_its_spec_in_its_milestone() {
    let (workspace, commit) = workspace(&manifest("2026-12-01"));
    let github = github(
        &commit,
        "[]",
        &[
            (
                format!("/repos/goodfetch/goodfetch/commits/{commit}/pulls"),
                r#"[{ "number": 3700, "merged_at": "2026-11-24T08:00:00Z", "body": "Closes #411\n" }]"#,
            ),
            (
                format!("{ISSUES}/411"),
                r#"{ "number": 411, "milestone": { "number": 2 }, "parent_issue_url": "https://api.github.com/repos/goodfetch/goodfetch/issues/327" }"#,
            ),
            (
                format!("POST {ISSUES}/327/sub_issues"),
                r#"{ "number": 327 }"#,
            ),
        ],
    );

    let outcome = check(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        writes(&github.writes()),
        [
            (
                "POST".to_owned(),
                ISSUES.to_owned(),
                json!({
                    "title": "Model retirement: openai/gpt-9-nano retires on 2026-12-01",
                    "labels": ["needs-triage"],
                    "milestone": 2,
                })
            ),
            (
                "POST".to_owned(),
                format!("{ISSUES}/327/sub_issues"),
                json!({ "sub_issue_id": 9001 })
            ),
        ]
    );
}

#[test]
fn a_manifest_outside_the_format_is_refused() {
    let unversioned = manifest("2027-04-01").replace("openai/gpt-9.2", "openai/gpt-latest");
    let no_larger = manifest("2027-04-01").replacen(
        "larger = { model = \"openai/gpt-9.2\", source = \"https://developers.openai.com/api/docs/deprecations\" }\n",
        "",
        1,
    );
    let timed = manifest("2027-04-01T00:00:00Z");
    let cases = [
        (unversioned.as_str(), "openai/gpt-latest"),
        (no_larger.as_str(), "openai-compatible"),
        (timed.as_str(), "2027-04-01T00:00:00Z"),
    ];
    for (manifest, named) in cases {
        let (workspace, commit) = workspace(manifest);
        let github = github(&commit, "[]", &[]);

        let outcome = check(&workspace, &github);

        assert_ne!(outcome.code, 0, "{manifest}");
        assert!(
            outcome.stderr.contains(named),
            "{named}: {}",
            outcome.stderr
        );
        assert_eq!(github.writes(), []);
    }
}
