//! The `scope` family (pitfall U15): a site whose index links a page on
//! its own host, its `www` host, a subdomain, its own host over `https`, a
//! sister domain and an off-site host, all on the port the request named;
//! the `www` page links back and to the subdomain; an unlinked page
//! redirects off site; an unlinked page a browser may keep in its cache;
//! an unlinked page that keeps a mark in a cookie and in `localStorage`;
//! and each request's host is in the observations.
//! Covers: story:2-33

use serde_json::Value;

use crate::crawler::{field, links};
use crate::proving_ground::ProvingGround;

const SEED: u64 = 840;

#[test]
fn the_index_links_every_host_of_the_scope_family_on_the_requests_port() {
    let proving_ground = ProvingGround::start(SEED);
    let port = proving_ground.port().to_owned();

    let index = proving_ground.get(&proving_ground.url("scope.test", "/"));

    assert_eq!(
        (index.status, links(&index.body, false)),
        (
            200,
            [
                "http://scope.test:{port}/about",
                "http://www.scope.test:{port}/news",
                "http://shop.scope.test:{port}/cart",
                "https://scope.test:{port}/account",
                "http://scope-sister.test:{port}/sister",
                "http://elsewhere.test:{port}/offsite",
            ]
            .map(|link| link.replace("{port}", &port))
            .to_vec()
        )
    );
}

#[test]
fn each_linked_page_answers_on_its_own_host_and_no_other() {
    let proving_ground = ProvingGround::start(SEED);
    let id = |host: &str, path: &str| {
        let page = proving_ground.get(&proving_ground.url(host, path));
        (
            page.status,
            field(&page.body, r#"data-id="([^"]+)""#).unwrap_or_default(),
        )
    };

    let pages = [
        id("scope.test", "/about"),
        id("www.scope.test", "/news"),
        id("shop.scope.test", "/cart"),
        id("scope.test", "/account"),
        id("scope-sister.test", "/sister"),
        id("elsewhere.test", "/offsite"),
        id("shop.scope.test", "/about"),
        id("scope.test", "/elsewhere"),
    ];

    assert_eq!(
        pages.to_vec(),
        [
            (200, "S-ABOUT"),
            (200, "S-NEWS"),
            (200, "S-CART"),
            (200, "S-ACCOUNT"),
            (200, "S-SISTER"),
            (200, "S-OFFSITE"),
            (404, ""),
            (404, ""),
        ]
        .map(|(status, id)| (status, id.to_owned()))
        .to_vec()
    );
}

#[test]
fn the_www_page_links_back_and_the_moved_page_redirects_off_site() {
    let proving_ground = ProvingGround::start(SEED);
    let port = proving_ground.port().to_owned();

    let news = proving_ground.get(&proving_ground.url("www.scope.test", "/news"));
    let moved = proving_ground.get(&proving_ground.url("scope.test", "/moved"));
    let hosts: Vec<Value> = proving_ground.observations()["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|request| request["host"].clone())
        .collect();

    assert_eq!(
        (
            links(&news.body, false),
            moved.status,
            moved.header("location").map(str::to_owned),
            hosts
        ),
        (
            vec![
                format!("http://scope.test:{port}/about"),
                format!("http://shop.scope.test:{port}/cart"),
            ],
            302,
            Some(format!("http://elsewhere.test:{port}/offsite")),
            vec![Value::from("www.scope.test"), Value::from("scope.test")]
        )
    );
}

#[test]
fn the_cached_page_lets_a_browser_keep_it_for_an_hour() {
    let proving_ground = ProvingGround::start(SEED);

    let cached = proving_ground.get(&proving_ground.url("scope.test", "/cached"));

    assert_eq!(
        (
            cached.status,
            cached.header("cache-control").map(str::to_owned),
            field(&cached.body, r#"data-id="([^"]+)""#)
        ),
        (
            200,
            Some("max-age=3600".to_owned()),
            Some("S-CACHED".to_owned())
        )
    );
}

#[test]
fn the_storage_page_keeps_its_mark_in_a_cookie_and_its_report_answers() {
    let proving_ground = ProvingGround::start(SEED);

    let marked = proving_ground.get(&proving_ground.url("scope.test", "/storage?mark=first-1"));
    let unmarked = proving_ground.get(&proving_ground.url("scope.test", "/storage?mark=a%3Bb"));
    let report = proving_ground.get(&proving_ground.url("scope.test", "/storage/found?mark=none"));

    assert_eq!(
        (
            marked.status,
            marked.header("set-cookie").map(str::to_owned),
            unmarked.header("set-cookie").map(str::to_owned),
            report.status,
        ),
        (
            200,
            Some("mark=first-1; Path=/; SameSite=Lax".to_owned()),
            Some("mark=none; Path=/; SameSite=Lax".to_owned()),
            200,
        )
    );
}

#[test]
fn the_quiet_page_shows_its_mark_or_none_for_an_unsafe_one_and_answers_the_status_asked() {
    let proving_ground = ProvingGround::start(SEED);

    let marked = proving_ground.get(&proving_ground.url("scope.test", "/quiet?mark=turns-1"));
    let unmarked = proving_ground.get(&proving_ground.url("scope.test", "/quiet?mark=a%3Bb"));
    let refusing =
        proving_ground.get(&proving_ground.url("scope.test", "/quiet?mark=x&status=429"));

    assert_eq!(
        (
            marked.status,
            marked.body.contains("<p id=\"mark\">turns-1</p>"),
            unmarked.body.contains("<p id=\"mark\">none</p>"),
            refusing.status,
        ),
        (200, true, true, 429)
    );
}
