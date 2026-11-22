//! The release table the version check grades browser builds against (ADR
//! 0035): `xtask browser-releases` writes each Chrome and Firefox major from
//! 2018 on with the day it reached Stable, from stand-ins of Chromium Dash
//! and Mozilla's release calendar.

use crate::github::GitHub;
use crate::workspace::Workspace;

const TABLE_FILE: &str = "crates/goodfetch-page/src/version_check/browser-releases.toml";

const CHROME_SCHEDULE: &str = "/fetch_milestone_schedule?offset=-100&n=112";

/// Chrome 63 before 2018, 82 skipped as Chrome skipped it, and a milestone
/// not yet scheduled.
const SCHEDULE: &str = r#"{"mstones": [
    {"mstone": 63, "stable_date": "2017-12-05T00:00:00", "branch_point": "2017-10-19T00:00:00"},
    {"mstone": 81, "stable_date": "2020-04-07T00:00:00"},
    {"mstone": 83, "stable_date": "2020-05-19T00:00:00"},
    {"mstone": 160, "stable_date": "2027-01-05T00:00:00"},
    {"mstone": 161, "stable_date": null}
]}"#;

/// Firefox 57 before 2018, a dot release and an unnumbered one.
const SHIPPED: &str = r#"{"57.0": "2017-11-14", "58.0": "2018-01-23", "58.0.1": "2018-01-29", "160.0": "2026-11-10"}"#;
const SCHEDULED: &str = r#"{"161.0": "2026-12-08", "162.0.x": "2026-12-15"}"#;

fn calendars(schedule: &str) -> (GitHub, GitHub) {
    (
        GitHub::serving(&[(CHROME_SCHEDULE, schedule)]),
        GitHub::serving(&[
            ("/api/firefox/releases/", SHIPPED),
            ("/api/firefox/releases/future/", SCHEDULED),
        ]),
    )
}

fn write_table(
    workspace: &Workspace,
    chromium_dash: &GitHub,
    mozilla: &GitHub,
) -> crate::workspace::Outcome {
    workspace.xtask(&[
        "browser-releases",
        "--chromium-dash",
        chromium_dash.url(),
        "--firefox-calendar",
        mozilla.url(),
    ])
}

#[test]
fn each_major_from_2018_is_written_with_its_stable_day() {
    let (chromium_dash, mozilla) = calendars(SCHEDULE);
    let workspace = Workspace::from_fixture("empty");

    let outcome = write_table(&workspace, &chromium_dash, &mozilla);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        workspace.read_text(TABLE_FILE),
        "# The release table the version check grades browser builds against\n\
         # (ADR 0035): each major of Chrome (Chromium Dash) and Firefox\n\
         # (Mozilla's release calendar) with the day it reached Stable,\n\
         # scheduled majors included. Written by `xtask browser-releases`;\n\
         # never edited by hand.\n\
         \n[[chrome]]\nmajor = 81\nstable = \"2020-04-07\"\n\
         \n[[chrome]]\nmajor = 83\nstable = \"2020-05-19\"\n\
         \n[[chrome]]\nmajor = 160\nstable = \"2027-01-05\"\n\
         \n[[firefox]]\nmajor = 58\nstable = \"2018-01-23\"\n\
         \n[[firefox]]\nmajor = 160\nstable = \"2026-11-10\"\n\
         \n[[firefox]]\nmajor = 161\nstable = \"2026-12-08\"\n"
    );
}

#[test]
fn a_schedule_whose_dates_fall_as_majors_rise_writes_nothing() {
    let (chromium_dash, mozilla) = calendars(
        r#"{"mstones": [
            {"mstone": 100, "stable_date": "2022-05-03T00:00:00"},
            {"mstone": 101, "stable_date": "2022-04-26T00:00:00"}
        ]}"#,
    );
    let workspace = Workspace::from_fixture("empty");

    let outcome = write_table(&workspace, &chromium_dash, &mozilla);

    assert_ne!(outcome.code, 0);
    assert_eq!(workspace.read(TABLE_FILE), None);
}
