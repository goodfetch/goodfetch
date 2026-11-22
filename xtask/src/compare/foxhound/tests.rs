use super::settings;
use crate::compare::matrix::{self, Scenario};
use crate::rss::crawl::Site;
use serde_json::{Value, json};

const SITE: Site<'static> = Site {
    address: "127.0.0.1:8080",
    pages: 10_000,
    concurrency: 16,
};

/// The polite scenario's main row forces the `http` tier, as the other
/// tools fetch; its `auto` row leaves `fetch.tier` at its default, so the
/// crawl samples each domain in a browser (#2283).
#[test]
fn the_polite_auto_row_leaves_the_tier_at_its_default() {
    let cell = matrix::cell(Scenario::Polite, "foxhound").expect("the polite scenario's row");
    let forced = settings(&SITE, cell, ("full", None), false);
    let auto = settings(&SITE, cell, ("full", None), true);

    assert_eq!(forced["fetch"]["tier"], "http");
    assert!(auto["fetch"].get("tier").is_none());
    assert_eq!(auto["politeness"], forced["politeness"]);
}

/// Scenario 1's first cell runs the default `batch` and the default shared
/// Politeness, keeping the scenario's other switches (#2175).
#[test]
fn scenario_1_runs_batch_and_the_shared_politeness() {
    let cell = matrix::cell(Scenario::Raw, "foxhound").expect("scenario 1's first row");
    let settings = settings(&SITE, cell, ("batch", None), false);

    assert_eq!(
        (
            &settings["run_state"],
            &settings["politeness"],
            &settings["fetch"],
            &settings["site_preferences"],
        ),
        (
            &json!({ "durability": "batch" }),
            &json!({ "domain": { "delay": 0, "concurrency": 16 } }),
            &json!({ "tier": "http", "block_classification": false }),
            &json!({ "respect_robots_txt": false }),
        )
    );
}

/// Each default-cost cell runs scenario 1's first cell's settings with its
/// one default on, so its count less the first cell's is that default's
/// instructions (#2438).
#[rstest::rstest]
#[case::robots_txt("foxhound robots.txt on", "site_preferences", "respect_robots_txt")]
#[case::learning("foxhound learning on", "frontier", "learning")]
#[case::provenance_log("foxhound provenance log on", "provenance", "log")]
#[case::block_classification("foxhound block classification on", "fetch", "block_classification")]
#[case::records("foxhound records on", "dataset", "records")]
fn each_default_cost_cell_runs_the_first_cell_with_its_one_default_on(
    #[case] name: &str,
    #[case] section: &str,
    #[case] key: &str,
) {
    let first = matrix::cell(Scenario::Raw, "foxhound").expect("scenario 1's first row");
    let first = settings(&SITE, first, ("batch", None), false);
    let cell = matrix::cell(Scenario::Raw, name).expect("the default-cost cell");
    let mut expected = first.clone();
    expected[section][key] = json!(true);

    let run = settings(&SITE, cell, ("batch", None), false);

    assert_eq!((&first[section][key], run), (&json!(false), expected));
}

/// The cell at the catalog's defaults sets nothing but what names the
/// crawl's site and its pace; its second row adds the other way of
/// `fetch.http_from`, `browser`, the default being `learned` (#2175).
#[test]
fn the_cell_at_the_defaults_sets_only_the_site_and_the_pace() {
    let in_force = matrix::cell(Scenario::Raw, "foxhound defaults").expect("the defaults row");
    let browser =
        matrix::cell(Scenario::Raw, "foxhound defaults browser").expect("the browser row");
    let pace = json!({ "domain": { "delay": 0, "concurrency": 16 } });
    let keys = |settings: &Value| -> Vec<String> {
        let mut keys: Vec<String> = settings
            .as_object()
            .expect("settings are an object")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    };
    let in_force = settings(&SITE, in_force, ("batch", None), true);
    let browser = settings(&SITE, browser, ("batch", None), true);

    assert_eq!(
        (
            keys(&in_force),
            &in_force["politeness"],
            keys(&browser),
            &browser["fetch"]
        ),
        (
            vec![
                "egress".to_owned(),
                "network".to_owned(),
                "politeness".to_owned()
            ],
            &pace,
            vec![
                "egress".to_owned(),
                "fetch".to_owned(),
                "network".to_owned(),
                "politeness".to_owned()
            ],
            &json!({ "http_from": "browser" }),
        )
    );
}
