//! The `portal` family, the lab's portal site ported with its tests as
//! cases: reports behind a CSRF login whose sessions expire, refresh and are
//! revoked; one listing behind six kinds of rendering; and a crawler trap and
//! a table that changes while it is paged. Each case drives the pages with a
//! plain HTTP client the way the lab's policies did, and the oracle judges the
//! Records it read and the sign-ins the Proving ground recorded.
//! Covers: story:2-40

use regex::Regex;
use serde_json::{Value, json};

use crate::browserless::{Client, Response};
use crate::proving_ground::ProvingGround;

const SEED: u64 = 161;

/// The task `id` of the task list.
fn task(proving_ground: &ProvingGround, id: &str) -> Value {
    proving_ground.tasks()["tasks"]
        .as_array()
        .expect("`tasks` is an array")
        .iter()
        .find(|task| task["id"] == id)
        .unwrap_or_else(|| panic!("the task list has no {id}"))
        .clone()
}

/// What a tool reports for `task`: its status and its Records.
fn result(task: &Value, status: &str, records: &[Value]) -> Value {
    json!({ "task": task["id"], "status": status, "records": records })
}

fn start_url(task: &Value) -> &str {
    task["start_url"].as_str().expect("a start URL")
}

/// The origin of a task's start URL.
fn origin(task: &Value) -> String {
    let url = start_url(task);
    let rest = url.strip_prefix("http://").expect("an http URL");
    format!("http://{}", &rest[..rest.find('/').expect("a path")])
}

