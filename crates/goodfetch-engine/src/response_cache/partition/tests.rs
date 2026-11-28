//! The partition key as the pure function it is: which partition keeps a
//! response and which a lookup reads, by example, and, for any Session's
//! keeper, request and response, that a response is found again by its
//! own Session and never by another's.

use bytes::Bytes;
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::run::RunId;
use proptest::prelude::*;
use rstest::rstest;

use super::{Keeper, kept_in, read_from, shares};
use crate::response_cache::{CacheRequest, Partition, ResponseHead};

fn pairs(headers: &[(&str, &str)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn get(headers: &[(&str, &str)]) -> CacheRequest {
    CacheRequest::new(
        "GET",
        &"https://cache.test/page".parse().unwrap(),
        &pairs(headers),
        Bytes::new(),
    )
}

fn head(headers: &[(&str, &str)]) -> ResponseHead {
    ResponseHead {
        status: 200,
        headers: pairs(headers),
    }
}

fn identity(name: &str) -> Keeper {
    Keeper::SavedIdentity(name.to_owned())
}

fn session(run: &str, session: &str) -> Keeper {
    Keeper::Session {
        run_id: RunId::parse(run).unwrap(),
        session_id: SessionId(session.to_owned()),
    }
}

/// A response that carries nothing of a person's is the public
/// partition's whoever fetched it; any other is its Session's Saved
/// identity's or the Session's own, and kept nowhere with neither, or
/// under a Saved identity named as another partition could be.
/// Covers: story:6-39
#[rstest]
#[case::public_with_no_keeper(None, get(&[]), head(&[("cache-control", "max-age=60")]), Some("public"))]
#[case::public_under_an_identity(Some(identity("ada-login")),get(&[("accept", "text/html")]), head(&[]), Some("public"))]
#[case::cookie_under_an_identity(Some(identity("ada-login")),get(&[("cookie", "a=1")]), head(&[]), Some("ada-login"))]
#[case::set_cookie_in_a_session(Some(session("r1", "s1")),get(&[]), head(&[("set-cookie", "a=1")]), Some("session:r1:s1"))]
#[case::private_in_a_session(Some(session("r1", "s2")),get(&[]), head(&[("cache-control", "private")]), Some("session:r1:s2"))]
#[case::authorization_with_no_keeper(None, get(&[("authorization", "Basic YTpi")]), head(&[]), None)]
#[case::set_cookie_with_no_keeper(None, get(&[]), head(&[("set-cookie", "a=1")]), None)]
#[case::identity_named_public(Some(identity("public")),get(&[("cookie", "a=1")]), head(&[]), None)]
#[case::identity_named_as_a_session(Some(identity("session:r1:s1")),get(&[("proxy-authorization", "Basic YTpi")]), head(&[]), None)]
fn the_partition_a_response_is_kept_in(
    #[case] keeper: Option<Keeper>,
    #[case] request: CacheRequest,
    #[case] response: ResponseHead,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        kept_in(keeper.as_ref(), &request, &response).as_deref(),
        expected
    );
}

/// A request with no cookie or credential reads the public partition and
/// its Session's; one with, its Session's alone, and nothing with no
/// partition of its own.
/// Covers: story:6-39
#[rstest]
#[case::plain_with_no_keeper(None, get(&[]), Some(["public", "public"]))]
#[case::plain_in_a_session(Some(session("r1", "s1")),get(&[]), Some(["public", "session:r1:s1"]))]
#[case::cookie_under_an_identity(Some(identity("ada-login")),get(&[("cookie", "a=1")]), Some(["ada-login", "ada-login"]))]
#[case::cookie_with_no_keeper(None, get(&[("cookie", "a=1")]), None)]
#[case::cookie_under_an_identity_named_public(Some(identity("public")),get(&[("cookie", "a=1")]), None)]
fn the_partitions_a_lookup_reads(
    #[case] keeper: Option<Keeper>,
    #[case] request: CacheRequest,
    #[case] expected: Option<[&str; 2]>,
) {
    assert_eq!(
        read_from(keeper.as_ref(), &request)
            .as_ref()
            .map(|[first, second]| [first.as_str(), second.as_str()]),
        expected
    );
}

/// Any Session's keeper: none, a Saved identity, names that could be read
/// as another partition's among them, or a Session of a Run.
fn keepers() -> impl Strategy<Value = Option<Keeper>> {
    prop_oneof![
        Just(None),
        prop_oneof![
            "[a-z:-]{0,10}",
            Just("public".to_owned()),
            Just("session:r1:s1".to_owned()),
        ]
        .prop_map(|name| Some(Keeper::SavedIdentity(name))),
        ("[a-z0-9]{1,3}", "[a-z0-9:]{0,4}").prop_map(|(run, session)| Some(Keeper::Session {
            run_id: RunId::parse(&run).unwrap(),
            session_id: SessionId(session),
        })),
    ]
}

/// Any request, with or without a cookie or credential.
fn requests() -> impl Strategy<Value = CacheRequest> {
    prop::sample::subsequence(
        vec![
            ("accept", "text/html"),
            ("cookie", "a=1"),
            ("authorization", "Basic YTpi"),
            ("proxy-authorization", "Basic YTpi"),
        ],
        0..=4,
    )
    .prop_map(|headers| get(&headers))
}

/// Any response, setting a cookie or `private` or neither.
fn responses() -> impl Strategy<Value = ResponseHead> {
    prop::sample::subsequence(
        vec![
            ("cache-control", "max-age=60"),
            ("set-cookie", "a=1"),
            ("cache-control", "private"),
        ],
        0..=3,
    )
    .prop_map(|headers| head(&headers))
}

/// Any full Partition across keeper, person, exit country, language and device class.
fn partitions() -> impl Strategy<Value = Partition> {
    (
        keepers(),
        prop_oneof![Just("ada"), Just("bob"), Just("os_user")],
        prop_oneof![Just(""), Just("US"), Just("DE"), Just("FR")],
        prop_oneof![Just(""), Just("en-US"), Just("fr-FR"), Just("de-DE")],
        prop_oneof![Just(""), Just("desktop"), Just("mobile")],
    )
        .prop_map(|(keeper, person, country, language, device)| Partition {
            keeper,
            person: person.to_owned(),
            country: country.to_owned(),
            language: language.to_owned(),
            device: device.to_owned(),
        })
}

proptest! {
    /// Covers: story:6-39
    #[test]
    fn a_kept_response_is_read_again_by_its_own_session(
        keeper in keepers(),
        request in requests(),
        response in responses(),
    ) {
        if let Some(partition) = kept_in(keeper.as_ref(), &request, &response) {
            let read = read_from(keeper.as_ref(), &request);
            prop_assert!(read.is_some_and(|read| read.contains(&partition)));
        }
    }

    /// Covers: story:6-39
    #[test]
    fn a_personal_response_is_never_read_by_another_keeper(
        (keeper, other) in (keepers(), keepers()).prop_filter("two keepers", |(one, two)| one != two),
        stored in requests(),
        response in responses(),
        asked in requests(),
    ) {
        let partition = kept_in(keeper.as_ref(), &stored, &response);
        let read = read_from(other.as_ref(), &asked);
        prop_assert!(
            partition.as_deref().is_none_or(|partition| partition == "public"
                || read.as_ref().is_none_or(|read| !read.iter().any(|name| name == partition))),
            "{partition:?} read by {other:?} as {read:?}"
        );
    }

    /// Two partitions differing in person, proxy exit country, language or
    /// device class never share any entry, even a public one.
    /// Covers: story:6-39
    #[test]
    fn partitions_differing_in_person_country_language_or_device_never_share(
        (stored, asked) in (partitions(), partitions()).prop_filter(
            "different person, country, language or device",
            |(one, two)| {
                one.person != two.person
                    || one.country != two.country
                    || one.language != two.language
                    || one.device != two.device
            },
        ),
        stored_request in requests(),
        response in responses(),
        asked_request in requests(),
    ) {
        prop_assert!(!shares(&stored, &stored_request, &response, &asked, &asked_request));
    }
}
