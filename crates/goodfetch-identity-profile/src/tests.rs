use std::collections::BTreeSet;

use proptest::prelude::*;
use rstest::rstest;

use crate::candidates::header_order;
use crate::population::kind;
use crate::{
    Brand, BrowserVersion, Constraints, Device, EngineFamily, ExitFacts, IdentityProfile,
    IdentitySeed, NarrowingError, NarrowingKey, Os, generate, refresh_version_fields,
};

fn words(values: &[&str]) -> Vec<String> {
    values.iter().map(|&value| value.to_owned()).collect()
}

fn whole(family: EngineFamily) -> Constraints {
    Constraints::narrowed(family, &[], &[], &[]).expect("a family's whole population is drawn from")
}

fn french_exit() -> ExitFacts {
    ExitFacts {
        country: Some("FR".to_owned()),
        language: Some("fr-FR".to_owned()),
        time_zone: Some("Europe/Paris".to_owned()),
        ..ExitFacts::default()
    }
}

fn family() -> impl Strategy<Value = EngineFamily> {
    prop_oneof![Just(EngineFamily::Chromium), Just(EngineFamily::Firefox)]
}

fn header<'a>(profile: &'a IdentityProfile, name: &str) -> Option<&'a str> {
    profile
        .http
        .headers
        .iter()
        .find(|(sent, _)| sent == name)
        .map(|(_, value)| value.as_str())
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:8-36
    #[test]
    fn every_drawn_profile_presents_one_consistent_browser(seed: u64, family in family()) {
        let profile = generate(IdentitySeed(seed), &whole(family), french_exit());

        let said = kind(&profile.user_agent).expect("the user agent is one of the population's");
        prop_assert_eq!(
            (said.os, said.device, said.brand, said.major),
            (profile.os, profile.device, profile.brand, profile.version.major())
        );
        prop_assert_eq!(profile.family, family);
        prop_assert_eq!(profile.brand.family(), family);
        match (&profile.client_hints, family) {
            (Some(hints), EngineFamily::Chromium) => {
                let platform = match profile.os {
                    Os::Windows => "Windows",
                    Os::Macos => "macOS",
                    Os::Linux => "Linux",
                    Os::Android => "Android",
                };
                prop_assert_eq!(hints.platform.as_str(), platform);
                prop_assert_eq!(hints.mobile, profile.device == Device::Mobile);
                prop_assert_eq!(
                    hints.brands.iter().any(|entry| entry.brand == "Microsoft Edge"),
                    profile.brand == Brand::Edge
                );
                prop_assert_eq!(hints.ua_full_version.as_str(), profile.version.full());
                prop_assert_eq!(profile.navigator.vendor.as_str(), "Google Inc.");
                prop_assert!(profile.navigator.device_memory.is_some());
                prop_assert!(profile.navigator.oscpu.is_none());
            }
            (None, EngineFamily::Firefox) => {
                prop_assert_eq!(profile.navigator.vendor.as_str(), "");
                prop_assert!(profile.navigator.device_memory.is_none());
                prop_assert!(profile.navigator.oscpu.is_some());
            }
            (hints, family) => prop_assert!(false, "{family} drew client hints {hints:?}"),
        }
        let platforms: &[&str] = match profile.os {
            Os::Windows => &["Win32"],
            Os::Macos => &["MacIntel"],
            Os::Linux | Os::Android => &["Linux x86_64", "Linux aarch64", "Linux armv81", "Linux armv8l"],
        };
        prop_assert!(platforms.contains(&profile.navigator.platform.as_str()));
        prop_assert!(profile.navigator.hardware_concurrency > 0);
        match (profile.device, profile.os) {
            (Device::Mobile, _) => prop_assert!(profile.navigator.max_touch_points > 0),
            (Device::Desktop, Os::Macos) => prop_assert_eq!(profile.navigator.max_touch_points, 0),
            (Device::Desktop, _) => {}
        }
        prop_assert_eq!(
            profile.video_card.renderer.contains("Direct3D"),
            profile.os == Os::Windows
        );
        prop_assert!(profile.screen.avail_width <= profile.screen.width);
        prop_assert!(profile.screen.avail_height <= profile.screen.height);
        prop_assert_eq!(&profile.navigator.languages, &words(&["fr-FR", "fr"]));
        prop_assert_eq!(header(&profile, "user-agent"), Some(profile.user_agent.as_str()));
        prop_assert!(header(&profile, "accept").is_some());
        prop_assert!(header(&profile, "accept-encoding").is_some());
        prop_assert_eq!(
            header(&profile, "sec-ch-ua").is_some(),
            family == EngineFamily::Chromium
        );
        let order = header_order(profile.brand);
        let positions: Vec<usize> = profile
            .http
            .headers
            .iter()
            .map(|(name, _)| order.iter().position(|known| known == name).expect("a sent header has its place"))
            .collect();
        prop_assert!(positions.is_sorted());
        prop_assert_eq!(&profile.exit, &french_exit());
    }

    /// Covers: story:8-36
    #[test]
    fn the_same_seed_draws_the_same_profile(seed: u64, family in family()) {
        let constraints = whole(family);

        prop_assert_eq!(
            generate(IdentitySeed(seed), &constraints, french_exit()),
            generate(IdentitySeed(seed), &constraints, french_exit())
        );
    }
}

