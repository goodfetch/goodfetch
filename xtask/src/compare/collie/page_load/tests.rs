use serde_json::json;

use super::*;

#[test]
fn the_cell_runs_seven_rows_on_each_of_its_two_pages() {
    let cells = cells();
    assert_eq!(cells.len(), 14);
    for page in PAGES {
        let rows: Vec<Row> = cells
            .iter()
            .filter(|(of, _)| *of == page)
            .map(|(_, row)| *row)
            .collect();
        assert_eq!(rows, Row::ALL, "{}", page.name());
    }
    assert_eq!(
        Row::ALL.map(Row::number),
        [1, 2, 3, 4, 5, 6, 7],
        "the report numbers the rows in order"
    );
    assert_eq!(
        Row::ALL.map(Row::tool),
        [
            Contender::Collie,
            Contender::Collie,
            Contender::Collie,
            Contender::Collie,
            Contender::Collie,
            Contender::Playwright,
            Contender::Playwright,
        ]
    );
    assert_eq!(
        Row::ALL.map(Row::routed),
        [false, false, false, false, false, false, true]
    );
    assert_eq!(
        (LoadPage::Plain.path(), LoadPage::Plain.own()),
        ("/plain", 150)
    );
    assert_eq!((LoadPage::Ads.path(), LoadPage::Ads.own()), ("/ads", 120));
}

#[test]
fn each_collie_row_turns_off_its_defaults_and_maps_the_page_and_ad_hosts_to_loopback() {
    let hosts = vec!["ib.adnxs.com".to_owned(), "u.openx.net".to_owned()];
    let settings = |row| -> toml::Table {
        let text = configuration(4000, &hosts, row).unwrap();
        let table: toml::Table = text.parse().unwrap();
        table["settings"].as_table().unwrap().clone()
    };
    let defaults = settings(Row::Defaults);
    for host in [HOST, "ib.adnxs.com", "u.openx.net"] {
        assert_eq!(
            defaults["network"]["resolve"][host],
            toml::Value::Array(vec!["127.0.0.1".into()]),
            "{host}"
        );
    }
    for absent in ["live_view", "consent", "filter_lists", "security"] {
        assert!(!defaults.contains_key(absent), "{absent}");
    }
    let hiding = settings(Row::LiveHidingOff);
    assert_eq!(hiding["live_view"]["hiding"].as_str(), Some("off"));
    assert!(!hiding.contains_key("consent"));
    let consent = settings(Row::ConsentOff);
    assert_eq!(consent["live_view"]["hiding"].as_str(), Some("off"));
    assert_eq!(consent["consent"]["handling"].as_str(), Some("off"));
    let lists = settings(Row::FilterListsOff);
    assert_eq!(lists["live_view"]["hiding"].as_str(), Some("off"));
    for list in ["ads", "trackers", "annoyances"] {
        assert_eq!(lists["filter_lists"][list].as_bool(), Some(false), "{list}");
    }
    let relay = settings(Row::EgressRelayOff);
    assert_eq!(
        relay["security"]["raw_cdp_egress_proxy"].as_bool(),
        Some(false)
    );
    assert!(!relay.contains_key("live_view"));
    assert_eq!(
        resolver_rules(&hosts),
        "MAP page-load.test 127.0.0.1, MAP ib.adnxs.com 127.0.0.1, MAP u.openx.net 127.0.0.1"
    );
}

#[test]
fn the_ad_hosts_are_those_of_the_absolute_subresources_each_once() {
    let page = "<script src=\"/assets/script-1.js\"></script>\n\
        <script src=\"http://ib.adnxs.com:4000/ad/1.js\"></script>\n\
        <img src=\"http://ib.adnxs.com:4000/ad/1.gif\">\n\
        <img src=\"http://u.openx.net:4000/ad/2.gif\">";
    assert_eq!(ad_hosts(page), ["ib.adnxs.com", "u.openx.net"]);
}

