//! The Scripted model (ADRs 0065 and 0125; story 121): a test adapter at
//! the engine's Model provider interface that answers call *n* with reply
//! *n*, so Repair, the Agent loop, Code mode and Limits are tested in the
//! Quick suite with no model. Each [`Reply`] is text, tool calls,
//! structured output, a stream of chunks or a provider error, with the
//! token usage the call reports. A call beyond the script fails, a reply
//! left unused fails at [`ScriptedModel::assert_done`] and on drop, and
//! [`ScriptedModel::requests`] lists the requests it received in order.
//!
//! Its own crate for STYLE.md's second reason, the direction the compiler
//! enforces: it takes the Model provider interface, and no Product crate
//! takes it but under `[dev-dependencies]` (`xtask dependency-direction`),
//! so neither Product's executable links it; each Product's cargo-deny
//! check bans it too (ADR 0087). Never shipped. It is not the SDK testing
//! kits' Scripted model (ADR 0069).

mod reply;
mod scripted;

pub use reply::Reply;
pub use scripted::{ScriptExhausted, ScriptedModel};
