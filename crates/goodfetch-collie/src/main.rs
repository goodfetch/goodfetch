//! Collie's executable, `goodfetch-collie` (ADR 0090): Collie's composition,
//! a list, passed to the engine at start (ADR 0087).

use std::process::ExitCode;

use goodfetch_collie::executable::{OwnCommands, Skills, Start, main as start};
use goodfetch_engine::heap::Counting;
use goodfetch_protocol::catalog::Product;

// mimalloc, counted, so that `daemon.status` answers the live heap (#1734).
#[global_allocator]
static ALLOCATOR: Counting<mimalloc::MiMalloc> = Counting::new(mimalloc::MiMalloc);

#[expect(
    clippy::disallowed_methods,
    reason = "main is the one place that reads the environment (ADR 0059)"
)]
fn main() -> ExitCode {
    start(&Start {
        product: Product::Collie,
        version: env!("CARGO_PKG_VERSION"),
        // A release build records its release date; any other build has
        // none, and never prints the age notice (ADR 0030).
        release_date: option_env!("GOODFETCH_RELEASE_DATE"),
        composition: goodfetch_collie::composition,
        commands: OwnCommands::COLLIE,
        skills: Skills::COLLIE,
        arguments: std::env::args_os().collect(),
        environment: std::env::vars_os().collect(),
    })
}
