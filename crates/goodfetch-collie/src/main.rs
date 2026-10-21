//! Collie's executable, `goodfetch-collie` (ADR 0090): Collie's composition,
//! a list, passed to the engine at start (ADR 0087).

use std::process::ExitCode;

use goodfetch_collie::executable::{Start, main as start};
use goodfetch_protocol::catalog::Product;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    start(&Start {
        product: Product::Collie,
        version: env!("CARGO_PKG_VERSION"),
        composition: goodfetch_collie::composition,
        arguments: std::env::args_os().collect(),
        environment: std::env::vars_os().collect(),
    })
}