#[test]
fn a_load_counts_its_own_subresources_once_and_every_request_it_sent() {
    let request = |host: &str, target: &str, status: u16| json!({"host": host, "target": target, "status": status, "method": "GET"});
    let observations = json!({"requests": [
        request("page-load.test:4000", "/ads", 200),
        request("page-load.test:4000", "/assets/script-1.js", 200),
        // Asked twice, counted once among the own subresources.
        request("page-load.test:4000", "/assets/script-1.js", 200),
        request("page-load.test:4000", "/assets/image-1.gif", 200),
        request("page-load.test:4000", "/favicon.ico", 404),
        request("page-load.test:4000", "/assets/missing.png", 404),
        request("ib.adnxs.com:4000", "/ad/1.js", 200),
        // Another host's request is none of the page's.
        request("1.actionability.test:4000", "/", 200),
    ]});
    assert_eq!(counted(&observations), (2, 7));
    assert_eq!(counted(&json!({})), (0, 0));
}

/// A trial of `row` on `page` that did its page's work in `wall` seconds.
fn load(page: LoadPage, row: Row, number: u32, wall: f64) -> LoadTrial {
    LoadTrial {
        page,
        row,
        number,
        load: None,
        status: Some(200),
        read: Some("digest of the content".to_owned()),
        own: page.own(),
        sent: page.own() + 1,
        cpu_seconds: wall / 2.0,
        wall_seconds: wall,
        peak_bytes: 300 << 20,
        // Collie's engine counts its paused requests and CDP commands;
        // Playwright has neither.
        paused: (row.tool() == Contender::Collie).then(|| page.own() + 1),
        commands: (row.tool() == Contender::Collie).then(|| 400 + u64::from(number)),
        retried: None,
    }
}

/// Every trial of a run of `trials` of each row on each page, each row's
/// wall time on each page from `wall`; the Collie scenario's tests' full
/// page-load cell too.
pub(in crate::compare::collie) fn every(
    trials: u32,
    wall: impl Fn(LoadPage, Row) -> f64,
) -> Vec<LoadTrial> {
    cells()
        .into_iter()
        .flat_map(|(page, row)| {
            let wall = wall(page, row);
            (1..=trials).map(move |number| load(page, row, number, wall))
        })
        .collect()
}

fn kinds(findings: &[Finding]) -> Vec<(&str, &str)> {
    findings
        .iter()
        .map(|finding| (finding.kind, finding.subject.as_str()))
        .collect()
}

#[test]
fn trials_of_the_same_work_pass_though_the_filter_lists_blocked_requests() {
    let mut loads = every(2, |_, _| 0.5);
    for trial in &mut loads {
        // Row 4 loads the ads the other Collie rows block.
        if trial.page == LoadPage::Ads && trial.row == Row::FilterListsOff {
            trial.sent += 30;
        }
    }
    assert!(check(2, &loads, &[]).is_empty());
}

#[test]
fn a_load_of_other_work_is_refused_and_named() {
    let mut loads = every(2, |_, _| 0.5);
    let at = |loads: &[LoadTrial], page, row, number| {
        loads
            .iter()
            .position(|trial| trial.page == page && trial.row == row && trial.number == number)
            .unwrap()
    };
    // An own subresource missing.
    let index = at(&loads, LoadPage::Plain, Row::Defaults, 1);
    loads[index].own = 149;
    // Another page answered.
    let index = at(&loads, LoadPage::Ads, Row::Playwright, 2);
    loads[index].status = Some(404);
    // Another text read.
    let index = at(&loads, LoadPage::Ads, Row::ConsentOff, 1);
    loads[index].read = Some("another digest".to_owned());
    // Nothing the Proving ground received.
    let index = at(&loads, LoadPage::Plain, Row::EgressRelayOff, 2);
    loads[index].sent = 0;
    // A row short of its trials.
    let index = at(&loads, LoadPage::Plain, Row::PlaywrightRouted, 2);
    loads.remove(index);
    assert_eq!(
        kinds(&check(2, &loads, &[])),
        [
            ("trial_count", "collie page_load plain row 7"),
            ("different_work", "collie page_load plain row 1 trial 1"),
            ("nothing_counted", "collie page_load plain row 5 trial 2"),
            ("different_work", "collie page_load ads row 3 trial 1"),
            ("different_work", "collie page_load ads row 6 trial 2"),
        ]
    );
}

