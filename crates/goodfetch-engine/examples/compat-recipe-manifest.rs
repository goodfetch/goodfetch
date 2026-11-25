//! The Recipe manifest's writer for the Compatibility corpus (ADR 0066):
//! writes a manifest of each entry, a program on its runtime with settings
//! it declares and exposes, a step list and a `CrawlDeclaration`, checks
//! that the engine reads each, its settings included, and puts them in the
//! folder given as the only argument. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::path::PathBuf;

use goodfetch_engine::recipe::manifest;
use goodfetch_engine::settings::Recipe;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::recipe::RECIPE_MANIFEST_FILE;

/// A program on Node, which declares a navigation timeout and lets its
/// users raise it within bounds or turn the audit chain on.
const PROGRAM: &str = r#"version = "0.1"
name = "price-watch"
product = "collie"

[entry]
program = "src/main.ts"

[runtime]
language = "node"
version = "24"

[settings.timeout]
navigation = 20000

[expose."timeout.navigation"]
minimum = 5000
maximum = 60000

[expose."security.audit_chain"]
one_of = [true]
"#;

/// A step list, which names no runtime.
const STEPS: &str = r#"version = "0.1"
name = "sign-in-check"
product = "collie"

[entry]
steps = "sign-in.steps.json"
"#;

/// A `CrawlDeclaration`, which only Foxhound runs.
const CRAWL: &str = r#"version = "0.1"
name = "catalog-crawl"
product = "foxhound"

[entry]
crawl = "catalog.crawl.json"
"#;

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    for (name, text) in [
        (RECIPE_MANIFEST_FILE, PROGRAM),
        ("steps.toml", STEPS),
        ("crawl.toml", CRAWL),
    ] {
        let read = manifest(text).expect("invariant: the engine reads what it wrote");
        Recipe::read(&read, Product::Foxhound)
            .expect("invariant: the engine reads the settings it wrote");
        std::fs::write(output.join(name), text).expect("invariant: the output folder is writable");
    }
}
