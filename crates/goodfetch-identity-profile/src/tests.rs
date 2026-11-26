//! A refresh of a drawn profile's version rewrites its version fields and
//! nothing else (story 8-36); the drawing and narrowing themselves are
//! tested through the public interface in `tests/it`.

use rstest::rstest;

use crate::{
    BrowserVersion, Constraints, EngineFamily, ExitFacts, IdentityProfile, IdentitySeed, generate,
    refresh_version_fields,
};

fn version(dotted: &str) -> BrowserVersion {
    BrowserVersion::parse(dotted).expect("a dotted version")
}

fn header<'a>(profile: &'a IdentityProfile, name: &str) -> Option<&'a str> {
    profile
        .http
        .headers
        .iter()
        .find(|(sent, _)| sent == name)
        .map(|(_, value)| value.as_str())
}

/// `profile` with its version fields emptied, so two profiles that differ
/// only by version compare equal.
fn without_version_fields(mut profile: IdentityProfile) -> IdentityProfile {
    profile.version = version("1.0");
    profile.user_agent.clear();
    profile.navigator.app_version.clear();
    if let Some(hints) = &mut profile.client_hints {
        hints.brands.clear();
        hints.full_version_list.clear();
        hints.ua_full_version.clear();
    }
    for (name, value) in &mut profile.http.headers {
        if name == "user-agent" || name == "sec-ch-ua" {
            value.clear();
        }
    }
    profile
}

fn chrome_on_windows() -> IdentityProfile {
    let constraints = Constraints::narrowed(
        EngineFamily::Chromium,
        &["windows".to_owned()],
        &[],
        &["chrome".to_owned()],
    )
    .expect("Chrome on Windows is drawn from");
    generate(IdentitySeed(7), &constraints, ExitFacts::default())
}

/// Covers: story:8-36
#[rstest]
#[case::chrome_131(
    "131.0.6778.85",
    "Chrome/131.0.0.0 ",
    r#""Google Chrome";v="131", "Chromium";v="131", "Not_A Brand";v="24""#
)]
#[case::chrome_120(
    "120.0.6099.109",
    "Chrome/120.0.0.0 ",
    r#""Not_A Brand";v="8", "Chromium";v="120", "Google Chrome";v="120""#
)]
fn a_refresh_rewrites_the_version_fields(
    #[case] dotted: &str,
    #[case] token: &str,
    #[case] sec_ch_ua: &str,
) {
    let mut refreshed = chrome_on_windows();

    refresh_version_fields(&mut refreshed, version(dotted));

    assert!(
        refreshed.user_agent.contains(token),
        "{}",
        refreshed.user_agent
    );
    assert_eq!(header(&refreshed, "sec-ch-ua"), Some(sec_ch_ua));
    assert_eq!(
        header(&refreshed, "user-agent"),
        Some(refreshed.user_agent.as_str())
    );
    let hints = refreshed
        .client_hints
        .as_ref()
        .expect("Chrome has client hints");
    assert_eq!(hints.ua_full_version, dotted);
    assert!(
        hints
            .full_version_list
            .iter()
            .any(|entry| entry.brand == "Google Chrome" && entry.version == dotted)
    );
}

/// Covers: story:8-36
#[test]
fn a_refresh_changes_nothing_but_the_version_fields() {
    let drawn = chrome_on_windows();
    let mut refreshed = drawn.clone();

    refresh_version_fields(&mut refreshed, version("131.0.6778.85"));

    assert_eq!(
        without_version_fields(refreshed),
        without_version_fields(drawn)
    );
}

/// Covers: story:8-36
#[test]
fn a_firefox_refresh_rewrites_its_release_tokens() {
    let mut refreshed = generate(
        IdentitySeed(3),
        &Constraints::narrowed(EngineFamily::Firefox, &[], &[], &[])
            .expect("Firefox's whole population is drawn from"),
        ExitFacts::default(),
    );

    refresh_version_fields(&mut refreshed, version("150.0"));

    assert!(
        refreshed.user_agent.contains("rv:150.0)"),
        "{}",
        refreshed.user_agent
    );
    assert!(refreshed.user_agent.ends_with(" Firefox/150.0"));
    assert_eq!(refreshed.version.major(), 150);
}