#[test]
fn a_row_stopped_twice_is_not_measured_and_owes_no_trial() {
    let mut loads = every(2, |_, _| 0.5);
    loads.retain(|trial| !(trial.page == LoadPage::Ads && trial.row == Row::Defaults));
    let stopped = [LoadStopped {
        page: LoadPage::Ads,
        row: Row::Defaults,
        number: 1,
        reason: "the tool ran past 300 s".to_owned(),
    }];
    assert!(check(2, &loads, &stopped).is_empty());
    let section = section(&loads, &stopped);
    assert_eq!(section.target.get(&LoadPage::Ads), Some(&None));
    assert_eq!(section.target_held, None);
    let report = serde_json::to_value(&section).unwrap();
    let ads_defaults = &report["rows"][7];
    assert_eq!(ads_defaults["page"], "ads");
    assert_eq!(ads_defaults["row"], "defaults");
    assert_eq!(ads_defaults["wall_seconds"]["status"], "not_measured");
    assert_eq!(ads_defaults["paused"]["status"], "not_measured");
}

#[test]
fn a_collie_trial_without_its_engines_counts_leaves_them_not_measured() {
    let mut loads = every(2, |_, _| 0.5);
    for trial in &mut loads {
        if trial.page == LoadPage::Plain && trial.row == Row::Defaults && trial.number == 2 {
            trial.paused = None;
            trial.commands = None;
        }
    }
    let section = section(&loads, &[]);
    let report = serde_json::to_value(&section).unwrap();
    assert_eq!(report["rows"][0]["paused"]["status"], "not_measured");
    assert_eq!(report["rows"][0]["commands"]["status"], "not_measured");
    assert_eq!(report["rows"][1]["paused"]["median"], 151.0);
}

/// The wall times of a run where each default costs its own time.
fn wall(page: LoadPage, row: Row) -> f64 {
    let collie = match row {
        Row::Defaults => 0.400,
        Row::LiveHidingOff => 0.390,
        Row::ConsentOff => 0.370,
        Row::FilterListsOff => 0.340,
        Row::EgressRelayOff => 0.300,
        Row::Playwright => 0.450,
        Row::PlaywrightRouted => 0.500,
    };
    match page {
        LoadPage::Plain => collie,
        LoadPage::Ads => collie - 0.050,
    }
}

