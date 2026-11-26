//! The one writer of a profile's version fields, so a browser update
//! changes them and nothing else.

use crate::population::{Brand, EngineFamily};
use crate::profile::{BrandVersion, BrowserVersion, IdentityProfile};

/// Sets `profile`'s browser version to `version`: the user agent, the
/// client hints' brands and full version, `navigator.appVersion` on
/// Chromium, and the `user-agent` and `sec-ch-ua` headers. Everything else
/// stays as drawn.
pub fn refresh_version_fields(profile: &mut IdentityProfile, version: BrowserVersion) {
    let major = version.major();
    match profile.family {
        EngineFamily::Chromium => {
            let reduced = format!("{major}.0.0.0");
            let mut user_agent = replace_token(&profile.user_agent, "Chrome/", &reduced);
            for edge in ["Edg/", "EdgA/"] {
                user_agent = replace_token(&user_agent, edge, &reduced);
            }
            profile.navigator.app_version = user_agent
                .strip_prefix("Mozilla/")
                .unwrap_or(&user_agent)
                .to_owned();
            profile.user_agent = user_agent;
        }
        EngineFamily::Firefox => {
            let release = format!("{major}.0");
            let mut user_agent = replace_token(&profile.user_agent, "rv:", &release);
            user_agent = replace_token(&user_agent, "Firefox/", &release);
            // Desktop Firefox freezes its Gecko token; Android's repeats
            // the version.
            if !user_agent.contains("Gecko/20100101") {
                user_agent = replace_token(&user_agent, "Gecko/", &release);
            }
            profile.user_agent = user_agent;
        }
    }
    if let Some(hints) = &mut profile.client_hints {
        let (brands, full_version_list) = brand_lists(profile.brand, &version);
        hints.brands = brands;
        hints.full_version_list = full_version_list;
        version.full().clone_into(&mut hints.ua_full_version);
    }
    for (name, value) in &mut profile.http.headers {
        match (name.as_str(), &profile.client_hints) {
            ("user-agent", _) => value.clone_from(&profile.user_agent),
            ("sec-ch-ua", Some(hints)) => *value = sec_ch_ua(&hints.brands),
            _ => {}
        }
    }
    profile.version = version;
}

/// `text` with the value after `prefix`, up to a space, `)` or `;`,
/// replaced by `value`.
fn replace_token(text: &str, prefix: &str, value: &str) -> String {
    let Some(start) = text.find(prefix).map(|at| at + prefix.len()) else {
        return text.to_owned();
    };
    let end = text[start..]
        .find([' ', ')', ';'])
        .map_or(text.len(), |length| start + length);
    format!("{}{value}{}", &text[..start], &text[end..])
}

/// The characters Chromium's GREASE brand is made of.
const GREASE_CHARACTERS: [&str; 11] = [" ", "(", ":", "-", ".", "/", ")", ";", "=", "?", "_"];
/// The versions Chromium gives its GREASE brand.
const GREASE_VERSIONS: [&str; 3] = ["8", "99", "24"];
/// Where Chromium puts the GREASE brand, `Chromium` and its own brand.
const BRAND_ORDERS: [[usize; 3]; 6] = [
    [0, 1, 2],
    [0, 2, 1],
    [1, 0, 2],
    [1, 2, 0],
    [2, 0, 1],
    [2, 1, 0],
];

/// The brands Chromium of `brand` lists at `version`, with their majors
/// and with their full versions, as Chromium derives them from its major.
fn brand_lists(brand: Brand, version: &BrowserVersion) -> (Vec<BrandVersion>, Vec<BrandVersion>) {
    let major = version.major();
    let seed = usize::try_from(major).expect("invariant: a u32 fits a usize");
    let grease_version = GREASE_VERSIONS[seed % GREASE_VERSIONS.len()];
    let grease = format!(
        "Not{}A{}Brand",
        GREASE_CHARACTERS[seed % GREASE_CHARACTERS.len()],
        GREASE_CHARACTERS[(seed + 1) % GREASE_CHARACTERS.len()]
    );
    let own = match brand {
        Brand::Edge => "Microsoft Edge",
        Brand::Chrome | Brand::AndroidChrome | Brand::Firefox => "Google Chrome",
    };
    let entries = [
        (
            grease,
            grease_version.to_owned(),
            format!("{grease_version}.0.0.0"),
        ),
        (
            "Chromium".to_owned(),
            major.to_string(),
            version.full().to_owned(),
        ),
        (own.to_owned(), major.to_string(), version.full().to_owned()),
    ];
    let mut listed = [0; 3];
    for (entry, &slot) in BRAND_ORDERS[seed % BRAND_ORDERS.len()].iter().enumerate() {
        listed[slot] = entry;
    }
    listed
        .iter()
        .map(|&entry| {
            let (brand, major, full) = &entries[entry];
            (
                BrandVersion {
                    brand: brand.clone(),
                    version: major.clone(),
                },
                BrandVersion {
                    brand: brand.clone(),
                    version: full.clone(),
                },
            )
        })
        .unzip()
}

/// The `sec-ch-ua` header's value for `brands`.
fn sec_ch_ua(brands: &[BrandVersion]) -> String {
    brands
        .iter()
        .map(|entry| format!("\"{}\";v=\"{}\"", entry.brand, entry.version))
        .collect::<Vec<_>>()
        .join(", ")
}
