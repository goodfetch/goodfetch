//! The compile-fail tests: each builds the cases of one `tests/ui` folder
//! with trybuild, which writes every test's cases into the one project it
//! generates for the crate. They run one at a time, in nextest's
//! `compile-fail` test group (TESTING.md, Isolation).

mod ai_work;
mod connection_type;
mod duration;
mod error;
mod forms;
mod inputs;
mod settings_types;
