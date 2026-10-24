use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;
use serde_json::json;

use super::Approval;

/// `consent.domains` covers a document by its registrable domain (ADR
/// 0004), a subdomain included, and `*` covers every one; another domain,
/// a lookalike among them, is not covered.
#[rstest]
#[case::the_domain(json!(["news.example"]), "https://news.example/a", Some("news.example"))]
#[case::a_subdomain(json!(["news.example"]), "https://www.news.example/a", Some("news.example"))]
#[case::written_in_capitals(json!(["News.Example."]), "https://news.example/", Some("news.example"))]
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
