#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::any::Any;

use goodfetch_protocol::foxhound::crawl::{
    HandlerFailedDetails, Seed as SeedRequest, SitemapRule, SitemapSeed,
};
use goodfetch_protocol::foxhound::dataset::{
    DatasetDeclaration as WireDataset, DatasetDuplicateDetails, DatasetKeyInvalidDetails,
    KeyRefused,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Crawl, CrawlContext, DatasetDeclaration, code_free_params, panicked};
use crate::{Schema, Seed, error_code, schemars};

/// The sitemap a test's sitemap seeds read.
const SITEMAP: &str = "https://shop.test/sitemap.xml";

/// A Record whose title it may lack.
#[derive(schemars::JsonSchema, Serialize, Deserialize)]
struct Edition {
    isbn: String,
    title: Option<String>,
}

/// The editions Dataset, typed by its derived schema and keyed by `key`.
fn editions(key: &str) -> DatasetDeclaration<Edition> {
    DatasetDeclaration::new("editions")
        .schema(&Schema::derived())
        .key(&[key])
}

/// A second Dataset of one name, and a key on a field the schema does not
/// require, are refused as each is declared, before any connection; the
/// crawl declares the others alone, a Dataset without a schema keyed by
/// any field.
///
/// Covers: story:11-1057, error:dataset_duplicate, error:dataset_key_invalid
#[test]
fn a_second_dataset_and_a_key_its_schema_does_not_require_are_refused_as_declared() {
    let mut crawl = Crawl::new();
    crawl.dataset::<Value>("books").unwrap();

    let duplicate = crawl.dataset::<Edition>("books").unwrap_err();
    let not_required = crawl.dataset(editions("title")).unwrap_err();
    crawl.dataset(editions("isbn")).unwrap();
    crawl
        .dataset(DatasetDeclaration::<Value>::new("notes").key(&["page"]))
        .unwrap();

    assert_eq!(
        (duplicate.details(), not_required.details()),
        (
            Some(DatasetDuplicateDetails {
                dataset: "books".to_owned()
            }),
            Some(DatasetKeyInvalidDetails {
                dataset: "editions".to_owned(),
                refused: KeyRefused::NotRequired {
                    field: "title".to_owned()
                },
            }),
        )
    );
    // What `crawl.start` declares the Datasets with.
    assert_eq!(
        crawl.datasets,
        [
            WireDataset {
                name: "books".to_owned(),
                schema: None,
                key: Vec::new(),
            },
            WireDataset {
                name: "editions".to_owned(),
                schema: Some(Schema::<Edition>::derived().wire_schema().clone()),
                key: vec!["isbn".to_owned()],
            },
            WireDataset {
                name: "notes".to_owned(),
                schema: None,
                key: vec!["page".to_owned()],
            },
        ]
    );
}

/// What a handler's panic with `payload` fails its attempt with.
fn details_of(payload: &(dyn Any + Send)) -> HandlerFailedDetails {
    let error = panicked("page", payload);
    assert_eq!(error.code(), error_code::HANDLER_FAILED);
    error
        .details()
        .expect("invariant: a panic's error details its exception")
}

#[test]
fn a_handlers_panic_is_handler_failed_of_the_exception_type_panic() {
    let details = details_of(&"the shelf is gone");

    assert_eq!(
        details,
        HandlerFailedDetails {
            exception_type: "panic".to_owned(),
            exception_message: "the shelf is gone".to_owned(),
            exception_stack: String::new(),
        }
    );
}

/// A crawl that attaches a handler to one Label and none to another is
/// refused before its start is sent, with `label_unhandled`. The SDK
/// raises it itself, so no exercise record of a test that connects can show it (ADR
/// 0072): this test, which never reaches `rpc`, declares it.
///
/// Covers: error:label_unhandled
#[test]
fn a_label_left_unhandled_beside_a_handled_one_refuses_the_start_with_label_unhandled() {
    let mut crawl = Crawl::new();
    let page = crawl.label::<Value>("page").unwrap();
    let _detail = crawl.label::<Value>("detail").unwrap();
    crawl
        .handle(&page, |_ctx: CrawlContext<Value>| async { Ok(()) })
        .unwrap();

    let refused = crawl.start_params(Vec::new()).unwrap_err();

    assert_eq!(refused.code(), error_code::LABEL_UNHANDLED, "{refused}");
}

/// A sitemap seed whose rules name their Labels by name goes to the
/// engine as given from a code-free crawl, and a `Crawl` takes it only
/// when it declares each rule's Label: one naming another is refused with
/// `label_unknown` before anything is sent.
///
/// Covers: story:11-1069
#[test]
fn a_named_sitemap_seed_starts_a_code_free_crawl_or_a_crawl_declaring_its_labels() {
    let named = || Seed::named_sitemap(SITEMAP, [("*/p/*", "product")]);
    let mut crawl = Crawl::new();
    let product = crawl.label::<()>("product").unwrap();
    crawl
        .handle(&product, |_ctx: CrawlContext<()>| async { Ok(()) })
        .unwrap();
    let mut other = Crawl::new();
    let listing = other.label::<()>("listing").unwrap();
    other
        .handle(&listing, |_ctx: CrawlContext<()>| async { Ok(()) })
        .unwrap();

    let code_free = code_free_params(vec![named()]).unwrap();
    let declared = crawl.start_params(vec![named()]).unwrap();
    let refused = other.start_params(vec![named()]).unwrap_err();

    let sitemap = SeedRequest::Sitemap(SitemapSeed {
        sitemap: SITEMAP.to_owned(),
        rules: vec![SitemapRule {
            pattern: "*/p/*".to_owned(),
            label: "product".to_owned(),
        }],
    });
    assert_eq!(
        (
            code_free.seeds.iter().collect::<Vec<_>>(),
            declared.seeds.iter().collect::<Vec<_>>(),
            refused.code()
        ),
        (vec![&sitemap], vec![&sitemap], error_code::LABEL_UNKNOWN),
        "{refused}"
    );
}

/// A sitemap seed holding a `Crawl`'s Labels belongs to that crawl: a
/// code-free crawl refuses it with `invalid_params`.
///
/// Covers: story:11-1069
#[test]
fn a_code_free_crawl_refuses_a_sitemap_seed_holding_labels() {
    let mut crawl = Crawl::new();
    let product = crawl.label::<()>("product").unwrap();

    let refused =
        code_free_params(vec![Seed::sitemap(SITEMAP, [("*/p/*", &product)])]).unwrap_err();

    assert_eq!(refused.code().as_str(), "invalid_params", "{refused}");
}

#[test]
fn a_panic_formatted_or_of_no_text_keeps_its_message_or_says_it_has_none() {
    let formatted = details_of(&format!("shelf {}", 3));
    let opaque = details_of(&7_u8);

    assert_eq!(
        [formatted.exception_message, opaque.exception_message],
        ["shelf 3", "a panic of no message"]
    );
}
