use rstest::rstest;

use super::{RunId, RunIdInvalid, Tags, TagsInvalid};
use crate::catalog::{Product, SharedType};

#[rstest]
#[case::one_letter("a")]
#[case::one_digit("7")]
#[case::every_allowed_character("my-run.v2_1")]
#[case::dots_inside("a..b")]
#[case::a_ulid("01jbz3kq8m5v2c7x9d4e6f0g1h")]
#[case::reserved_name_inside_another("console")]
#[case::longest(&"a".repeat(128))]
fn a_run_id_of_safe_characters_reads(#[case] text: &str) {
    assert_eq!(
        RunId::parse(text).map(|id| id.as_str().to_owned()),
        Ok(text.to_owned())
    );
}

/// TESTING.md's path traversal cases, each OS's own included (ADR 0067):
/// a Run ID names a folder, so none reaches outside the run folder.
#[rstest]
#[case::empty("", RunIdInvalid::Characters)]
#[case::current_folder(".", RunIdInvalid::Characters)]
#[case::parent_folder("..", RunIdInvalid::Characters)]
#[case::parent_then_name("../escape", RunIdInvalid::Characters)]
#[case::slash("a/b", RunIdInvalid::Characters)]
#[case::backslash("a\\b", RunIdInvalid::Characters)]
#[case::absolute_unix("/etc", RunIdInvalid::Characters)]
#[case::windows_drive("c:", RunIdInvalid::Characters)]
#[case::windows_absolute("c:\\windows", RunIdInvalid::Characters)]
#[case::windows_unc("\\\\server\\share", RunIdInvalid::Characters)]
#[case::alternate_data_stream("a:stream", RunIdInvalid::Characters)]
#[case::trailing_dot("a.", RunIdInvalid::Characters)]
#[case::trailing_space("a ", RunIdInvalid::Characters)]
#[case::leading_dot(".hidden", RunIdInvalid::Characters)]
#[case::leading_hyphen("-a", RunIdInvalid::Characters)]
#[case::uppercase("Run", RunIdInvalid::Characters)]
#[case::non_ascii("été", RunIdInvalid::Characters)]
#[case::nul_byte("a\0b", RunIdInvalid::Characters)]
#[case::too_long(&"a".repeat(129), RunIdInvalid::Characters)]
#[case::windows_device("con", RunIdInvalid::Reserved)]
#[case::windows_device_with_extension("nul.txt", RunIdInvalid::Reserved)]
#[case::windows_serial_port("com1", RunIdInvalid::Reserved)]
#[case::windows_printer_port_with_extension("lpt9.log", RunIdInvalid::Reserved)]
fn a_run_id_that_could_name_another_folder_is_refused(
    #[case] text: &str,
    #[case] expected: RunIdInvalid,
) {
    assert_eq!(RunId::parse(text), Err(expected));
}

#[rstest]
#[case::sixteen_tags_of_sixty_four_characters(
    serde_json::json!(vec!["é".repeat(64); 16]),
    true
)]
#[case::seventeen_tags(serde_json::json!(vec!["t"; 17]), false)]
#[case::a_tag_of_sixty_five_characters(serde_json::json!(["t".repeat(65)]), false)]
fn tags_hold_at_most_sixteen_of_sixty_four_characters(
    #[case] tags: serde_json::Value,
    #[case] read: bool,
) {
    assert_eq!(serde_json::from_value::<Tags>(tags).is_ok(), read);
}

/// A program builds tags by the same bounds the wire reads them by.
#[rstest]
#[case::sixteen_tags_of_sixty_four_characters(vec!["é".repeat(64); 16], Ok(16))]
#[case::none(Vec::new(), Ok(0))]
#[case::seventeen_tags(vec!["t".to_owned(); 17], Err(TagsInvalid::TooMany))]
#[case::a_tag_of_sixty_five_characters(vec!["t".repeat(65)], Err(TagsInvalid::TooLong))]
fn tags_parse_by_the_bounds_the_wire_reads(
    #[case] tags: Vec<String>,
    #[case] expected: Result<usize, TagsInvalid>,
) {
    assert_eq!(Tags::parse(tags).map(|tags| tags.iter().count()), expected);
}

