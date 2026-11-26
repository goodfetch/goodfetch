use std::io;
use std::path::PathBuf;

use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;

use super::refusal;
use crate::plugin_install::{InstallError, PullFailure};
use crate::plugin_oci::Reference;
use crate::plugin_store::{PluginStore, StoreError};

const REFERENCE: &str = "registry.example.com/goodfetch-plugin-kettle@sha256:1111111111111111111111111111111111111111111111111111111111111111";

/// Each way an install fails answers its named error, the details naming
/// what the caller acts on: the layer that set `egress.offline`, the
/// registry that refused, the version installed, or the store's folder.
///
/// Covers: story:7-81
#[rstest]
#[case::offline(
    InstallError::Offline(SettingsLayer::UserConfiguration),
    "artifact_unavailable_offline",
    "/limit/layer",
    "user_configuration"
)]
#[case::pull(
    InstallError::Pull(PullFailure::Absent),
    "artifact_download_failed",
    "/host",
    "registry.example.com"
)]
#[case::installed(
    InstallError::Store(StoreError::Installed { name: "kettle".to_owned(), version: "1.0.0".to_owned() }),
    "plugin_installed",
    "",
    ""
)]
#[case::unwritable(
    InstallError::Staging(io::Error::from(io::ErrorKind::StorageFull)),
    "file_write_failed",
    "/path",
    "/home/data/plugins"
)]
fn an_install_that_fails_answers_its_named_error(
    #[case] error: InstallError,
    #[case] code: &str,
    #[case] pointer: &str,
    #[case] value: &str,
) {
    let reference: Reference = REFERENCE.parse().unwrap();
    let store = PluginStore::new(PathBuf::from("/home/data/plugins"));

    let answered = refusal(error, &reference, &store);

    assert_eq!(answered.code(), code);
    if !pointer.is_empty() {
        let details = serde_json::Value::Object(answered.details().unwrap().clone());
        assert_eq!(details.pointer(pointer).unwrap(), value, "{details}");
    }
}
