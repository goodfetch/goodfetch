//! The Model Context Protocol server shared by Collie and Foxhound (spec
//! #608; ADRs 0011, 0070, 0087 and 0095): projects catalog operations onto
//! MCP tools, composes profiles identically in both Products, bridges tool
//! calls to a Goodfetch engine Connection with cancellation, progress and
//! W3C Trace Context propagation, and serves over stdio and Streamable HTTP.
//!
//! No module in this crate names a Foxhound type (ADR 0087); Foxhound's
//! executable composes this crate with its own Product and profiles.

pub mod code_mode_tool;
pub mod engine;
pub mod oauth;
pub mod observations;
pub mod profiles;
pub mod projection;
pub mod resources;
pub mod revisions;
pub mod server;
pub mod transports;

#[cfg(test)]
mod tests;
