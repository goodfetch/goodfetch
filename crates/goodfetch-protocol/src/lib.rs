//! Goodfetch's contracts, defined once as Rust types (ADR 0038): every entry
//! of the Operation catalog and every contract generated beside it, each
//! marked with the smallest Product that carries it by where it is defined,
//! Foxhound's own in the `foxhound` module (ADRs 0095 and 0099).
//!
//! `xtask codegen` emits the committed JSON Schema bundle and catalog file
//! from them, through the catalog's [`catalog::sections`], and generates
//! every other surface from those.
#![warn(missing_docs)]

pub mod audit;
pub mod block;
pub mod browser_cache;
pub mod case_log;
pub mod catalog;
pub mod checkpoint;
pub mod clean_up_rules;
pub mod cli;
pub mod connection;
pub mod daemon;
pub mod dns;
pub mod download;
pub mod effect;
pub mod egress;
pub mod engine;
pub mod entries;
pub mod error_code;
mod error_details;
pub mod events;
pub mod executables;
pub mod formats;
pub mod foxhound;
pub mod foxhound_names;
pub mod governor;
pub mod home;
pub mod hook;
pub mod hostile_input;
pub mod journey;
pub mod jsonrpc;
pub mod key_value_store;
pub mod log;
pub mod model;
pub mod naming;
pub mod needs_you;
pub mod ops;
pub mod page;
pub mod plugin;
pub mod point_in_time;
pub mod politeness;
pub mod refusals;
pub mod releases;
pub mod response_limits;
pub mod result_document;
pub mod routing;
pub mod run;
pub mod sample;
pub mod secret_reference;
pub mod secrets;
pub mod security;
pub mod setting_scopes;
pub mod settings;
pub mod site_preferences;
pub mod storage;
pub mod tls;
pub mod usage;
pub mod versions;
pub mod wire_name;
