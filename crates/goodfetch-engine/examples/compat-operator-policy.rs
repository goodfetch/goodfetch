//! The Operator policy's writer for the Compatibility corpus (ADR 0066):
//! writes `operator-policy.toml`, a managed policy that sets and locks
//! every key of either Product, caps every count and duration and names an
//! install Limit, through the engine's rewrite, checks that the engine
//! reads it, and puts it in the folder given as the only argument. `xtask
//! compat write` runs it.
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
    matches!(
        setting.kind,
        ValueKind::Count | ValueKind::CountUpTo(_) | ValueKind::Duration
    )
}

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let mut text = settings_corpus::every_key(&[
        ("settings", any_key),
        ("lock", any_key),
        ("cap", bounded_key),
    ]);
    // A managed policy names an install Limit (ADR 0041).
    text.push_str("\n[managed]\ninstall_limits = [{ usage_kind = \"paid_solves\", currency = \"USD\", period = { every = \"month\", time_zone = \"UTC\" }, amount = 20000000 }]\n");
    let version = ProductVersion::parse(env!("CARGO_PKG_VERSION"))
        .expect("invariant: the crate's version is a Product version");
    let files = std::iter::once((OPERATOR_POLICY_FILE.to_owned(), text)).chain(
        ["settings", "lock"]
            .into_iter()
            .flat_map(|table| settings_corpus::shaped_examples(table, any_key))
            .map(|(name, text)| (format!("{name}.toml"), text)),
    );
    for (name, text) in files {
        operator_policy(&text, Product::Foxhound, &version)
            .expect("invariant: the engine reads what it wrote");
        std::fs::write(output.join(name), text).expect("invariant: the output folder is writable");
    }
}
