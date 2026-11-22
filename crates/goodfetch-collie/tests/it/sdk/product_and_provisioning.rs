//! What a Collie engine lacks (ADRs 0087, 0090 and 0095), the
//! `product-and-provisioning-errors` client conformance case in `linked`
//! mode: Rust's linked engine is the real one, installed from crates.io, so
//! it plays the case's Collie part, Foxhound's operation and setting
//! refused with `not_in_product` naming Foxhound's install command on
//! crates.io. A missing build's provisioning codes only the Scripted
//! engine plays, which the client crate's `product_and_provisioning`
//! reaches as a Daemon. TypeScript's `product-and-provisioning-errors.test.ts`
//! and Python's `test_product_and_provisioning_errors.py` are its twins.

use goodfetch_collie::{Collie, Error, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::connection::NotInProductDetails;
use goodfetch_test_support::{TestEnv, case_logs, real_time_runtime};
use serde_json::{Value, json};

use super::options;

/// Foxhound's install command on the linked engine's channel, crates.io.
const INSTALL: &str = "cargo install goodfetch-foxhound --locked --features executable";

/// The error `answer` holds, which must be Foxhound's `not_in_product`
/// for `requested`, naming its install command on crates.io whole.
fn assert_not_in_product(answer: Result<Value, Error>, requested: &str) {
    let error = match answer {
        Ok(value) => panic!("`{requested}` answered {value}"),
        Err(error) => error,
    };
    assert_eq!(
        (error.code(), error.group(), error.retryable()),
        (ErrorCode::NOT_IN_PRODUCT, ErrorGroup::Unavailable, false),
        "{error:?}"
    );
    let fix = format!(
        "Run it with Foxhound, which contains everything Collie does (`{INSTALL}`), or connect to a Foxhound engine."
    );
    assert_eq!(error.fix(), Some(fix.as_str()));
    assert_eq!(
        serde_json::to_value(error.details::<NotInProductDetails>().unwrap()).unwrap(),
        json!({
            "product": "collie",
            "product_version": env!("CARGO_PKG_VERSION"),
            "needs": "foxhound",
            "requested": requested,
            "install": INSTALL,
            "fix": fix,
        })
    );
}

/// Foxhound's operation and a Run's start setting Foxhound's setting, on
/// the linked engine, each fail with `not_in_product` naming Foxhound and
/// its install command on crates.io.
/// Covers: story:11-10, story:11-189
#[test]
fn foxhounds_operation_and_setting_fail_on_the_linked_engine_naming_foxhounds_cargo_install() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "product-and-provisioning-errors",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();

    let (operation, setting) = real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let operation = collie
            .call("crawl.start")
            .params(json!({"seeds": [{"url": "https://shop.test/", "label": "page"}]}))
            .await;
        let setting = collie
            .call("run.start")
            .params(json!({"settings": {"crawl": {"max_requests": 10}}}))
            .await;
        collie.close().await.unwrap();
        (operation, setting)
    });

    assert_not_in_product(operation, "crawl.start");
    assert_not_in_product(setting, "crawl.max_requests");
}
