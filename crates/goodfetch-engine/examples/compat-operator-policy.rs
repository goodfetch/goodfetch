//! The Operator policy's writer for the Compatibility corpus (ADR 0066):
//! writes `operator-policy.toml`, a policy that sets and locks every key
//! of either Product and caps every count and duration, through the
//! engine's rewrite, checks that the engine reads it, and puts it in the
//! folder given as the only argument. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

#[path = "support/settings_corpus.rs"]
mod settings_corpus;

use std::path::PathBuf;

use goodfetch_engine::settings::{OPERATOR_POLICY_FILE, operator_policy};
use goodfetch_protocol::catalog::{Product, Setting, ValueKind};
use goodfetch_protocol::connection::ProductVersion;

fn any_key(_: &Setting) -> bool {
    true
}

fn bounded_key(setting: &Setting) -> bool {
    matches!(setting.kind, ValueKind::Count | ValueKind::Duration)
}

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let text = settings_corpus::every_key(&[
        ("settings", any_key),
        ("lock", any_key),
        ("cap", bounded_key),
    ]);
    let version = ProductVersion::parse(env!("CARGO_PKG_VERSION"))
        .expect("invariant: the crate's version is a Product version");
    operator_policy(&text, Product::Foxhound, &version)
        .expect("invariant: the engine reads what it wrote");
    std::fs::write(output.join(OPERATOR_POLICY_FILE), text)
        .expect("invariant: the output folder is writable");
}
