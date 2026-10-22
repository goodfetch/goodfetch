//! Covers: story:3-146

use goodfetch_protocol::jsonrpc::TraceContext;
use opentelemetry::trace::{SpanId, TraceId};
use rstest::rstest;

use super::{Dropped, receive};

/// A version-00 `traceparent` W3C Trace Context's examples use.
const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

fn context(traceparent: Option<&str>, tracestate: Option<&str>) -> TraceContext {
    TraceContext {
        traceparent: traceparent.map(str::to_owned),
        tracestate: tracestate.map(str::to_owned),
    }
}

#[test]
fn a_valid_traceparent_is_the_requests_remote_parent_with_its_tracestate() {
    let received = receive(&context(Some(TRACEPARENT), Some("congo=t61rcWkgMzE")));

    let parent = received.parent.unwrap();
    assert_eq!(
        (
            parent.trace_id(),
            parent.span_id(),
            parent.is_sampled(),
            parent.is_remote(),
            parent.trace_state().header(),
        ),
        (
            TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").unwrap(),
            SpanId::from_hex("b7ad6b7169203331").unwrap(),
            true,
            true,
            "congo=t61rcWkgMzE".to_owned(),
        )
    );
    assert_eq!(received.dropped, []);
}

#[test]
fn a_request_without_a_traceparent_has_no_parent_and_drops_nothing() {
    let received = receive(&context(None, None));

    assert_eq!((received.parent, received.dropped), (None, vec![]));
}

/// A later version's `traceparent` keeps its first four fields and ignores
/// what follows, as W3C Trace Context says.
#[test]
fn a_later_versions_traceparent_is_read_by_its_first_four_fields() {
    let traceparent = "cc-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01-what-follows";

    let received = receive(&context(Some(traceparent), None));

    assert_eq!(
        received.parent.map(|parent| parent.span_id()),
        Some(SpanId::from_hex("b7ad6b7169203331").unwrap())
    );
}

#[rstest]
#[case::the_forbidden_version("ff-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")]
#[case::a_version_not_hex("0g-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")]
#[case::an_all_zero_trace_id("00-00000000000000000000000000000000-b7ad6b7169203331-01")]
#[case::an_all_zero_parent_id("00-0af7651916cd43dd8448eb211c80319c-0000000000000000-01")]
#[case::a_short_trace_id("00-0af7651916cd43dd8448eb211c8031-b7ad6b7169203331-01")]
#[case::a_long_parent_id("00-0af7651916cd43dd8448eb211c80319c-b7ad6b716920333100-01")]
#[case::short_flags("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-1")]
#[case::uppercase_hex("00-0AF7651916CD43DD8448EB211C80319C-b7ad6b7169203331-01")]
#[case::version_00_with_more("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01-more")]
#[case::a_later_version_run_on("cc-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01more")]
#[case::empty("")]
#[case::not_one("a traceparent")]
fn a_traceparent_that_does_not_parse_starts_a_new_trace_and_drops_tracestate(
    #[case] traceparent: &str,
) {
    let received = receive(&context(Some(traceparent), Some("congo=t61rcWkgMzE")));

    assert_eq!(
        (received.parent, received.dropped),
        (
            None,
            vec![
                Dropped::TraceparentUnreadable,
                Dropped::TracestateWithoutParent
            ]
        )
    );
}

#[test]
fn a_traceparent_over_256_characters_is_dropped_though_it_would_parse() {
    let traceparent = format!(
        "cc-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01-{}",
        "a".repeat(200)
    );
    assert_eq!(traceparent.len(), 256);
    let longer = format!("{traceparent}a");

    let at_the_bound = receive(&context(Some(&traceparent), None));
    let over_it = receive(&context(Some(&longer), None));

    assert!(at_the_bound.parent.is_some());
    assert_eq!(
        (over_it.parent, over_it.dropped),
        (None, vec![Dropped::TraceparentTooLong])
    );
}

/// A valid `tracestate` of `length` characters, at least 509: three list
/// members, each value within W3C's 256 characters.
fn tracestate_of(length: usize) -> String {
    let value = "x".repeat(250);
    format!("a={value},b={value},c={}", "x".repeat(length - 508))
}

#[rstest]
#[case::over_512_characters(&tracestate_of(513), Dropped::TracestateTooLong)]
#[case::invalid_on_its_own("not a tracestate", Dropped::TracestateUnreadable)]
#[case::a_key_with_uppercase("Congo=t61rcWkgMzE", Dropped::TracestateUnreadable)]
fn a_tracestate_it_cannot_use_is_dropped_and_its_traceparent_kept(
    #[case] tracestate: &str,
    #[case] dropped: Dropped,
) {
    let received = receive(&context(Some(TRACEPARENT), Some(tracestate)));

    assert_eq!(
        received
            .parent
            .map(|parent| (parent.span_id(), parent.trace_state().header())),
        Some((SpanId::from_hex("b7ad6b7169203331").unwrap(), String::new()))
    );
    assert_eq!(received.dropped, [dropped]);
}

#[test]
fn a_tracestate_of_512_characters_is_kept() {
    let tracestate = tracestate_of(512);
    assert_eq!(tracestate.len(), 512);

    let received = receive(&context(Some(TRACEPARENT), Some(&tracestate)));

    assert_eq!(
        received.parent.map(|parent| parent.trace_state().header()),
        Some(tracestate)
    );
}

#[test]
fn a_tracestate_without_a_traceparent_is_dropped() {
    let received = receive(&context(None, Some("congo=t61rcWkgMzE")));

    assert_eq!(
        (received.parent, received.dropped),
        (None, vec![Dropped::TracestateWithoutParent])
    );
}
