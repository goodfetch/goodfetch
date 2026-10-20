//! The crate's integration tests, through the engine's executable.

/// Covers: operation:sample.echo, operation:sample.crawl
#[test]
fn calls_echo_then_crawl() {}

/// Covers: error:sample_refused
#[test]
fn gets_the_refusal() {}

/// Covers: event:sample.echoed
#[test]
fn receives_the_echoed_event() {}

/// Covers: command:browsers install, exit:not_installed
#[test]
fn installs_with_its_exit_code() {}

#[test]
fn declares_nothing() {}