/// The Records a page's HTML shows, as `id` and `price`.
fn records(html: &str) -> Vec<Value> {
    Regex::new(r#"data-id="([^"]+)" data-price="([0-9]+)""#)
        .expect("a valid pattern")
        .captures_iter(html)
        .map(|record| json!({ "id": &record[1], "price": &record[2] }))
        .collect()
}

/// The `href` of a page's Next link, resolved against `origin`.
fn next_link(origin: &str, html: &str) -> Option<String> {
    Regex::new(r#"<a class="next" href="([^"]+)""#)
        .expect("a valid pattern")
        .captures(html)
        .map(|link| format!("{origin}{}", link[1].replace("&amp;", "&")))
}

/// Signs in with the task's account through the login form, as the lab's
/// session recipe did: its page's CSRF token, then the form, whose redirect
/// it leaves to the crawl.
fn sign_in(client: &mut Client<'_>, task: &Value) -> Response {
    let origin = origin(task);
    let form = client.get(&format!("{origin}/login")).text();
    let csrf = Regex::new(r#"name="csrf" value="([^"]+)""#)
        .expect("a valid pattern")
        .captures(&form)
        .expect("the login form has a CSRF token")[1]
        .to_owned();
    let account = &task["account"];
    client.post_form_once(
        &format!("{origin}/login"),
        &format!(
            "csrf={csrf}&user={}&password={}",
            account["username"].as_str().expect("a username"),
            account["password"].as_str().expect("a password")
        ),
    )
}

/// How a crawl of the reports tells a page it can use from a lost session.
#[derive(Clone, Copy)]
enum Policy {
    /// Trusts the status and the URL: a `401`, a `403` or the login page's
    /// URL means the session is lost, anything else is a page of reports.
    StatusOnly,
    /// Also requires the page to be the signed-in listing; on a lost
    /// session it refreshes when it holds a refresh token, else signs in.
    Contract,
    /// As `Contract`, but never refreshes: it signs in again.
    ContractWithoutRefresh,
}

/// What a lost session looks like to `policy`.
fn session_lost(policy: Policy, page: &Response) -> bool {
    let at_login = page.url.contains("/login");
    match policy {
        Policy::StatusOnly => matches!(page.status, 401 | 403) || at_login,
        Policy::Contract | Policy::ContractWithoutRefresh => {
            at_login || !page.text().contains(r#"<main id="records" data-auth="#)
        }
    }
}

/// Crawls the reports from `next` until the last page or `pages` pages, as
/// the lab's session run did; answers the Records and the next page left.
fn crawl_reports(
    client: &mut Client<'_>,
    task: &Value,
    policy: Policy,
    mut next: Option<String>,
    pages: usize,
) -> (Vec<Value>, Option<String>) {
    let origin = origin(task);
    let mut found = Vec::new();
    let mut read = 0;
    while let Some(url) = next.clone().filter(|_| read < pages) {
        let mut page = client.get(&url);
        let mut recoveries = 0;
        while session_lost(policy, &page) {
            assert!(recoveries < 2, "the session could not be restored");
            recoveries += 1;
            let refreshed = matches!(policy, Policy::Contract)
                && client.cookie("rt").is_some()
                && client.post(&format!("{origin}/session/refresh")).status == 204;
            if !refreshed {
                sign_in(client, task);
            }
            page = client.get(&url);
        }
        let html = page.text();
        found.extend(records(&html));
        read += 1;
        next = next_link(&origin, &html);
    }
    (found, next)
}

/// Covers: story:2-40
#[test]
fn a_crawl_behind_the_csrf_login_that_signs_in_once_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/1");
    let mut client = Client::new(&proving_ground);

    let (found, _) = crawl_reports(
        &mut client,
        &task,
        Policy::Contract,
        Some(start_url(&task).to_owned()),
        usize::MAX,
    );
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert_eq!(found.len(), 100);
}

/// The whole crawl of a session task under `policy`, from its start page.
fn crawl_all(client: &mut Client<'_>, task: &Value, policy: Policy) -> Vec<Value> {
    let (found, _) = crawl_reports(
        client,
        task,
        policy,
        Some(start_url(task).to_owned()),
        usize::MAX,
    );
    found
}

/// A crawl that stops after two pages, then resumes in a new client from
/// its journal: the next page, the Records so far, and the cookies unless
/// `keep_session` is false; between the two, `between` runs.
fn resumed_crawl(
    proving_ground: &ProvingGround,
    task: &Value,
    keep_session: bool,
    between: impl FnOnce(),
) -> Vec<Value> {
    let mut first = Client::new(proving_ground);
    let (mut found, next) = crawl_reports(
        &mut first,
        task,
        Policy::Contract,
        Some(start_url(task).to_owned()),
        2,
    );
    let cookies = if keep_session {
        first.cookies()
    } else {
        Vec::new()
    };
    drop(first);
    between();
    let mut second = Client::new(proving_ground);
    second.import(cookies);
    if !keep_session {
        sign_in(&mut second, task);
    }
    let (rest, _) = crawl_reports(&mut second, task, Policy::Contract, next, usize::MAX);
    found.extend(rest);
    found
}

/// Covers: story:2-40
#[test]
fn a_portal_that_answers_an_expired_session_with_a_200_login_page_fools_a_status_only_crawl() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/2");
    let mut client = Client::new(&proving_ground);

    let found = crawl_all(&mut client, &task, Policy::StatusOnly);
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(found.len(), 80);
    assert_eq!(
        answer["missing"].as_array().map(Vec::len),
        Some(20),
        "{answer}"
    );
    assert_eq!(
        answer["missing_effects"],
        json!([{ "kind": "sign_in" }]),
        "{answer}"
    );
}

/// Covers: story:2-40
#[test]
fn a_crawl_that_signs_in_again_when_the_session_expires_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/2");
    let mut client = Client::new(&proving_ground);

    let found = crawl_all(&mut client, &task, Policy::Contract);
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn the_refresh_token_avoids_a_second_sign_in() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/3");
    let mut client = Client::new(&proving_ground);

    let found = crawl_all(&mut client, &task, Policy::Contract);
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn a_crawl_that_signs_in_again_instead_of_refreshing_carries_out_an_extra_sign_in() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/3");
    let mut client = Client::new(&proving_ground);

    let found = crawl_all(&mut client, &task, Policy::ContractWithoutRefresh);
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["missing"], json!([]), "{answer}");
    assert_eq!(
        answer["extra_effects"],
        json!([{ "kind": "sign_in" }]),
        "{answer}"
    );
}

/// Covers: story:2-40
#[test]
fn a_resumed_crawl_reuses_its_journaled_session() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/4");

    let found = resumed_crawl(&proving_ground, &task, true, || {});
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn a_resumed_crawl_that_signs_in_at_every_run_carries_out_an_extra_sign_in() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/4");

    let found = resumed_crawl(&proving_ground, &task, false, || {});
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["extra_effects"],
        json!([{ "kind": "sign_in" }]),
        "{answer}"
    );
}