/// Covers: story:8-36
#[test]
fn different_seeds_draw_different_profiles() {
    let constraints = whole(EngineFamily::Chromium);

    let drawn: Vec<IdentityProfile> = (0..100)
        .map(|seed| generate(IdentitySeed(seed), &constraints, ExitFacts::default()))
        .collect();

    for (at, profile) in drawn.iter().enumerate() {
        assert!(
            !drawn[at + 1..].contains(profile),
            "seed {at} draws a profile a later seed draws too"
        );
    }
}

/// Covers: story:8-38
#[rstest]
#[case::mobile(EngineFamily::Chromium, &[], &["mobile"], &[], &[Device::Mobile], &[Brand::AndroidChrome, Brand::Edge])]
#[case::edge(EngineFamily::Chromium, &[], &[], &["edge"], &[Device::Desktop, Device::Mobile], &[Brand::Edge])]
#[case::macos_firefox(EngineFamily::Firefox, &["macos"], &[], &[], &[Device::Desktop], &[Brand::Firefox])]
fn a_narrowed_identity_is_drawn_among_the_values_named(
    #[case] family: EngineFamily,
    #[case] os: &[&str],
    #[case] device: &[&str],
    #[case] brand: &[&str],
    #[case] devices: &[Device],
    #[case] brands: &[Brand],
) {
    let constraints = Constraints::narrowed(family, &words(os), &words(device), &words(brand))
        .expect("the narrowing keeps browsers");

    let drawn: Vec<IdentityProfile> = (0..64)
        .map(|seed| generate(IdentitySeed(seed), &constraints, ExitFacts::default()))
        .collect();

    assert!(drawn.iter().all(|profile| devices.contains(&profile.device)
        && brands.contains(&profile.brand)
        && os.iter().all(|&os| profile.os.as_str() == os)));
}

/// Covers: story:8-38
#[rstest]
#[case::ios(EngineFamily::Chromium, &["ios"], &[], &[], NarrowingKey::Os, "ios")]
#[case::edge_on_firefox(EngineFamily::Firefox, &[], &[], &["edge"], NarrowingKey::Brand, "edge")]
#[case::tablet(EngineFamily::Chromium, &[], &["tablet"], &[], NarrowingKey::Device, "tablet")]
fn a_value_outside_the_population_is_refused_with_the_population(
    #[case] family: EngineFamily,
    #[case] os: &[&str],
    #[case] device: &[&str],
    #[case] brand: &[&str],
    #[case] key: NarrowingKey,
    #[case] value: &str,
) {
    let refused = Constraints::narrowed(family, &words(os), &words(device), &words(brand));

    assert!(matches!(
        refused,
        Err(NarrowingError::OutsidePopulation { key: refused_key, value: refused_value, ref population, .. })
            if refused_key == key && refused_value == value && !population.is_empty()
    ));
}

