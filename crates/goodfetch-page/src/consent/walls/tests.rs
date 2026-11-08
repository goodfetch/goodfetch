#![expect(
    clippy::disallowed_methods,
    reason = "the waits run on the Clock's test version (ADR 0059)"
)]

use std::time::{Duration, SystemTime};

use goodfetch_engine::clock::Clock;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;
use serde_json::json;

use super::{Approval, Walls};

/// `consent.domains` covers a document by its registrable domain (ADR
/// 0004), a subdomain included, and `*` covers every one; another domain,
/// a lookalike among them, is not covered.
#[rstest]
#[case::the_domain(json!(["news.example"]), "https://news.example/a", Some("news.example"))]
#[case::a_subdomain(json!(["news.example"]), "https://www.news.example/a", Some("news.example"))]
#[case::written_in_capitals(json!(["News.Example."]), "https://news.example/", Some("news.example"))]
#[case::an_international_name(json!(["Bücher.de"]), "https://bücher.de/a", Some("xn--bcher-kva.de"))]
#[case::in_punycode(json!(["xn--bcher-kva.de"]), "https://bücher.de/a", Some("xn--bcher-kva.de"))]
#[case::every_domain(json!(["*"]), "https://shop.example/", Some("shop.example"))]
#[case::another_domain(json!(["news.example"]), "https://shop.example/", None)]
#[case::a_lookalike(json!(["news.example"]), "https://news.example.evil.test/", None)]
fn consent_domains_cover_a_document_by_its_registrable_domain(
    #[case] domains: serde_json::Value,
    #[case] url: &str,
    #[case] covered: Option<&str>,
) {
    let approval = Approval::of(&domains, SettingsLayer::UserConfiguration)
        .expect("a list with a domain approves it");
    assert_eq!(approval.covers(url).as_deref(), covered);
}

/// An empty `consent.domains`, the default, approves nothing.
#[test]
fn an_empty_list_approves_nothing() {
    assert_eq!(Approval::of(&json!([]), SettingsLayer::Default), None);
}

/// What a call waiting on the runner's first pass sees when `steps` run on
/// the walls 2 s into its wait: whether the pass ended in time, and how
/// long it waited, of a 30 s limit.
fn waited(detected: bool, steps: fn(&Walls, &PageId)) -> (bool, Duration) {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(SystemTime::UNIX_EPOCH);
        let (walls, page) = (Walls::default(), PageId("page-1".to_owned()));
        if detected {
            walls.detected(&page);
        }
        let started = clock.now();
        let (seen, ()) = tokio::join!(
            async {
                let ended = walls.verdict(&page, Duration::from_secs(30), clock).await;
                (ended, clock.now() - started)
            },
            async {
                tokio::time::sleep(Duration::from_secs(2)).await;
                steps(&walls, &page);
            }
        );
        seen
    })
}

/// A call waits on the runner's first pass only once it found a prompt, and
/// then until the pass ends or the document goes, within its limit.
#[rstest]
#[case::nothing_detected(false, |_: &Walls, _: &PageId| {}, (true, Duration::ZERO))]
#[case::the_pass_settles(true, |walls: &Walls, page: &PageId| walls.settled(page), (true, Duration::from_secs(2)))]
#[case::the_document_goes(true, |walls: &Walls, page: &PageId| walls.new_document(page), (true, Duration::from_secs(2)))]
#[case::the_pass_never_ends(true, |_: &Walls, _: &PageId| {}, (false, Duration::from_secs(30)))]
fn a_call_waits_on_the_first_pass_only_once_it_found_a_prompt(
    #[case] detected: bool,
    #[case] steps: fn(&Walls, &PageId),
    #[case] seen: (bool, Duration),
) {
    assert_eq!(waited(detected, steps), seen);
}

/// A pass that ended stays ended: a later detection does not make a call
/// wait again on the same document.
#[test]
fn a_settled_pass_is_never_pending_again() {
    let (walls, page) = (Walls::default(), PageId("page-1".to_owned()));

    walls.settled(&page);
    walls.detected(&page);

    assert!(!walls.pending(&page));
}