/// Covers: story:2-40
#[test]
fn a_resumed_crawl_whose_session_was_revoked_signs_in_again() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/5");

    let found = resumed_crawl(&proving_ground, &task, true, || {
        let revoked = Client::new(&proving_ground).post(&format!(
            "http://{}/revoke-sessions",
            proving_ground.address()
        ));
        assert_eq!(revoked.status, 204);
    });
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn a_sign_in_without_the_forms_csrf_token_is_forbidden_and_signs_nobody_in() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/1");
    let mut client = Client::new(&proving_ground);
    let account = &task["account"];

    client.get(&format!("{}/login", origin(&task)));
    let refused = client.post_form(
        &format!("{}/login", origin(&task)),
        &format!(
            "csrf=forged&user={}&password={}",
            account["username"].as_str().unwrap(),
            account["password"].as_str().unwrap()
        ),
    );

    assert_eq!(refused.status, 403);
    assert_eq!(client.cookie("sid"), None);
    let answer = proving_ground.judge(&result(&task, "failure", &[]));
    assert_eq!(
        answer["missing_effects"],
        json!([{ "kind": "sign_in" }]),
        "{answer}"
    );
}

/// Covers: story:2-40
#[test]
fn a_wrong_password_answers_the_login_form_again_with_a_401() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/1");
    let mut client = Client::new(&proving_ground);
    let form = client.get(&format!("{}/login", origin(&task))).text();
    let csrf = Regex::new(r#"name="csrf" value="([^"]+)""#)
        .unwrap()
        .captures(&form)
        .unwrap()[1]
        .to_owned();

    let refused = client.post_form(
        &format!("{}/login", origin(&task)),
        &format!(
            "csrf={csrf}&user={}&password=wrong",
            task["account"]["username"].as_str().unwrap()
        ),
    );

    assert_eq!(refused.status, 401);
    assert!(
        refused.text().contains(r#"<form id="login""#),
        "{}",
        refused.text()
    );
    assert_eq!(client.cookie("sid"), None);
}

/// The Records of every listing page from the task's start page, read from
/// the HTML alone, through each Next link.
fn crawl_html(client: &mut Client<'_>, task: &Value) -> Vec<Value> {
    let origin = origin(task);
    let mut found = Vec::new();
    let mut next = Some(start_url(task).to_owned());
    while let Some(url) = next {
        let html = client.get(&url).text();
        found.extend(records(&html));
        next = next_link(&origin, &html);
    }
    found
}

/// The Records of the listing API's pages from page 1, starting at
/// `offset` on each, through each answer's `next`, as a page's script reads
/// them.
fn crawl_api(client: &mut Client<'_>, task: &Value, offset: u32) -> Vec<Value> {
    let origin = origin(task);
    let mut found = Vec::new();
    let mut page = Some(1);
    while let Some(number) = page {
        let answer = client
            .get(&format!("{origin}/api/list?page={number}&offset={offset}"))
            .json();
        found.extend(
            answer["items"]
                .as_array()
                .expect("items")
                .iter()
                .map(api_record),
        );
        page = answer["next"].as_u64();
    }
    found
}

/// A Record from an API item, which carries its price as a number.
fn api_record(item: &Value) -> Value {
    json!({ "id": item["id"], "price": item["price"].to_string() })
}

/// Covers: story:2-40
#[test]
fn a_static_listing_read_from_its_html_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/6");
    let mut client = Client::new(&proving_ground);

    let found = crawl_html(&mut client, &task);
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn a_script_shells_html_holds_no_records_and_its_api_holds_them_all() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/7");
    let mut client = Client::new(&proving_ground);

    let from_html = crawl_html(&mut client, &task);
    let answer = proving_ground.judge(&result(&task, "success", &crawl_api(&mut client, &task, 0)));

    assert_eq!(from_html, Vec::<Value>::new());
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn a_partly_hydrated_listing_read_from_its_html_alone_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/8");
    let mut client = Client::new(&proving_ground);

    let found = crawl_html(&mut client, &task);
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(found.len(), 40);
    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["missing"].as_array().map(Vec::len),
        Some(60),
        "{answer}"
    );
}