/// A warning of a crawl's is Foxhound's alone, so Collie's SDKs never type it;
/// any other is Collie's, which Foxhound contains (ADR 0084).
///
/// Covers: story:3-155
#[rstest]
#[case::trap_links_off("trap_links_off", Product::Foxhound)]
#[case::learning_off("learning_off", Product::Foxhound)]
#[case::provenance_off("provenance_off", Product::Foxhound)]
#[case::records_off("records_off", Product::Foxhound)]
#[case::block_classification_off("block_classification_off", Product::Foxhound)]
#[case::local_address_rotated_within_block("local_address_rotated_within_block", Product::Foxhound)]
#[case::fingerprint_none("fingerprint_none", Product::Collie)]
#[case::fingerprint_js_injection("fingerprint_js_injection", Product::Collie)]
#[case::fingerprint_basic("fingerprint_basic", Product::Collie)]
#[case::fingerprint_basic_fallback("fingerprint_basic_fallback", Product::Collie)]
#[case::browser_build_outdated("browser_build_outdated", Product::Collie)]
#[case::browser_build_third_party("browser_build_third_party", Product::Collie)]
#[case::browser_build_firefox("browser_build_firefox", Product::Collie)]
#[case::browser_build_plugin("browser_build_plugin", Product::Collie)]
#[case::human_input_fallback("human_input_fallback", Product::Collie)]
#[case::proxy_exit_may_change("proxy_exit_may_change", Product::Collie)]
#[case::filter_vendor_flagged("filter_vendor_flagged", Product::Collie)]
#[case::cpu_throttled("cpu_throttled", Product::Collie)]
#[case::storage_low("storage_low", Product::Collie)]
#[case::politeness_store_degraded("politeness_store_degraded", Product::Collie)]
#[case::clean_up_rules_stale("clean_up_rules_stale", Product::Collie)]
#[case::clean_up_rules_fallback("clean_up_rules_fallback", Product::Collie)]
#[case::block_resources_visible("block_resources_visible", Product::Collie)]
#[case::path_not_writable("path_not_writable", Product::Collie)]
#[case::file_write_failed("file_write_failed", Product::Collie)]
#[case::baseline_unavailable("baseline_unavailable", Product::Collie)]
#[case::plugin_quantity_above_maximum("plugin_quantity_above_maximum", Product::Collie)]
#[case::plugin_figures("plugin_figures", Product::Collie)]
fn a_warning_code_belongs_to_the_product_whose_runs_raise_it(
    #[case] code: &str,
    #[case] product: Product,
) {
    assert_eq!(
        crate::entries::product_of_value(SharedType::RunWarningCode, code),
        Some(product)
    );
}

/// A report as a newer Foxhound engine sends it: a warning code this reader
/// does not know, a crawl's parts and a Foxhound Limit scope and Usage kind.
fn newer_foxhound_report() -> serde_json::Value {
    serde_json::json!({
        "run_id": "books",
        "lifecycle": "finished",
        "verdict": "verified",
        "completeness": "complete",
        "started_at": "2026-10-22T08:00:00Z",
        "ended_at": "2026-10-22T08:05:00Z",
        "usage": {
            "model": {
                "calls": 0,
                "tokens": 0,
                "input_tokens": 0,
                "cache_read_tokens": 0,
                "cache_write_tokens": 0,
                "output_tokens": 0,
                "reasoning_tokens": 0,
                "price": null
            },
            "browser_time": 0,
            "proxy_bytes": 0,
            "requests": 3,
            "paid_fetches": [{ "currency": "USD", "amount": 20000 }],
            "other": [{ "usage_kind": "a_newer_kind", "amount": 7 }],
            "limits": [{
                "scope": { "kind": "partitioned_crawl", "id": "books" },
                "usage_kind": "paid_fetch",
                "currency": "USD",
                "amount": 1_000_000,
                "left": 980_000,
                "set_by": "person"
            }]
        },
        "warnings": [
            { "code": "a_code_from_a_newer_engine", "message": "Something new.", "sessions": ["s1"] },
            { "code": "trap_links_off", "message": "Trap links were followed." }
        ],
        "requests": { "succeeded": 3, "failed": 0, "skipped": 0, "blocked": 0, "pending": 0 },
        "records": [{ "dataset": "books", "committed": 3, "dropped": 0, "rejected": 0 }]
    })
}

/// Covers: story:3-155, story:3-157
#[test]
fn a_collie_reader_keeps_what_it_does_not_know_of_a_report() {
    let sent = newer_foxhound_report();

    let read: super::RunReport = serde_json::from_value(sent.clone()).unwrap();

    assert_eq!(read.warnings[0].code, "a_code_from_a_newer_engine");
    assert_eq!(read.usage.limits[0].scope.kind, "partitioned_crawl");
    assert_eq!(read.usage.limits[0].usage_kind, "paid_fetch");
    assert_eq!(serde_json::to_value(&read).unwrap(), sent);
}

