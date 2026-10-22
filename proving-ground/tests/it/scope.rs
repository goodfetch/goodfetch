//! The `scope` family (pitfall U15): a site whose index links a page on
//! its own host, its `www` host, a subdomain, its own host over `https`, a
//! sister domain and an off-site host, all on the port the request named;
//! the `www` page links back and to the subdomain; an unlinked page
//! redirects off site; and each request's host is in the observations.
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
