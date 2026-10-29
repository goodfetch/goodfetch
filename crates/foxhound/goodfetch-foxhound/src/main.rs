//! Foxhound's executable, `goodfetch-foxhound` (ADR 0090): Collie's
//! executable entry, given Foxhound's composition, a list that starts from
//! Collie's (ADR 0087), with its Console build (ADR 0057).

mod console_build;

use std::process::ExitCode;

use goodfetch_collie::executable::{Start, main as start};
use goodfetch_protocol::catalog::Product;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[expect(
    clippy::disallowed_methods,
    reason = "main is the one place that reads the environment (ADR 0059)"
)]
fn main() -> ExitCode {
    start(&Start {
        product: Product::Foxhound,
        version: env!("CARGO_PKG_VERSION"),
        // A release build records its release date; any other build has
        // none, and never prints the age notice (ADR 0030).
        release_date: option_env!("GOODFETCH_RELEASE_DATE"),
        composition: console_build::composition,
        arguments: std::env::args_os().collect(),
        environment: std::env::vars_os().collect(),
    })
}
