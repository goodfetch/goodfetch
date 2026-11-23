//! A suite's kept document: the Release suite's cells keep each document
//! `just test` gathers, and the Feature report reads its cases from the
//! results store (ADR 0072).

use goodfetch_protocol::result_document::{
    CapabilityFacts, CaseCheck, CaseEntry, CaseFacts, ConformanceDocument, ConformanceHeader,
    Evidence, LoadedDocument, ProvingGroundCopy, Suite, Target, TargetIdentity, read,
};
use goodfetch_test_support::TestEnv;

use crate::{keep_in, platform};

fn document() -> ConformanceDocument {
    let header = ConformanceHeader {
        suite: Suite {
            name: "page".to_owned(),
            version: "1".to_owned(),
        },
        target: Target {
            name: "goodfetch-collie".to_owned(),
            identity: TargetIdentity::Version("0.0.0".to_owned()),
            product: None,
        },
        platform: platform(),
        runtime: "rust".to_owned(),
        proving_ground: ProvingGroundCopy {
            version: "0.0.0".to_owned(),
            seed: 7,
        },
        online: false,
        gate: false,
    };
    ConformanceDocument::new(
        header,
        vec![CapabilityFacts {
            name: "navigation".to_owned(),
            cases: vec![CaseFacts {
                id: "goto-answers-the-documents-response".to_owned(),
                covers: Vec::new(),
                entries: vec![CaseEntry {
                    mode: None,
                    blocking: None,
                    unreachable: Vec::new(),
                    checks: vec![CaseCheck {
                        name: "the response is the document's".to_owned(),
                        passed: true,
                    }],
                    evidence: Evidence::default(),
                }],
            }],
        }],
    )
}

#[test]
fn a_kept_document_reads_back_as_the_suite_gave_it() {
    let env = TestEnv::new();
    let written = document();

    let kept = keep_in(Some(env.folder()), &written, "page-collie-attached").unwrap();

    assert_eq!(
        kept.file_name().unwrap(),
        "conformance-page-collie-attached.json"
    );
    match read(&std::fs::read(&kept).unwrap()).unwrap() {
        LoadedDocument::Conformance(read) => assert_eq!(read, written),
        LoadedDocument::Journey(read) => panic!("a conformance document was read as {read:?}"),
    }
}

#[test]
fn outside_just_test_nothing_is_kept() {
    let kept = keep_in(None, &document(), "page-collie-attached");

    assert_eq!(kept, None);
}
