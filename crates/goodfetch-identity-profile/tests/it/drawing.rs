//! A drawn Identity profile presents one real browser, the same seed draws
//! it again, and the whole population is drawn from (story 8-36).

use goodfetch_identity_profile::{
    Brand, Device, EngineFamily, ExitFacts, IdentityProfile, IdentitySeed, Os, generate,
};
use proptest::prelude::*;
use rstest::rstest;

use crate::{french_exit, header, whole, words};

/// What a browser of one operating system shows.
struct OsSigns {
    os: Os,
    /// A token its user agents carry.
    token: &'static str,
    /// Its client hints' platform.
    hint_platform: &'static str,
    /// The `navigator.platform` values its browsers show.
    navigator_platforms: &'static [&'static str],
}

const LINUX_PLATFORMS: &[&str] = &[
    "Linux x86_64",
    "Linux aarch64",
    "Linux armv81",
    "Linux armv8l",
];

const OPERATING_SYSTEMS: [OsSigns; 4] = [
    OsSigns {
        os: Os::Windows,
        token: "(Windows NT ",
        hint_platform: "Windows",
        navigator_platforms: &["Win32"],
    },
    OsSigns {
        os: Os::Macos,
        token: "(Macintosh; ",
        hint_platform: "macOS",
        navigator_platforms: &["MacIntel"],
    },
    OsSigns {
        os: Os::Linux,
        token: "(X11; ",
        hint_platform: "Linux",
        navigator_platforms: LINUX_PLATFORMS,
    },
    OsSigns {
        os: Os::Android,
        token: "Android ",
        hint_platform: "Android",
        navigator_platforms: LINUX_PLATFORMS,
    },
];

/// What a browser of one engine family shows.
struct FamilySigns {
    family: EngineFamily,
    vendor: &'static str,
    /// Whether it shows `navigator.deviceMemory`.
    device_memory: bool,
    /// Whether it shows `navigator.oscpu`.
    oscpu: bool,
    /// Whether it sends client hints.
    client_hints: bool,
    /// Its `accept-language` for the French exit.
    accept_language: &'static str,
}

const FAMILIES: [FamilySigns; 2] = [
    FamilySigns {
        family: EngineFamily::Chromium,
        vendor: "Google Inc.",
        device_memory: true,
        oscpu: false,
        client_hints: true,
        accept_language: "fr-FR,fr;q=0.9",
    },
    FamilySigns {
        family: EngineFamily::Firefox,
        vendor: "",
        device_memory: false,
        oscpu: true,
        client_hints: false,
        accept_language: "fr-FR,fr;q=0.5",
    },
];

/// The headers a Chromium profile sends, in Chrome's and Edge's order.
const CHROMIUM_HEADERS: &[&str] = &[
    "sec-ch-ua",
    "sec-ch-ua-mobile",
    "sec-ch-ua-platform",
    "upgrade-insecure-requests",
    "user-agent",
    "accept",
    "accept-encoding",
    "accept-language",
];

/// What a browser of one brand shows.
struct BrandSigns {
    brand: Brand,
    /// The token its user agents carry, around their major version.
    token: &'static str,
    /// The headers it sends, in its order.
    headers: &'static [&'static str],
}

const BRANDS: [BrandSigns; 4] = [
    BrandSigns {
        brand: Brand::Chrome,
        token: "Chrome/{major}.0.0.0 Safari/537.36",
        headers: CHROMIUM_HEADERS,
    },
    BrandSigns {
        brand: Brand::Edge,
        token: "Chrome/{major}.0.0.0 ",
        headers: CHROMIUM_HEADERS,
    },
    BrandSigns {
        brand: Brand::AndroidChrome,
        token: "Chrome/{major}.0.0.0 Mobile Safari/537.36",
        headers: CHROMIUM_HEADERS,
    },
    BrandSigns {
        brand: Brand::Firefox,
        token: "Firefox/{major}.0",
        headers: &[
            "user-agent",
            "accept",
            "accept-language",
            "accept-encoding",
            "upgrade-insecure-requests",
        ],
    },
];

