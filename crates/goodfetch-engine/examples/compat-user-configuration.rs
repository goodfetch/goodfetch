//! User configuration's writer for the Compatibility corpus (ADR 0066):
//! writes `configuration.toml`, which sets every key of either Product,
//! through the engine's rewrite, checks that both Products' engines read
//! it, and puts it in the folder given as the only argument. `xtask compat
//! write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

#[path = "support/settings_corpus.rs"]
mod settings_corpus;

use std::path::PathBuf;

use goodfetch_engine::settings::{USER_CONFIGURATION_FILE, configuration};
use goodfetch_protocol::catalog::{Product, Setting};
use goodfetch_protocol::settings::SettingsLayer;

fn any_key(_: &Setting) -> bool {
    true
}

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let text = settings_corpus::every_key(&[("settings", any_key)]);
    for product in [Product::Collie, Product::Foxhound] {
        configuration(SettingsLayer::UserConfiguration, &text, product)
            .expect("invariant: both Products read what the engine wrote");
    }
    std::fs::write(output.join(USER_CONFIGURATION_FILE), text)
        .expect("invariant: the output folder is writable");
}
