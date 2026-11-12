//! The plain hosts, which belong to no task: every path of each is a
//! small page, none has a robots.txt, and a page under `/held/<key>` is
//! sent only once `/release/<key>` was asked for, so a crawl's test keeps
//! a request in flight for as long as it needs.

use std::sync::mpsc;
use std::time::Duration;

use crate::proving_ground::ProvingGround;
use crate::raw_clients;

const SEED: u64 = 1602;

/// How long the test waits for a page the site must not send yet: a held
/// page that came within it fails the test, and one the site holds, as it
/// must, never comes however long the wait.
const NOT_SENT_WITHIN: Duration = Duration::from_millis(300);

#[test]
fn any_path_of_a_plain_host_is_a_small_page_that_names_it() {
    let proving_ground = ProvingGround::start(SEED);

    let page = raw_clients::get(&proving_ground, "shop.test", "/any/path", &[]);

    assert_eq!(
        (
            page.status(),
            String::from_utf8_lossy(page.body()).contains("A page at /any/path.")
        ),
        (Some(200), true),
        "{:?}",
        page.body()
    );
}

#[test]
fn a_plain_host_has_no_robots_txt() {
    let proving_ground = ProvingGround::start(SEED);

    let robots = raw_clients::get(&proving_ground, "bob.github.io", "/robots.txt", &[]);

    assert_eq!(robots.status(), Some(404));
}

#[test]
fn a_held_page_is_sent_only_once_its_key_is_released() {
    let proving_ground = ProvingGround::start(SEED);

    let (before, released, held, after) = std::thread::scope(|scope| {
        let (sent, arrived) = mpsc::channel();
        let proving_ground = &proving_ground;
        scope.spawn(move || {
            let page = raw_clients::get(proving_ground, "a.test", "/held/first", &[]);
            sent.send(page.status()).expect("the test still waits");
        });
        let before = arrived.recv_timeout(NOT_SENT_WITHIN).ok();
        let released = raw_clients::get(proving_ground, "b.test", "/release/first", &[]);
        let held = arrived.recv().expect("the held page arrives");
        let after = raw_clients::get(proving_ground, "a.test", "/held/first", &[]);
        (before, released.status(), held, after.status())
    });

    assert_eq!(
        (before, released, held, after),
        (None, Some(200), Some(200), Some(200))
    );
}
