//! A linked connection whose `hello` needs a Product its engine does not
//! contain, as a Foxhound program that met a Collie engine would: the
//! engine refuses it at connect with `not_in_product` (ADRs 0087, 0090 and
//! 0095). A zero `max_listener_queue` is refused before the engine starts
//! (ADR 0081). A linked connection's close returns once its engine has stopped
//! (ADR 0078).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::sync::{Arc, OnceLock};

use goodfetch_collie_client::ErrorCode;
use goodfetch_engine::runs::ReportPart;
use goodfetch_engine::{Channel, Contribution};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::NotInProductDetails;
use goodfetch_protocol::run::RunReport;
use goodfetch_protocol::settings::SettingInvalidDetails;
use goodfetch_test_support::{TestEnv, real_time_runtime};

use super::{Composed, ConnectOptions, ProductConnection, version};
use crate::EngineChoice;

/// The engine fails `hello` before any other call, with its Product and
/// version, the Product needed, and a fix that names the install command
/// of the crates.io channel the crate came from.
/// Covers: story:11-1005, error:not_in_product
#[test]
fn a_connection_that_needs_foxhound_fails_on_a_collie_engine_with_not_in_product() {
    let env = TestEnv::new();
    let options = ConnectOptions::default().environment(env.env_os());

    let failed = real_time_runtime().block_on(ProductConnection::connect_needing(
        Composed::COLLIE,
        Product::Foxhound,
        options,
    ));

    let Err(error) = failed else {
        panic!("connected: {failed:?}");
    };
    assert_eq!(error.code(), ErrorCode::NOT_IN_PRODUCT);
    let details: NotInProductDetails = error.details().unwrap();
    let install = Channel::CratesIo.install_command(Product::Foxhound);
    assert_eq!(
        (
            details.product,
            &details.product_version,
            details.needs,
            details.requested.as_deref(),
            details.install.as_deref(),
        ),
        (
            Product::Collie,
            &version(),
            Product::Foxhound,
            None,
            Some(install.as_str()),
        )
    );
    assert!(details.fix.contains(&install), "{details:?}");
}

/// A zero `max_listener_queue`, which would lose every event, fails the
/// connection with `setting_invalid` naming it, before the engine starts.
/// Covers: error:setting_invalid
#[test]
fn a_zero_max_listener_queue_fails_the_connection_with_setting_invalid() {
    let env = TestEnv::new();
    let options = ConnectOptions::default()
        .environment(env.env_os())
        .max_listener_queue(0);

    let failed = real_time_runtime().block_on(ProductConnection::connect_needing(
        Composed::COLLIE,
        Product::Collie,
        options,
    ));

    let Err(error) = failed else {
        panic!("connected: {failed:?}");
    };
    let details: SettingInvalidDetails = error.details().unwrap();
    assert_eq!(
        (error.code(), details.key.as_str()),
        (ErrorCode::SETTING_INVALID, "max_listener_queue")
    );
}

/// A linked connection's close returns only once its engine has stopped:
/// the engine, which the task serving its Connection owns, is dropped by
/// then.
/// Covers: story:11-19
#[test]
fn a_linked_close_returns_once_its_engine_has_stopped() {
    let env = TestEnv::new();
    let options = ConnectOptions::default()
        .environment(env.env_os())
        .engine(EngineChoice::Linked);

    let stopped = real_time_runtime().block_on(async {
        let linked = ProductConnection::connect(MARKED, options).await.unwrap();
        let _run = linked.connection().start().await.unwrap();
        linked.close().await.unwrap();
        ENGINE_DROPPED.get().is_some()
    });

    assert!(stopped, "the close returned before its engine stopped");
}

/// Whether [`MARKED`]'s engine has been dropped.
static ENGINE_DROPPED: OnceLock<()> = OnceLock::new();

/// Collie's engine, holding a [`DropMark`] that only it and its Runs hold.
const MARKED: Composed = Composed {
    product: Product::Collie,
    composition: || {
        crate::composition()
            .with(Contribution::new("drop-mark").with_report_part(Arc::new(DropMark)))
    },
};

/// A report part that adds nothing and sets [`ENGINE_DROPPED`] when it is
/// dropped.
struct DropMark;

impl ReportPart for DropMark {
    fn fill(
        &self,
        _connection: &rusqlite::Connection,
        _report: &mut RunReport,
    ) -> rusqlite::Result<()> {
        Ok(())
    }
}

impl Drop for DropMark {
    fn drop(&mut self) {
        let _first = ENGINE_DROPPED.set(());
    }
}
