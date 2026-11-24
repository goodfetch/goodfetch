//! The engine both Products' executables run (ADR 0087): a library each
//! executable crate composes, at start, from the contributions of the
//! domain crates it links, and runs as an Attached engine over stdio, and
//! later as a Daemon. It is its own crate because both Products'
//! executables and their published SDK crates link it, and V8 must stay out
//! of a crate built without the `executable` feature (STYLE.md, reason 2).
//! It never names a Foxhound type: where Foxhound adds behaviour, it fills
//! an interface defined here, such as a Run's driver and the tables it keeps
//! in run state.

pub mod admission;
pub mod age_notice;
mod attached;
pub mod audit;
mod audit_operations;
mod body;
pub mod browser_cache;
mod call_error;
mod certs_operations;
pub mod checkpoint;
pub mod children;
pub mod client;
pub mod clock;
mod composition;
mod config;
pub mod controls;
pub mod crash_report;
pub mod daemon;
pub mod deadline;
pub mod dns;
mod durable;
pub mod effect;
pub mod egress;
pub mod endpoint;
mod engine;
mod engine_context;
pub mod entropy;
pub mod events;
pub mod executables;
mod forward;
pub mod governor;
pub mod happy_eyeballs;
pub mod heap;
pub mod home;
pub mod host_name;
pub mod hostile_input;
pub mod http_client;
mod info;
pub mod installed;
pub mod key_value_store;
pub mod known_certificates;
pub mod lease;
pub mod ledger;
pub mod local_addresses;
pub mod log;
pub mod lowest_minor;
pub mod mask;
pub mod metrics;
pub mod needs_you;
mod needs_you_operations;
pub mod ops;
pub mod otlp;
pub mod pinned_download;
pub mod politeness;
mod product;
pub mod request_signing;
pub mod response_limits;
pub mod retry_after;
pub mod robots_txt;
pub mod rpc;
pub mod rule_sources;
pub mod run;
pub mod run_files;
pub mod run_files_operations;
mod run_index;
pub mod run_key;
pub mod run_state;
pub mod run_trace;
pub mod runs;
pub mod sandbox_probe;
mod sdk_handlers;
pub mod secret_endpoint;
pub mod secrets;
pub mod session_bundle;
pub mod settings;
mod settings_editor;
mod signing_key_operations;
mod sqlite_store;
mod sqlite_wal;
pub mod staging;
mod stored_time;
mod sync;
pub mod tasks;
pub mod tls;
pub mod trace;
pub mod transition;
pub mod transport;
pub mod upgrade;
mod usage;
mod wire;

pub use call_error::{CallError, named};
pub use composition::{
    Answered, CallContext, CallInput, CallStream, Composition, CompositionError, Contribution,
    Driver, Enclose, Guard, GuardAnswer, Handler, Params, ReadParams, RunOfCall,
};
pub use engine::{AttachedFailure, Dependencies, Engine};
pub use engine_context::EngineContext;
pub use info::contribution;
pub use product::{Channel, Identity, Requested};
pub use stored_time::StoredTime;