#[test]
fn the_section_gives_each_defaults_cost_on_the_plain_page_and_the_target_on_both() {
    let section = section(&every(2, wall), &[]);
    let milliseconds = |seconds: Option<f64>| format!("{:.1}", seconds.unwrap() * 1000.0);
    let costs: Vec<(&str, u8, u8, String)> = section
        .costs
        .iter()
        .map(|cost| {
            (
                cost.default,
                cost.with.number(),
                cost.without.number(),
                milliseconds(cost.seconds),
            )
        })
        .collect();
    assert_eq!(
        costs,
        [
            ("live_view.hiding", 1, 2, "10.0".to_owned()),
            ("consent.handling", 2, 3, "20.0".to_owned()),
            ("filter_lists", 2, 4, "50.0".to_owned()),
            ("security.raw_cdp_egress_proxy", 1, 5, "100.0".to_owned()),
        ]
    );
    assert_eq!(milliseconds(section.route), "50.0");
    assert_eq!(section.target_held, Some(true));
    let report = serde_json::to_value(&section).unwrap();
    assert_eq!(report["rows"].as_array().unwrap().len(), 14);
    assert_eq!(report["rows"][0]["wall_seconds"]["median"], 0.4);
    let text = {
        let mut text = String::new();
        markdown(&mut text, &section);
        text
    };
    assert!(
        text.contains("| 1 Collie, defaults | plain |"),
        "the table names each row: {text}"
    );
    assert!(
        text.contains("| 151 (151–151) | 401.5 (401–402) |"),
        "Collie's rows give its engine's paused requests and CDP commands: {text}"
    );
    assert!(!text.contains("missing"), "{text}");
    assert!(text.contains("| n/a | n/a |"), "{text}");
    assert_eq!(report["rows"][0]["commands"]["median"], 401.5);
    assert!(
        report["rows"][5].get("commands").is_none(),
        "Playwright's rows have no counts"
    );
    assert!(text.contains("`filter_lists`: +50.0 ms"), "{text}");
    assert!(text.contains("#1895"), "{text}");
    assert!(text.contains("(#2435)"), "{text}");
    assert!(
        text.contains("on both pages: held (plain held, ads held)"),
        "{text}"
    );
}

#[test]
fn the_target_is_held_only_on_both_pages() {
    // Collie at its defaults slower than Playwright on the page with ads
    // alone.
    let section = section(
        &every(2, |page, row| match (page, row) {
            (LoadPage::Ads, Row::Defaults) => 0.600,
            _ => wall(page, row),
        }),
        &[],
    );
    assert_eq!(section.target.get(&LoadPage::Plain), Some(&Some(true)));
    assert_eq!(section.target.get(&LoadPage::Ads), Some(&Some(false)));
    assert_eq!(section.target_held, Some(false));
}

#[test]
fn the_order_runs_each_rows_trials_on_each_page_once() {
    let mut state = 7;
    let mut order = order(2, &mut state, |state| {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        *state >> 33
    });
    assert_eq!(order.len(), 28);
    assert_ne!(
        order[..4],
        [
            (LoadPage::Plain, Row::Defaults, 1),
            (LoadPage::Plain, Row::Defaults, 2),
            (LoadPage::Plain, Row::LiveHidingOff, 1),
            (LoadPage::Plain, Row::LiveHidingOff, 2),
        ],
        "the trials are shuffled"
    );
    order.sort();
    order.dedup();
    assert_eq!(order.len(), 28);
}

#[test]
fn the_engines_counts_are_its_c5_answers_and_c1_sends_in_its_metrics() {
    let scraped = "\
# HELP goodfetch_counted_work_total The Counted work.
# TYPE goodfetch_counted_work_total counter
goodfetch_counted_work_total{shared_path=\"cdp_command\",work=\"sends\",otel_scope_name=\"goodfetch\"} 412
goodfetch_counted_work_total{shared_path=\"cdp_command\",work=\"decodes\",otel_scope_name=\"goodfetch\"} 900
goodfetch_counted_work_total{shared_path=\"intercepted_request\",work=\"answers\",otel_scope_name=\"goodfetch\"} 151
goodfetch_counted_work_total{shared_path=\"intercepted_request\",work=\"decisions\",otel_scope_name=\"goodfetch\"} 3
";
    assert_eq!(
        engine_counts(scraped),
        Some(EngineCounts {
            paused: 151,
            commands: 412
        })
    );
    let no_answers = scraped.replace("work=\"answers\"", "work=\"other\"");
    assert_eq!(engine_counts(&no_answers), None);
}

#[test]
fn the_counts_it_reads_are_the_engines_c5_answers_and_c1_sends_rows() {
    use goodfetch_counted_work::Row;

    assert_eq!(
        (PAUSED, COMMANDS),
        (
            (
                Row::InterceptedAnswers.path(),
                Row::InterceptedAnswers.work()
            ),
            (Row::CdpSends.path(), Row::CdpSends.work()),
        )
    );
}
