//! The Plugin lockfile's writer for the Compatibility corpus (ADR 0066): a
//! lockfile that fills every field, two Plugins pinned, one with its signer
//! and approvals and one without, built from the protocol's types, written
//! as the engine writes it and read back through the engine's reader, then
//! put into the folder given as the only argument as
//! `plugin-lockfile.toml`: `goodfetch-plugins.lock`'s text under the `.toml`
//! extension `xtask compat check` reads a TOML format's documents by.
//! `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::path::PathBuf;

use goodfetch_engine::plugin_lockfile::{read, text};
use goodfetch_protocol::plugin::lockfile::{LockedPlugin, PluginLockfile};

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let mut lockfile = PluginLockfile::empty();
    for plugin in [
        LockedPlugin {
            name: "kettle".to_owned(),
            version: "1.2.0".to_owned(),
            digest: format!("sha256:{}", "1".repeat(64)),
            signer: Some(
                "https://github.com/example/goodfetch-plugin-kettle/.github/workflows/release.yml@refs/tags/v1.2.0"
                    .to_owned(),
            ),
            approvals: Some(format!("sha256:{}", "2".repeat(64))),
        },
        LockedPlugin {
            name: "archive".to_owned(),
            version: "0.3.1".to_owned(),
            digest: format!("sha256:{}", "3".repeat(64)),
            signer: None,
            approvals: None,
        },
    ] {
        lockfile
            .lock(plugin)
            .expect("invariant: the corpus pins each name once");
    }
    let written = text(&lockfile);
    let read = read(&written).expect("invariant: the corpus's lockfile is valid");
    assert_eq!(read, lockfile, "the lockfile reads back as it was written");
    std::fs::write(output.join("plugin-lockfile.toml"), written)
        .expect("invariant: the output folder is writable");
}
