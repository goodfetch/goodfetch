//! A Foxhound-only key in a Collie Run's settings (ADRs 0086 and 0087):
//! Collie's crate names no such key, which the compile-fail case of the
//! client crate shows, and a program that takes one from Foxhound's crate
//! has its Run refused when it opens, before the Run begins, with
//! `not_in_product` on a Collie engine and `setting_not_applicable` on a
//! Foxhound engine, whose crawls alone take the key.
//!
//! Covers: story:11-1092

use goodfetch_foxhound::settings::{Settings, crawl};
use goodfetch_foxhound::{Connection, ErrorCode, Foxhound};
use goodfetch_test_support::{TestEnv, real_time_runtime};

use super::{collie_options, options};

/// A crawl's key, which only Foxhound's crawls take.
fn a_crawls_key() -> Settings {
    Settings::new().set(crawl::KEEP_URL_FRAGMENT, true)
}

/// The code the Run's start fails with; the connection then opens and
/// finishes the next Run.
async fn refused(connection: &impl Connection) -> ErrorCode {
    let refused = connection
        .start()
        .settings(a_crawls_key())
        .await
        .expect_err("a Collie Run takes no crawl's key");
    let next = connection.start().await.expect("the next Run opens");
    next.finish().await.expect("the next Run finishes");
    refused.code()
}

#[test]
fn a_crawls_key_refuses_a_collie_run_on_either_products_engine() {
    let env = TestEnv::new();
    real_time_runtime().block_on(async {
        let collie = goodfetch_collie::Collie::connect(collie_options(&env))
            .await
            .unwrap();
        let foxhound = Foxhound::connect(options(&env)).await.unwrap();

        let refusals = [refused(&collie).await, refused(&foxhound).await];
        Connection::close(&collie).await.unwrap();
        Connection::close(&foxhound).await.unwrap();

        assert_eq!(
            refusals,
            [ErrorCode::NOT_IN_PRODUCT, ErrorCode::SETTING_NOT_APPLICABLE]
        );
    });
}
