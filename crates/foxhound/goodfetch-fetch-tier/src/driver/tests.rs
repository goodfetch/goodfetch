//! The proxy-or-site rule over the HTTP tiers' answers (spec #579's
//! Testing Decisions: property-tested in process): an answer is the
//! proxy's exactly when the proxy refused, wanted credentials for or never
//! opened the tunnel, or forwarded the request itself and named an error of
//! its own in the last member of `Proxy-Status`; and only the site's
//! answers back a domain off, only the proxy's lower its ceiling.
//!
//! Covers: story:5-133

use goodfetch_frontier::backoff_classes::{Class, class, party_of};
use goodfetch_page::Party;
use proptest::prelude::*;

use super::{TransportFailure, met};

mod kept;
mod untyped;

/// Every failure an HTTP tier's transport meets that names no value of
/// its own, with whether it is the proxy's.
fn failures() -> Vec<(TransportFailure, bool)> {
    vec![
        (TransportFailure::NameNotResolved, false),
        (TransportFailure::ConnectionRefused, false),
        (TransportFailure::ConnectionTimedOut, false),
        (TransportFailure::ConnectionReset, false),
        (TransportFailure::ConnectionClosed, false),
        (TransportFailure::ClosedAtReuse, false),
        (TransportFailure::RefusedStream, false),
        (TransportFailure::Http11Required, false),
        (TransportFailure::EnhanceYourCalm, false),
        (TransportFailure::Tls, false),
        (TransportFailure::InvalidResponse, false),
        (TransportFailure::ContentDecodingFailed, false),
        (TransportFailure::Other, false),
        (TransportFailure::TunnelRefused, true),
        (TransportFailure::ProxyAuthenticationRequired, true),
        (TransportFailure::ProxyUnreachable, true),
    ]
}

/// One member of a `Proxy-Status` field: an intermediary's name, then
/// parameters, an `error` among them when `error` names one.
fn member() -> impl Strategy<Value = (String, bool)> {
    (
        "[a-z][a-z0-9.-]{0,12}",
        prop::option::of("[a-z_]{1,24}"),
        prop::option::of(("[a-z]{1,8}", "[a-z0-9]{1,8}")),
    )
        .prop_map(|(name, error, other)| {
            let mut member = name;
            if let Some((key, value)) = other.filter(|(key, _)| key != "error") {
                member = format!("{member}; {key}={value}");
            }
            if let Some(error) = &error {
                member = format!("{member}; error={error}");
            }
            (member, error.is_some())
        })
}

/// A `Proxy-Status` field of one to three members, with whether its last
/// member, the nearest intermediary's, names an error.
fn proxy_status() -> impl Strategy<Value = (String, bool)> {
    prop::collection::vec(member(), 1..=3).prop_map(|members| {
        let last_names_error = members.last().is_some_and(|(_, error)| *error);
        let field = members
            .into_iter()
            .map(|(member, _)| member)
            .collect::<Vec<_>>()
            .join(", ");
        (field, last_names_error)
    })
}

proptest! {
    #[test]
    fn a_response_is_the_proxy_s_only_when_it_forwarded_it_and_named_its_own_error(
        status in 100_u16..600,
        field in prop::option::of(proxy_status()),
        forwarded in any::<bool>(),
        other_headers in prop::collection::vec(("[a-z-]{1,12}", "[ -~]{0,16}"), 0..4),
    ) {
        let mut headers: Vec<(String, String)> = other_headers
            .into_iter()
            .filter(|(name, _)| name != "proxy-status")
            .collect();
        if let Some((value, _)) = &field {
            headers.push(("proxy-status".to_owned(), value.clone()));
        }
        let by_proxy = forwarded && field.as_ref().is_some_and(|(_, error)| *error);

        let answered = met(status, &headers, forwarded);
        let (party, class) = (party_of(answered), class(answered));

        prop_assert_eq!(party, if by_proxy { Party::Proxy } else { Party::Site });
        prop_assert_eq!(
            class,
            match (by_proxy, status) {
                (true, 429 | 502) => Class::ProxyRefusal,
                (false, 429) => Class::TooManyRequests,
                (false, 502..=504) => Class::BackOff,
                (true | false, _) => Class::Done,
            }
        );
    }

    #[test]
    fn a_failure_is_the_proxy_s_only_when_the_proxy_answered_the_tunnel(
        index in 0..failures().len(),
    ) {
        let (failure, by_proxy) = failures().swap_remove(index);

        let answered = failure.met();
        let (party, class) = (party_of(answered), class(answered));

        prop_assert_eq!(party, if by_proxy { Party::Proxy } else { Party::Site });
        let backs_off = matches!(
            failure,
            TransportFailure::ConnectionRefused
                | TransportFailure::ConnectionTimedOut
                | TransportFailure::EnhanceYourCalm
        );
        prop_assert_eq!(
            class,
            match failure {
                TransportFailure::TunnelRefused => Class::ProxyRefusal,
                _ if backs_off => Class::BackOff,
                _ => Class::Done,
            }
        );
    }
}
