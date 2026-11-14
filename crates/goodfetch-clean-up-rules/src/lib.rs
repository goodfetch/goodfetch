//! The Clean-up rules (spec #577; ADRs 0047 and 0057): the embedded Filter
//! lists, compiled at build time into `adblock` engines from the binary's
//! own `adblock` version and loaded from their zstd form once, the
//! never-block list, and the redirect stand-ins with their alias table.
//! [`matching::judge`] decides what the interception does with a request:
//! the never-block list first, then the Filter lists, then
//! `browser.block_resources`. The cosmetic rules ([`cosmetic`]) hide
//! matched elements on the live page through [`hiding`]'s sheet and remove
//! them from a document before any extraction ([`removal`]).
//! [`consent::Runner`] is the Consent prompt runner's bundle (ADR 0048). No
//! browser crate is a dependency: the page crate calls it from its
//! interception, its live hiding, its formats and its consent runner.

pub mod consent;
pub mod cosmetic;
mod filter_set;
mod generic;
pub mod hiding;
mod index;
pub mod list;
pub mod matching;
pub mod never_block;
pub mod removal;
pub mod rules;
mod stand_ins;