fn family() -> impl Strategy<Value = EngineFamily> {
    prop_oneof![Just(EngineFamily::Chromium), Just(EngineFamily::Firefox)]
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:8-36
    #[test]
    fn every_drawn_profile_presents_one_consistent_browser(seed: u64, family in family()) {
        let profile = generate(IdentitySeed(seed), &whole(family), french_exit());

        let os = OPERATING_SYSTEMS.iter().find(|signs| signs.os == profile.os).expect("a row per os");
        let brand = BRANDS.iter().find(|signs| signs.brand == profile.brand).expect("a row per brand");
        let engine = FAMILIES.iter().find(|signs| signs.family == family).expect("a row per family");
        prop_assert_eq!(profile.family, family);
        prop_assert_eq!(profile.brand.family(), family);
        prop_assert!(profile.user_agent.contains(os.token), "{} on {:?}", profile.user_agent, profile.os);
        let brand_token = brand.token.replace("{major}", &profile.version.major().to_string());
        prop_assert!(profile.user_agent.contains(&brand_token), "{} as {:?}", profile.user_agent, profile.brand);
        prop_assert_eq!(profile.user_agent.contains(" Edg"), profile.brand == Brand::Edge);
        prop_assert_eq!(profile.user_agent.contains("Mobile"), profile.device == Device::Mobile);
        prop_assert_eq!(
            (
                profile.navigator.vendor.as_str(),
                profile.navigator.device_memory.is_some(),
                profile.navigator.oscpu.is_some(),
                profile.client_hints.is_some(),
            ),
            (engine.vendor, engine.device_memory, engine.oscpu, engine.client_hints)
        );
        if let Some(hints) = &profile.client_hints {
            prop_assert_eq!(hints.platform.as_str(), os.hint_platform);
            prop_assert_eq!(hints.mobile, profile.device == Device::Mobile);
            prop_assert_eq!(
                hints.brands.iter().any(|entry| entry.brand == "Microsoft Edge"),
                profile.brand == Brand::Edge
            );
            prop_assert_eq!(hints.ua_full_version.as_str(), profile.version.full());
            let quoted_platform = format!("\"{}\"", os.hint_platform);
            prop_assert_eq!(header(&profile, "sec-ch-ua-platform"), Some(quoted_platform.as_str()));
        }
        prop_assert!(os.navigator_platforms.contains(&profile.navigator.platform.as_str()));
        prop_assert!(profile.navigator.hardware_concurrency > 0);
        prop_assert!(profile.device == Device::Desktop || profile.navigator.max_touch_points > 0);
        prop_assert!(profile.os != Os::Macos || profile.navigator.max_touch_points == 0);
        prop_assert_eq!(profile.video_card.renderer.contains("Direct3D"), profile.os == Os::Windows);
        prop_assert!(profile.screen.avail_width <= profile.screen.width);
        prop_assert!(profile.screen.avail_height <= profile.screen.height);
        prop_assert_eq!(&profile.navigator.languages, &words(&["fr-FR", "fr"]));
        prop_assert_eq!(header(&profile, "user-agent"), Some(profile.user_agent.as_str()));
        prop_assert_eq!(header(&profile, "accept-language"), Some(engine.accept_language));
        prop_assert!(header(&profile, "accept").is_some());
        prop_assert!(header(&profile, "accept-encoding").is_some());
        prop_assert_eq!(header(&profile, "sec-ch-ua").is_some(), engine.client_hints);
        let sent: Vec<&str> = profile.http.headers.iter().map(|(name, _)| name.as_str()).collect();
        let in_order: Vec<&str> = brand.headers.iter().copied().filter(|name| sent.contains(name)).collect();
        prop_assert_eq!(sent, in_order);
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

    let repeated = drawn
        .iter()
        .enumerate()
        .find(|&(at, profile)| drawn[at + 1..].contains(profile));
    assert_eq!(
        repeated.map(|(at, _)| at),
        None,
        "a seed draws a profile a later seed draws too"
    );
}

#[rstest]
#[case::windows_chrome(EngineFamily::Chromium, Os::Windows, Device::Desktop, Brand::Chrome)]
#[case::macos_chrome(EngineFamily::Chromium, Os::Macos, Device::Desktop, Brand::Chrome)]
#[case::linux_chrome(EngineFamily::Chromium, Os::Linux, Device::Desktop, Brand::Chrome)]
#[case::android_chrome(
    EngineFamily::Chromium,
    Os::Android,
    Device::Mobile,
    Brand::AndroidChrome
)]
#[case::windows_firefox(EngineFamily::Firefox, Os::Windows, Device::Desktop, Brand::Firefox)]
#[case::macos_firefox(EngineFamily::Firefox, Os::Macos, Device::Desktop, Brand::Firefox)]
#[case::linux_firefox(EngineFamily::Firefox, Os::Linux, Device::Desktop, Brand::Firefox)]
fn every_kind_of_the_population_is_drawn_from(
    #[case] family: EngineFamily,
    #[case] os: Os,
    #[case] device: Device,
    #[case] brand: Brand,
) {
    let constraints = whole(family);

    let drawn = (0..400)
        .map(|seed| generate(IdentitySeed(seed), &constraints, ExitFacts::default()))
        .any(|profile| (profile.os, profile.device, profile.brand) == (os, device, brand));

    assert!(
        drawn,
        "no seed of 400 draws {brand:?} on a {device:?} {os:?}"
    );
}
