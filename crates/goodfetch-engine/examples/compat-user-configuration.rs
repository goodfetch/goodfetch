//! User configuration's writer for the Compatibility corpus (ADR 0066):
//! writes `configuration.toml`, which sets every key of either Product,
//! and, for a key whose values have a shape, one more file for each further
//! example of it, so that every field of the shape is given, each through
//! the engine's rewrite; checks that both Products' engines read each, and
//! puts them in the folder given as the only argument. `xtask compat write`
//! runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

#[path = "support/settings_corpus.rs"]
mod settings_corpus;

use std::path::PathBuf;

use goodfetch_engine::settings::user_configuration;
use goodfetch_protocol::catalog::{Product, Setting};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::home::USER_CONFIGURATION_FILE;
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
    let version = ProductVersion::parse(env!("CARGO_PKG_VERSION"))
        .expect("invariant: the crate's version is a Product version");
    let files = std::iter::once((USER_CONFIGURATION_FILE.to_owned(), text)).chain(
        settings_corpus::shaped_examples("settings", any_key)
            .into_iter()
            .map(|(name, text)| (format!("{name}.toml"), text)),
    );
    for (name, text) in files {
        for product in [Product::Collie, Product::Foxhound] {
            user_configuration(SettingsLayer::UserConfiguration, &text, product, &version)
                .expect("invariant: both Products read what the engine wrote");
        }
        std::fs::write(output.join(name), text).expect("invariant: the output folder is writable");
    }
}