/// Covers: story:2-40
#[test]
fn a_partly_hydrated_listing_read_with_the_api_its_script_calls_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/8");
    let mut client = Client::new(&proving_ground);

    let mut found = crawl_html(&mut client, &task);
    found.extend(crawl_api(&mut client, &task, 8));
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn stale_html_prices_are_wrong_until_the_script_corrects_them() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/9");
    let mut client = Client::new(&proving_ground);

    let found = crawl_html(&mut client, &task);
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["missing"], json!([]), "{answer}");
    assert_eq!(
        answer["wrong"].as_array().map(Vec::len),
        Some(100),
        "{answer}"
    );
    let wrong = &answer["wrong"][0];
    let shown = wrong["submitted"].as_str().unwrap().parse::<u64>().unwrap();
    let expected = wrong["expected"].as_str().unwrap().parse::<u64>().unwrap();
    assert_eq!(shown, expected + 7, "{answer}");
}

/// Covers: story:2-40
#[test]
fn stale_html_with_the_prices_its_script_reads_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/9");
    let mut client = Client::new(&proving_ground);
    let origin = origin(&task);

    let found: Vec<Value> = (1..=5)
        .flat_map(|page| {
            client
                .get(&format!("{origin}/api/prices?page={page}"))
                .json()["prices"]
                .as_array()
                .expect("prices")
                .iter()
                .map(api_record)
                .collect::<Vec<_>>()
        })
        .collect();
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn a_graphql_listing_paged_by_its_cursor_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/10");
    let mut client = Client::new(&proving_ground);
    let origin = origin(&task);
    let shell = client.get(start_url(&task)).text();

    let mut found = Vec::new();
    let mut after = Value::Null;
    loop {
        let answer = client.post_json(
            &format!("{origin}/graphql"),
            &json!({
                "query": "query Products($after:String){products(first:20,after:$after){nodes{id name price} pageInfo{endCursor hasNextPage}}}",
                "variables": { "after": after },
            }),
        );
        let products = &answer.json()["data"]["products"];
        found.extend(
            products["nodes"]
                .as_array()
                .expect("nodes")
                .iter()
                .map(api_record),
        );
        if products["pageInfo"]["hasNextPage"] != true {
            break;
        }
        after = products["pageInfo"]["endCursor"].clone();
    }
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert!(
        shell.contains(r#"<button class="more" hidden>Load more</button>"#),
        "{shell}"
    );
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn an_infinite_feed_read_without_scrolling_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/11");
    let mut client = Client::new(&proving_ground);

    let found = records(&client.get(start_url(&task)).text());
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(found.len(), 20);
    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

/// Covers: story:2-40
#[test]
fn an_infinite_feed_read_through_its_cursors_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/11");
    let mut client = Client::new(&proving_ground);
    let origin = origin(&task);
    let feed = client.get(start_url(&task)).text();

    let mut found = records(&feed);
    let mut cursor = Regex::new(r#"data-cursor="([^"]+)""#)
        .unwrap()
        .captures(&feed)
        .unwrap()[1]
        .to_owned();
    loop {
        let answer = client
            .get(&format!("{origin}/api/feed?cursor={cursor}"))
            .json();
        found.extend(
            answer["items"]
                .as_array()
                .expect("items")
                .iter()
                .map(api_record),
        );
        match answer["nextCursor"].as_str() {
            Some(next) => cursor = next.to_owned(),
            None => break,
        }
    }
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Every `href` of a page, resolved against `origin`, `&amp;` decoded.
fn links(origin: &str, html: &str) -> Vec<String> {
    Regex::new(r#"href="([^"]+)""#)
        .expect("a valid pattern")
        .captures_iter(html)
        .map(|link| format!("{origin}{}", link[1].replace("&amp;", "&")))
        .collect()
}

/// A URL without the parameters that never change the page: session IDs,
/// tracking parameters and sort orders, as the lab's canonical frontier
/// dropped them.
fn canonical(url: &str) -> String {
    let Some((path, query)) = url.split_once('?') else {
        return url.to_owned();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| {
            let name = pair.split('=').next().unwrap_or_default();
            !(name == "sid" || name.starts_with("utm_") || name == "sort" || name == "order")
        })
        .collect();
    if kept.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{}", kept.join("&"))
    }
}

/// Crawls the shop breadth-first within `budget` requests, from its start
/// page; `canonical_frontier` drops the parameters that never change a
/// page and stops paging the calendar after three empty months. Answers the
/// Records and whether the frontier emptied within the budget.
fn crawl_shop(
    client: &mut Client<'_>,
    task: &Value,
    canonical_frontier: bool,
    budget: usize,
) -> (Vec<Value>, bool) {
    let origin = origin(task);
    let normalize = |url: &str| {
        if canonical_frontier {
            canonical(url)
        } else {
            url.to_owned()
        }
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut frontier = std::collections::VecDeque::from([normalize(start_url(task))]);
    seen.insert(frontier[0].clone());
    let mut found = std::collections::BTreeMap::new();
    let mut empty_months = 0;
    let mut requests = 0;
    while let Some(url) = frontier.pop_front() {
        if requests == budget {
            return (found.into_values().collect(), false);
        }
        requests += 1;
        let html = client.get(&url).text();
        let page_records = records(&html);
        if url.contains("/events") {
            empty_months = if page_records.is_empty() {
                empty_months + 1
            } else {
                0
            };
            if canonical_frontier && empty_months >= 3 {
                continue;
            }
        }
        for record in page_records {
            found.insert(record["id"].as_str().expect("an ID").to_owned(), record);
        }
        for link in links(&origin, &html) {
            let link = normalize(&link);
            if seen.insert(link.clone()) {
                frontier.push_back(link);
            }
        }
    }
    (found.into_values().collect(), true)
}

/// Covers: story:2-40
#[test]
fn the_canonical_frontier_ends_a_crawl_through_session_ids_sort_orders_and_an_endless_calendar() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/12");
    let mut client = Client::new(&proving_ground);

    let (found, finished) = crawl_shop(&mut client, &task, true, 300);
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert!(finished, "the canonical frontier did not empty");
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn the_naive_frontier_runs_out_of_its_budget_in_the_crawler_trap() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/12");
    let mut client = Client::new(&proving_ground);

    let (_, finished) = crawl_shop(&mut client, &task, false, 300);

    assert!(!finished, "the naive frontier emptied within its budget");
}

/// Covers: story:2-40
#[test]
fn a_crawler_trap_page_is_the_same_for_the_same_url() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/12");
    let mut client = Client::new(&proving_ground);

    let pages = (
        client.get(start_url(&task)).text(),
        client.get(start_url(&task)).text(),
    );

    assert_eq!(pages.0, pages.1);
    assert!(pages.0.contains("sid="), "{}", pages.0);
}

/// Crawls the orders table, by its page numbers or by the `rel="next"`
/// cursor link, until it ends.
fn crawl_table(client: &mut Client<'_>, task: &Value, by_cursor: bool) -> Vec<Value> {
    let origin = origin(task);
    let cursor = Regex::new(r#"<link rel="next" href="([^"]+)">"#).expect("a valid pattern");
    let mut found = Vec::new();
    let mut next = Some(start_url(task).to_owned());
    while let Some(url) = next {
        let html = client.get(&url).text();
        found.extend(records(&html));
        next = if by_cursor {
            cursor
                .captures(&html)
                .map(|link| format!("{origin}{}", &link[1]))
        } else {
            next_link(&origin, &html)
        };
    }
    found
}

/// Covers: story:2-40
#[test]
fn offset_pagination_over_a_table_that_changes_while_it_is_paged_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/13");
    let mut client = Client::new(&proving_ground);

    let found = crawl_table(&mut client, &task, false);
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

/// Covers: story:2-40
#[test]
fn cursor_pagination_over_a_table_that_changes_while_it_is_paged_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/13");
    let mut client = Client::new(&proving_ground);

    let found = crawl_table(&mut client, &task, true);
    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn the_table_oracle_counts_a_row_submitted_twice_as_extra() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/13");
    let mut client = Client::new(&proving_ground);
    let mut found = crawl_table(&mut client, &task, true);
    found.push(found[0].clone());

    let answer = proving_ground.judge(&result(&task, "success", &found));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["extra"], json!([found[0]]), "{answer}");
}

/// Covers: story:2-40
#[test]
fn the_table_oracle_requires_the_rows_that_lived_through_the_whole_crawl() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "portal/13");
    let mut client = Client::new(&proving_ground);
    let first = records(&client.get(start_url(&task)).text());

    let answer = proving_ground.judge(&result(&task, "needs-review", &first));

    assert_eq!(first.len(), 50);
    assert_eq!(answer["outcome"], "incomplete", "{answer}");
    assert_eq!(
        answer["missing"].as_array().map(Vec::len),
        Some(1450),
        "{answer}"
    );
}
