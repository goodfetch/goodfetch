use std::net::IpAddr;
use std::panic::{self, AssertUnwindSafe};

use super::{Lookup, LookupFailure, ScriptedResolver};

fn address(text: &str) -> IpAddr {
    text.parse().unwrap()
}

/// The typed failure `call` panicked with.
fn failure_of(call: impl FnOnce()) -> LookupFailure {
    let payload = panic::catch_unwind(AssertUnwindSafe(call)).unwrap_err();
    payload
        .downcast::<LookupFailure>()
        .map(|failure| *failure)
        .unwrap()
}

/// Drops a resolver whose script a test broke on purpose, ignoring the
/// failure its drop repeats.
fn discard(resolver: ScriptedResolver) {
    drop(panic::catch_unwind(AssertUnwindSafe(|| drop(resolver))));
}

#[test]
fn a_scripted_resolver_answers_its_lookups_in_order() {
    let resolver = ScriptedResolver::new([
        Lookup::new("rebind.ssrf.test", [address("192.31.196.1")]),
        Lookup::new("rebind.ssrf.test", [address("127.0.0.1"), address("::1")]),
    ]);
    let handle = resolver.handle();

    let answers = [
        handle.lookup("rebind.ssrf.test"),
        handle.lookup("rebind.ssrf.test"),
    ];

    assert_eq!(
        answers,
        [
            vec![address("192.31.196.1")],
            vec![address("127.0.0.1"), address("::1")]
        ]
    );
}

#[test]
fn a_lookup_beyond_the_script_fails_the_test() {
    let resolver =
        ScriptedResolver::new([Lookup::new("public.ssrf.test", [address("192.31.196.1")])]);
    let handle = resolver.handle();
    handle.lookup("public.ssrf.test");

    let failure = failure_of(|| {
        handle.lookup("public.ssrf.test");
    });

    assert_eq!(
        failure,
        LookupFailure::BeyondScript {
            name: "public.ssrf.test".to_owned(),
            scripted: 1,
        }
    );
    discard(resolver);
}

#[test]
fn a_lookup_of_another_name_than_the_scripts_next_fails_the_test() {
    let resolver =
        ScriptedResolver::new([Lookup::new("public.ssrf.test", [address("192.31.196.1")])]);
    let handle = resolver.handle();

    let failure = failure_of(|| {
        handle.lookup("internal.ssrf.test");
    });

    assert_eq!(
        failure,
        LookupFailure::OtherName {
            expected: "public.ssrf.test".to_owned(),
            name: "internal.ssrf.test".to_owned(),
        }
    );
    discard(resolver);
}

#[test]
fn a_failed_lookup_the_code_under_test_swallowed_fails_the_test_when_the_resolver_drops() {
    let resolver = ScriptedResolver::new([]);
    let handle = resolver.handle();
    drop(panic::catch_unwind(AssertUnwindSafe(|| {
        handle.lookup("public.ssrf.test")
    })));

    let failure = failure_of(|| drop(resolver));

    assert_eq!(
        failure,
        LookupFailure::BeyondScript {
            name: "public.ssrf.test".to_owned(),
            scripted: 0,
        }
    );
}

#[test]
fn a_resolver_whose_lookups_kept_to_its_script_drops_quietly() {
    let resolver =
        ScriptedResolver::new([Lookup::new("public.ssrf.test", [address("192.31.196.1")])]);
    resolver.handle().lookup("public.ssrf.test");

    let dropped = panic::catch_unwind(AssertUnwindSafe(|| drop(resolver)));

    assert!(dropped.is_ok());
}
