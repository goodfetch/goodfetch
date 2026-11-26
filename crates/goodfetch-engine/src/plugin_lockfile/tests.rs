use goodfetch_protocol::plugin::lockfile::{LockedPlugin, PluginLockfile};
use rstest::rstest;

use super::{read, text};

const KETTLE: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

/// A lockfile the engine wrote reads back as it was, its optional fields
/// left out when unset.
/// Covers: story:7-5
#[test]
fn a_written_lockfile_reads_back_as_it_was() {
    let mut lockfile = PluginLockfile::empty();
    for (name, signer) in [
        ("kettle", Some("https://example.com/release")),
        ("archive", None),
    ] {
        lockfile
            .lock(LockedPlugin {
                name: name.to_owned(),
                version: "1.0.0".to_owned(),
                digest: KETTLE.to_owned(),
                signer: signer.map(str::to_owned),
                approvals: None,
            })
            .unwrap();
    }

    let written = text(&lockfile);

    assert_eq!(
        (read(&written).unwrap(), written.matches("signer").count()),
        (lockfile, 1)
    );
}

/// A newer lockfile is refused for its version, before a field this engine
/// does not know; text that is no lockfile is refused as such.
/// Covers: story:7-5
#[rstest]
#[case::newer_minor(
    "version = \"0.2\"\n\n[[plugin]]\nname = \"kettle\"\nmirror = \"x\"\n",
    "format_version_unsupported"
)]
#[case::newer_major("version = \"1.0\"\n", "format_version_unsupported")]
#[case::not_toml("version = \n", "plugin_lockfile_invalid")]
#[case::no_version("[[plugin]]\nname = \"kettle\"\n", "plugin_lockfile_invalid")]
#[case::unknown_field("version = \"0.1\"\nmirror = \"x\"\n", "plugin_lockfile_invalid")]
fn a_lockfile_this_engine_does_not_read_is_refused(#[case] text: &str, #[case] code: &str) {
    assert_eq!(read(text).unwrap_err().code(), code);
}