/// Covers: story:8-38
#[test]
fn a_combination_no_browser_has_is_refused_at_the_key_that_empties_it() {
    let refused = Constraints::narrowed(
        EngineFamily::Chromium,
        &words(&["macos"]),
        &words(&["mobile"]),
        &[],
    );

    assert_eq!(
        refused,
        Err(NarrowingError::Empty {
            family: EngineFamily::Chromium,
            key: NarrowingKey::Device,
        })
    );
}

fn version(dotted: &str) -> BrowserVersion {
    BrowserVersion::parse(dotted).expect("a dotted version")
}

/// Covers: story:8-36
#[rstest]
#[case::chrome_131(
    "131.0.6778.85",
    r#""Google Chrome";v="131", "Chromium";v="131", "Not_A Brand";v="24""#
)]
#[case::chrome_120(
    "120.0.6099.109",
    r#""Not_A Brand";v="8", "Chromium";v="120", "Google Chrome";v="120""#
)]
fn a_refresh_changes_only_the_version_fields(#[case] dotted: &str, #[case] sec_ch_ua: &str) {
    let constraints = Constraints::narrowed(
        EngineFamily::Chromium,
        &words(&["windows"]),
        &[],
        &words(&["chrome"]),
    )
    .expect("Chrome on Windows is drawn from");
    let drawn = generate(IdentitySeed(7), &constraints, french_exit());
    let mut refreshed = drawn.clone();

    refresh_version_fields(&mut refreshed, version(dotted));

    let major = dotted.split('.').next().expect("a major");
    assert!(
        refreshed
            .user_agent
            .contains(&format!("Chrome/{major}.0.0.0 "))
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
    let unchanged = |profile: &IdentityProfile| {
        (
            profile.os,
            profile.device,
            profile.brand,
            profile.screen,
            profile.video_card.clone(),
            profile.navigator.hardware_concurrency,
            profile.navigator.platform.clone(),
            profile
                .http
                .headers
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>(),
            profile.exit.clone(),
        )
    };
    assert_eq!(unchanged(&refreshed), unchanged(&drawn));

    refresh_version_fields(&mut refreshed, drawn.version.clone());

    assert_eq!(refreshed, drawn);
}

/// Covers: story:8-36
#[test]
fn a_firefox_refresh_rewrites_its_release_tokens() {
    let drawn = generate(
        IdentitySeed(3),
        &whole(EngineFamily::Firefox),
        ExitFacts::default(),
    );
    let mut refreshed = drawn.clone();

    refresh_version_fields(&mut refreshed, version("150.0"));

    assert!(refreshed.user_agent.contains("rv:150.0)"));
    assert!(refreshed.user_agent.ends_with(" Firefox/150.0"));
    assert_eq!(refreshed.version.major(), 150);
    refresh_version_fields(&mut refreshed, drawn.version.clone());
    assert_eq!(refreshed, drawn);
}

#[test]
fn every_kind_of_the_population_is_drawn_from() {
    let drawn: BTreeSet<(Os, Device, Brand)> = [EngineFamily::Chromium, EngineFamily::Firefox]
        .into_iter()
        .flat_map(|family| {
            let constraints = whole(family);
            (0..400).map(move |seed| {
                let profile = generate(IdentitySeed(seed), &constraints, ExitFacts::default());
                (profile.os, profile.device, profile.brand)
            })
        })
        .collect();

    for os in Os::ALL.into_iter().filter(|&os| os != Os::Android) {
        assert!(
            drawn.contains(&(os, Device::Desktop, Brand::Chrome)),
            "Chrome on {os:?}"
        );
        assert!(
            drawn.contains(&(os, Device::Desktop, Brand::Firefox)),
            "Firefox on {os:?}"
        );
    }
    assert!(drawn.contains(&(Os::Android, Device::Mobile, Brand::AndroidChrome)));
}
