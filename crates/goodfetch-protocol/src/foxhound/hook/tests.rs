use rstest::rstest;
use serde_json::json;

use super::{
    CRAWL_HOOK_HANDLE, CrawlHookDeclaration, CrawlHookHandleParams, CrawlHookStage,
    HookDuplicateDetails, HookIneffectiveDetails, HookUnknownDetails,
};
use crate::catalog::Product;
use crate::entries::MODULES;
use crate::foxhound::crawl::{CrawlRequest, CrawlStartParams, PushedRecord};
use crate::foxhound::frontier::HttpMethod;
use crate::run::RunId;

#[rstest]
#[case::duplicate("hook_duplicate", Product::Foxhound)]
#[case::ineffective("hook_ineffective", Product::Foxhound)]
#[case::failed_also_by_a_collie_script_hook("hook_failed", Product::Collie)]
#[case::unknown("hook_unknown", Product::Foxhound)]
fn each_hook_code_is_declared_once_in_the_smallest_product_that_raises_it(
    #[case] name: &str,
    #[case] expected: Product,
) {
    let declared: Vec<_> = MODULES
        .into_iter()
        .flat_map(|entries| {
            entries
                .error_codes
                .iter()
                .filter(|code| code.name == name)
                .map(|_| Product::of_module(entries.defined_in))
        })
        .collect();

    assert_eq!(
        declared,
        [expected],
        "`{name}` is declared in these Products"
    );
}

#[test]
fn hook_duplicate_names_the_hook_and_its_stage() {
    let details = HookDuplicateDetails {
        hook: "addToken".to_owned(),
        stage: "before_request".to_owned(),
    };

    assert_eq!(
        serde_json::to_value(&details).unwrap(),
        json!({ "hook": "addToken", "stage": "before_request" })
    );
}

#[test]
fn hook_ineffective_names_the_hook_and_its_stage() {
    let details = HookIneffectiveDetails {
        hook: "before_navigation#1".to_owned(),
        stage: "before_navigation".to_owned(),
    };

    assert_eq!(
        serde_json::to_value(&details).unwrap(),
        json!({ "hook": "before_navigation#1", "stage": "before_navigation" })
    );
}

#[rstest]
#[case::closest(Some("addToken"), json!({ "hook": "addTokn", "closest": "addToken" }))]
#[case::none_held(None, json!({ "hook": "addTokn", "closest": null }))]
fn hook_unknown_names_the_hook_asked_for_and_the_closest(
    #[case] closest: Option<&str>,
    #[case] expected: serde_json::Value,
) {
    let details = HookUnknownDetails {
        hook: "addTokn".to_owned(),
        closest: closest.map(str::to_owned),
    };

    assert_eq!(serde_json::to_value(&details).unwrap(), expected);
}

#[test]
fn a_hook_call_is_a_handler_request_only_foxhound_sends() {
    let declared: Vec<_> = MODULES
        .into_iter()
        .filter(|entries| {
            entries
                .handler_requests
                .iter()
                .any(|request| request.name == CRAWL_HOOK_HANDLE.name)
        })
        .map(|entries| Product::of_module(entries.defined_in))
        .collect();

    assert_eq!(declared, [Product::Foxhound]);
}

#[test]
fn a_crawl_declares_its_hooks_in_order_with_their_stage_and_scope() {
    let params: CrawlStartParams = serde_json::from_value(json!({
        "seeds": [{ "url": "https://example.com/", "label": "list" }],
        "hooks": [
            { "name": "stamp", "stage": "before_record_commit", "datasets": ["products"] },
            { "name": "sign", "stage": "before_request", "labels": ["detail"] },
        ],
    }))
    .unwrap();

    assert_eq!(
        params.hooks,
        [
            CrawlHookDeclaration {
                name: "stamp".to_owned(),
                stage: CrawlHookStage::BeforeRecordCommit,
                labels: None,
                datasets: Some(vec!["products".to_owned()]),
            },
            CrawlHookDeclaration {
                name: "sign".to_owned(),
                stage: CrawlHookStage::BeforeRequest,
                labels: Some(vec!["detail".to_owned()]),
                datasets: None,
            },
        ]
    );
}

#[rstest]
#[case::before_request(CrawlHookStage::BeforeRequest, "before_request")]
#[case::after_response(CrawlHookStage::AfterResponse, "after_response")]
#[case::before_navigation(CrawlHookStage::BeforeNavigation, "before_navigation")]
#[case::after_navigation(CrawlHookStage::AfterNavigation, "after_navigation")]
#[case::page_ready(CrawlHookStage::PageReady, "page_ready")]
#[case::before_record_commit(CrawlHookStage::BeforeRecordCommit, "before_record_commit")]
fn each_stage_of_a_hook_in_code_keeps_the_catalogs_name(
    #[case] stage: CrawlHookStage,
    #[case] name: &str,
) {
    assert_eq!(serde_json::to_value(stage).unwrap(), json!(name));
    assert_eq!(stage.as_str(), name);
}

#[test]
fn a_record_stage_call_names_the_hook_its_request_and_the_record() {
    let params = CrawlHookHandleParams {
        run_id: RunId::parse("run-1").unwrap(),
        hook: "stamp".to_owned(),
        stage: CrawlHookStage::BeforeRecordCommit,
        crawl_request: CrawlRequest {
            id: "r1".to_owned(),
            url: "https://example.com/".to_owned(),
            label: "detail".to_owned(),
            payload: None,
            method: HttpMethod::get(),
            body: None,
        },
        attempt: 1,
        lease_epoch: 3,
        record: Some(PushedRecord {
            dataset: "products".to_owned(),
            data: json!({ "name": "Kettle" }),
        }),
    };

    assert_eq!(
        serde_json::to_value(&params).unwrap(),
        json!({
            "run_id": "run-1",
            "hook": "stamp",
            "stage": "before_record_commit",
            "crawl_request": {
                "id": "r1",
                "url": "https://example.com/",
                "label": "detail",
                "method": "GET",
            },
            "attempt": 1,
            "lease_epoch": 3,
            "record": { "dataset": "products", "data": { "name": "Kettle" } },
        })
    );
}
