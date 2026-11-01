//! The content origin's signing behind its interface (spec #592's Seam 3),
//! on the paused `Clock`: an address signed for a file serves that file,
//! as it is to be shown, for 5 minutes and no longer; an altered address,
//! another file's signature, another key's and an address with no
//! signature serve nothing.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059)"
)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_test_support::runtime;
use rstest::rstest;

use super::{ContentFile, ContentKey, Shown, Unsigned, VALID_FOR};
use crate::clock::Clock;

const BODY: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
const OTHER_BODY: &str = "60303ae22b998861bce3b28f33eec1be758a213c86c93c076dbe9f558c11c752";

fn start() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_790_000_000)
}

fn file(body: &str) -> ContentFile {
    ContentFile::of("first", body, Shown::Html).unwrap()
}

fn key() -> ContentKey {
    ContentKey::generated().unwrap()
}

/// Covers: story:12-127
#[test]
fn an_address_serves_its_file_until_five_minutes_have_passed() {
    runtime().block_on(async {
        let clock = Clock::paused(start());
        let key = key();
        let signed = key.sign(&file(BODY), clock.wall());

        assert_eq!(key.check(&signed, clock.wall()), Ok(file(BODY)));
        tokio::time::advance(VALID_FOR.checked_sub(Duration::from_secs(1)).unwrap()).await;
        assert_eq!(key.check(&signed, clock.wall()), Ok(file(BODY)));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(key.check(&signed, clock.wall()), Err(Unsigned::Expired));
    });
}

/// Covers: story:12-127
#[test]
fn an_address_names_its_file_and_how_it_is_shown_and_no_credential() {
    let key = key();
    let file = ContentFile::of("first", BODY, Shown::Png).unwrap();

    let signed = key.sign(&file, start());

    let (path, query) = signed.split_once('?').unwrap();
    assert_eq!(path, format!("/bodies/first/{BODY}"));
    let names: Vec<&str> = query
        .split('&')
        .map(|pair| pair.split_once('=').unwrap().0)
        .collect();
    assert_eq!(names, ["type", "expires", "signature"]);
    assert!(query.starts_with("type=png&expires=1790000300&"), "{query}");
    assert_eq!(
        key.check(&signed, start()).map(|file| file.shown),
        Ok(Shown::Png)
    );
}

/// The signed address of [`BODY`], changed by `change`.
fn changed(key: &ContentKey, change: fn(&str) -> String) -> String {
    change(&key.sign(&file(BODY), start()))
}

/// Covers: story:12-127
#[rstest]
#[case::signature_altered(|signed: &str| {
    let (rest, signature) = signed.rsplit_once('=').unwrap();
    let flipped = if signature.starts_with('0') { "1" } else { "0" };
    format!("{rest}={flipped}{}", &signature[1..])
}, Unsigned::Wrong)]
#[case::expiry_moved_later(|signed: &str| signed.replace("expires=1790000300", "expires=1790009999"), Unsigned::Wrong)]
#[case::shown_another_way(|signed: &str| signed.replace("type=html", "type=text"), Unsigned::Wrong)]
#[case::another_files_path(|signed: &str| signed.replace(BODY, OTHER_BODY), Unsigned::Wrong)]
#[case::another_runs_path(|signed: &str| signed.replace("/first/", "/second/"), Unsigned::Wrong)]
#[case::signature_twice(|signed: &str| format!("{signed}&signature=00"), Unsigned::Wrong)]
#[case::way_unknown(|signed: &str| signed.replace("type=html", "type=script"), Unsigned::Wrong)]
#[case::no_signature(|signed: &str| signed.split_once("&signature=").unwrap().0.to_owned(), Unsigned::Missing)]
#[case::no_query(|signed: &str| signed.split_once('?').unwrap().0.to_owned(), Unsigned::Missing)]
#[case::no_file(|signed: &str| signed.replace(BODY, "not-a-body"), Unsigned::NoFile)]
#[case::outside_the_bodies(|signed: &str| signed.replace("/bodies/", "/files/"), Unsigned::NoFile)]
fn an_address_changed_in_any_part_serves_nothing(
    #[case] change: fn(&str) -> String,
    #[case] refused: Unsigned,
) {
    let key = key();

    assert_eq!(key.check(&changed(&key, change), start()), Err(refused));
}

/// Covers: story:12-127
#[test]
fn another_keys_signature_serves_nothing() {
    let signed = key().sign(&file(BODY), start());

    assert_eq!(key().check(&signed, start()), Err(Unsigned::Wrong));
}
