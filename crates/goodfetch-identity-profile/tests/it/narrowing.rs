//! The `stealth.identity.*` settings narrow the population a profile is
//! drawn from, and a value it does not hold is refused with the values it
//! does (story 8-38).

use goodfetch_identity_profile::{
    Brand, Constraints, Device, EngineFamily, ExitFacts, IdentityProfile, IdentitySeed,
    NarrowingError, NarrowingKey, Os, generate,
};
use rstest::rstest;

use crate::words;

const EVERY_OS: &[Os] = &[Os::Windows, Os::Macos, Os::Linux, Os::Android];

/// Covers: story:8-38
#[rstest]
#[case::mobile(EngineFamily::Chromium, &[], &["mobile"], &[], &[Os::Android], &[Device::Mobile], &[Brand::AndroidChrome, Brand::Edge])]
#[case::edge(EngineFamily::Chromium, &[], &[], &["edge"], EVERY_OS, &[Device::Desktop, Device::Mobile], &[Brand::Edge])]
#[case::macos_firefox(EngineFamily::Firefox, &["macos"], &[], &[], &[Os::Macos], &[Device::Desktop], &[Brand::Firefox])]
fn a_narrowed_identity_is_drawn_among_the_values_named(
    #[case] family: EngineFamily,
    #[case] os: &[&str],
    #[case] device: &[&str],
    #[case] brand: &[&str],
    #[case] oses: &[Os],
    #[case] devices: &[Device],
    #[case] brands: &[Brand],
) {
    let constraints = Constraints::narrowed(family, &words(os), &words(device), &words(brand))
        .expect("the narrowing keeps browsers");

    let outside: Vec<IdentityProfile> = (0..64)
        .map(|seed| generate(IdentitySeed(seed), &constraints, ExitFacts::default()))
        .filter(|profile| {
            !(oses.contains(&profile.os)
                && devices.contains(&profile.device)
                && brands.contains(&profile.brand))
        })
        .collect();

    assert_eq!(outside, Vec::new());
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
