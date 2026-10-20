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

/// Asserts the library's own Error code and event through its interface,
/// on a loopback stand-in, never reaching `rpc`.
/// Covers: error:sample_refused, event:sample.echoed
#[test]
fn refuses_a_text_the_stand_in_rejects() {}

/// Covers: command:browsers.install, exit:not_installed
#[test]
fn installs_with_its_exit_code() {}

#[test]
fn declares_nothing() {}
