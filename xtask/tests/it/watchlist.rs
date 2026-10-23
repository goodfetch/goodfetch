//! The watchlist job (story 61): run on hand-written copies of both
//! watchlists and a coverage file, it opens one `watchlist` issue per vendor
//! no block-detection rule covers through a recorded stand-in of GitHub's
//! interface, never a second one for a vendor that has one, open or closed,
//! and writes nothing into the workspace.
//!
//! Covers: story:8-61

use serde_json::{Value, json};

use crate::github::{GitHub, Request};
use crate::workspace::{Outcome, Workspace};

const ISSUES: &str = "/repos/goodfetch/goodfetch/issues";

/// GitHub holding `existing` as the `watchlist` issues.
fn github(existing: &str) -> GitHub {
    GitHub::serving(&[
        (
            &format!("{ISSUES}?labels=watchlist&state=all&per_page=100&page=1"),
            existing,
        ),
        (&format!("POST {ISSUES}"), r#"{ "number": 41 }"#),
    ])
}

fn watchlist(workspace: &Workspace, github: &GitHub, extra: &[&str]) -> Outcome {
    let mut args = vec![
        "watchlist",
        "--webappanalyzer",
        "webappanalyzer",
        "--antibot-detector",
        "antibot-detector",
        "--repository",
        "goodfetch/goodfetch",
        "--api-url",
        github.url(),
    ];
    args.extend_from_slice(extra);
    workspace.xtask(&args)
}

/// The marker line of each issue the job opened, after checking its labels
/// and that it names no milestone.
fn opened(writes: &[Request]) -> Vec<String> {
    writes
        .iter()
        .map(|request| {
            assert_eq!(
                (request.method.as_str(), request.path.as_str()),
                ("POST", ISSUES)
            );
            let body: Value = serde_json::from_str(&request.body).unwrap();
            assert_eq!(body["labels"], json!(["watchlist", "needs-triage"]));
            assert_eq!(body.get("milestone"), None);
            body["body"]
                .as_str()
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .to_owned()
        })
        .collect()
}

fn marker(key: &str) -> String {
    format!("<!-- goodfetch-results: watchlist {key} -->")
}

#[test]
fn each_vendor_no_rule_covers_gets_one_issue_and_the_workspace_is_unchanged() {
    let workspace = Workspace::from_fixture("watchlist");
    workspace.commit_all();
    let github = github("[]");

    let outcome = watchlist(&workspace, &github, &[]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let writes = github.writes();
    assert_eq!(
        opened(&writes),
        ["akamai", "datadome", "geetest", "kasada"].map(marker)
    );
    let datadome: Value = serde_json::from_str(&writes[1].body).unwrap();
    assert_eq!(
        datadome["title"],
        "Block detection: no rule covers DataDome, which a watchlist lists"
    );
    let text = datadome["body"].as_str().unwrap();
    assert!(
        text.contains("- webappanalyzer (GPL-3.0) lists `DataDome`."),
        "{text}"
    );
    assert!(
        text.contains("- Scrapfly's Antibot-Detector (NPOSL-3.0) lists `datadome`."),
        "{text}"
    );
    assert_eq!(workspace.git(&["status", "--porcelain"]), "");
}

#[test]
fn a_rerun_opens_none_for_a_vendor_whose_issue_is_open_or_closed() {
    let workspace = Workspace::from_fixture("watchlist");
    let existing = json!([
        { "number": 7, "state": "open", "closed_at": null, "body": format!("{}\nopen", marker("datadome")) },
        { "number": 8, "state": "closed", "closed_at": "2026-10-01T00:00:00Z", "body": format!("{}\ndeclined", marker("kasada")) },
    ])
    .to_string();
    let github = github(&existing);

    let outcome = watchlist(&workspace, &github, &[]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(opened(&github.writes()), ["akamai", "geetest"].map(marker));
}

#[test]
fn a_run_opens_at_most_its_limit_and_says_how_many_wait() {
    let workspace = Workspace::from_fixture("watchlist");
    let github = github("[]");

    let outcome = watchlist(&workspace, &github, &["--limit", "1"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(opened(&github.writes()), [marker("akamai")]);
    assert!(
        outcome.stdout.contains("3 more vendors"),
        "{}",
        outcome.stdout
    );
}