/// Covers: story:3-154
#[test]
fn a_crawls_report_reads_what_every_report_holds_and_what_a_crawl_adds() {
    let sent = newer_foxhound_report();

    let read: crate::foxhound::report::CrawlRunReport =
        serde_json::from_value(sent.clone()).unwrap();

    assert_eq!(read.requests.as_ref().unwrap().succeeded, 3);
    assert!(read.report.kind_parts.is_empty());
    assert_eq!(serde_json::to_value(&read).unwrap(), sent);
}

/// A crawl's progress as a newer Foxhound engine sends it: with a wait
/// reason this engine does not know.
fn newer_crawl_progress() -> serde_json::Value {
    serde_json::json!({
        "run_id": "books",
        "position": 4_294_967_300_u64,
        "at": "2026-01-05T09:00:00Z",
        "lifecycle": "running",
        "writer": "live",
        "epoch": 1,
        "pauses": [],
        "last": false,
        "waiting": [
            {
                "reason": "domain_delay",
                "count": 2,
                "domains": [{ "domain": "shop.test", "next_admitted_at": "2026-01-05T09:00:01Z" }],
                "setting": { "key": "politeness.domain.delay", "value": 1000, "layer": "run" }
            },
            { "reason": "a_reason_from_a_newer_engine", "count": 1, "domains": [{ "domain": "a.test" }] }
        ],
        "idle": false
    })
}

/// Covers: story:3-162
#[test]
fn a_collie_reader_keeps_a_crawls_parts_of_its_progress() {
    let sent = newer_crawl_progress();

    let read: super::RunProgress = serde_json::from_value(sent.clone()).unwrap();

    assert_eq!(serde_json::to_value(&read).unwrap(), sent);
}

/// Covers: story:3-162
#[test]
fn a_crawls_progress_keeps_a_wait_reason_a_newer_engine_sends() {
    let sent = newer_crawl_progress();

    let read: crate::foxhound::progress::CrawlRunProgress =
        serde_json::from_value(sent.clone()).unwrap();

    assert_eq!(
        (
            read.waiting.as_ref().unwrap()[1].reason.as_str(),
            read.progress.kind_parts.is_empty(),
            serde_json::to_value(&read).unwrap(),
        ),
        ("a_reason_from_a_newer_engine", true, sent)
    );
}

/// A Run's model Usage whose price needs a price the table lacks is `null`,
/// never a partial sum, and each Model reference keeps its own counts and
/// price (ADR 0101).
#[test]
fn a_model_usage_without_a_whole_price_keeps_each_models_priced_part() {
    let counts = |calls: u64, price: serde_json::Value| {
        serde_json::json!({
            "calls": calls,
            "tokens": calls * 1500,
            "input_tokens": calls * 1200,
            "cache_read_tokens": calls * 1000,
            "cache_write_tokens": 0,
            "output_tokens": calls * 300,
            "reasoning_tokens": calls * 100,
            "price": price
        })
    };
    let mut sent = counts(3, serde_json::Value::Null);
    sent["by_model"] = serde_json::json!({
        "anthropic/claude-sonnet-4-5": counts(1, serde_json::json!({ "currency": "USD", "amount": 4200 })),
        "local/qwen3": counts(2, serde_json::Value::Null)
    });

    let read: super::ModelUsage = serde_json::from_value(sent.clone()).unwrap();

    assert_eq!(
        (
            read.counts.price.is_none(),
            read.by_model["anthropic/claude-sonnet-4-5"]
                .price
                .as_ref()
                .map(|price| price.amount),
            serde_json::to_value(&read).unwrap(),
        ),
        (true, Some(4200), sent)
    );
}

/// A Rust program's batch threshold goes on the wire as the value it reads
/// back from (ADRs 0083 and 0142).
#[rstest]
#[case::a_count(serde_json::json!(500))]
#[case::none(serde_json::json!("none"))]
fn a_batch_threshold_writes_the_value_it_is_read_from(#[case] value: serde_json::Value) {
    let read = super::durability::BatchThreshold::read(&value).unwrap();

    assert_eq!(serde_json::to_value(read).unwrap(), value);
}
