//! goodfetch-clean-up-rules's integration tests: what the interception does
//! with a request, through the crate's public interface, in process, on the
//! embedded Clean-up rules (ADR 0047), and what loading them holds.

mod consent;
mod hiding;
mod matching;
#[cfg(target_os = "linux")]
mod memory;
mod page_load;
mod removal;
mod versions;
