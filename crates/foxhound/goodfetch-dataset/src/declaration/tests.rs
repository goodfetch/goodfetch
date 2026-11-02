use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::foxhound::dataset::DatasetDeclaration;
use serde_json::{Value, json};

use super::Datasets;

/// The Datasets of a crawl declaring `books`, under a schema requiring
/// `title`, and `free`, under none.
fn declared() -> Datasets {
    Datasets::declare(&[
        DatasetDeclaration {
            name: "books".to_owned(),
            schema: Some(json!({ "type": "object", "required": ["title"] })),
            key: Vec::new(),
        },
        DatasetDeclaration {
            name: "free".to_owned(),
            schema: None,
            key: Vec::new(),
        },
    ])
    .unwrap()
}

fn pushed(dataset: &str, data: Value) -> PushedRecord {
    PushedRecord {
        dataset: dataset.to_owned(),
        data,
    }
}

/// The validator builds and the validations `work` counted.
fn counted(work: &CountedWork) -> (u64, u64) {
    (
        work.read(Row::RecordValidatorBuilds),
        work.read(Row::RecordValidations),
    )
}

/// F4's check is counted on both of its ways in, and a Dataset with no schema
/// does neither; declaring a schema, once as the crawl starts, is no Record's
/// work.
#[test]
fn a_records_check_counts_its_validator_and_its_validation_only_under_a_schema() {
    let datasets = declared();
    let work = CountedWork::default();
    assert_eq!(counted(&work), (0, 0));

    datasets
        .check(vec![pushed("books", json!({ "title": "Dune" }))], &work)
        .unwrap();
    assert_eq!(counted(&work), (1, 1));

    datasets
        .check_pushed(pushed("books", json!({ "title": "Emma" })), &work)
        .unwrap();
    assert_eq!(counted(&work), (2, 2));

    datasets
        .check_pushed(pushed("free", json!({ "title": "Kim" })), &work)
        .unwrap();
    assert_eq!(counted(&work), (2, 2));
}

/// A refused push's rejection names its issues by checking the Record again:
/// that second check is counted, so the row sees it.
#[test]
fn a_refused_push_counts_the_second_check_its_rejection_makes() {
    let work = CountedWork::default();

    let refused = declared().check_pushed(pushed("books", json!({})), &work);

    assert!(refused.is_err());
    assert_eq!(counted(&work), (2, 2));
}
