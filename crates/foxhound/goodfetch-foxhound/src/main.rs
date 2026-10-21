//! Foxhound's executable, `goodfetch-foxhound` (ADR 0090): Collie's
//! executable entry, given Foxhound's composition, a list that starts from
//! Collie's (ADR 0087).

use std::process::ExitCode;

use goodfetch_collie::executable::{Start, main as start};
use goodfetch_protocol::catalog::Product;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    start(&Start {
        product: Product::Foxhound,
        version: env!("CARGO_PKG_VERSION"),
        composition: goodfetch_foxhound::composition,
        arguments: std::env::args_os().collect(),
        environment: std::env::vars_os().collect(),
    })
}
