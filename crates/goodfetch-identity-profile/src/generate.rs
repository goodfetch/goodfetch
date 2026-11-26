//! Drawing an Identity profile from the population by seed.

use chacha20::ChaCha20Rng;
use chacha20::rand_core::{Rng as _, SeedableRng as _};

use crate::candidates::{Candidate, all, header_order, hint_platform};
use crate::constraints::Constraints;
use crate::population::EngineFamily;
use crate::profile::{
    BrowserVersion, ClientHints, ExitFacts, HttpProfile, IdentityProfile, IdentitySeed, Navigator,
};
use crate::version_fields::refresh_version_fields;

/// The language a profile shows when the exit's is unknown.
const DEFAULT_LANGUAGE: &str = "en-US";

/// The Identity profile `seed` draws among the browsers `constraints`
/// admits, by their share of real traffic, with `exit`'s facts: the same
/// arguments always give the same profile.
pub fn generate(seed: IdentitySeed, constraints: &Constraints, exit: ExitFacts) -> IdentityProfile {
    let mut key = [0; 32];
    key[..8].copy_from_slice(&seed.0.to_le_bytes());
    let mut rng = ChaCha20Rng::from_seed(key);
    let admitted: Vec<(&Candidate, f64)> = all()
        .iter()
        .filter(|candidate| constraints.admits(candidate.kind))
        .map(|candidate| (candidate, candidate.weight))
        .collect();
    let candidate = *pick(&mut rng, &admitted);
    let kind = candidate.kind;
    let hints = pick(&mut rng, &candidate.client_hints).clone();
    let navigator = Navigator {
        platform: pick(&mut rng, &candidate.platform).clone(),
        vendor: pick(&mut rng, &candidate.vendor).clone(),
        product_sub: pick(&mut rng, &candidate.product_sub).clone(),
        app_version: pick(&mut rng, &candidate.app_version).clone(),
        oscpu: pick(&mut rng, &candidate.oscpu).clone(),
        hardware_concurrency: *pick(&mut rng, &candidate.hardware_concurrency),
        device_memory: *pick(&mut rng, &candidate.device_memory),
        max_touch_points: *pick(&mut rng, &candidate.max_touch_points),
        languages: languages(exit.language.as_deref().unwrap_or(DEFAULT_LANGUAGE)),
    };
    let screen = *pick(&mut rng, &candidate.screen);
    let video_card = pick(&mut rng, &candidate.video_card).clone();
    let upgrade_insecure_requests = *pick(&mut rng, &candidate.upgrade_insecure_requests);
    let mut sent = vec![
        ("user-agent", String::new()),
        ("accept", pick(&mut rng, &candidate.accept).clone()),
        (
            "accept-encoding",
            pick(&mut rng, &candidate.accept_encoding).clone(),
        ),
        (
            "accept-language",
            accept_language(kind.brand.family(), &navigator.languages),
        ),
    ];
    if upgrade_insecure_requests {
        sent.push(("upgrade-insecure-requests", "1".to_owned()));
    }
    if let Some(hints) = &hints {
        sent.extend([
            ("sec-ch-ua", String::new()),
            (
                "sec-ch-ua-mobile",
                if hints.mobile { "?1" } else { "?0" }.to_owned(),
            ),
            (
                "sec-ch-ua-platform",
                format!("\"{}\"", hint_platform(kind.os)),
            ),
        ]);
    }
    let headers = header_order(kind.brand)
        .iter()
        .filter_map(|name| {
            let (_, value) = sent.iter().find(|(sent, _)| sent == name)?;
            Some((name.clone(), value.clone()))
        })
        .collect();
    let version = match &hints {
        Some(hints) => hints.full_version.clone(),
        None => BrowserVersion::parse(&format!("{}.0", kind.major))
            .expect("invariant: a major and a zero make a version"),
    };
    let mut profile = IdentityProfile {
        family: kind.brand.family(),
        os: kind.os,
        device: kind.device,
        brand: kind.brand,
        version: version.clone(),
        user_agent: candidate.user_agent.clone(),
        client_hints: hints.map(|hints| ClientHints {
            brands: Vec::new(),
            full_version_list: Vec::new(),
            mobile: hints.mobile,
            platform: hints.platform,
            platform_version: hints.platform_version,
            architecture: hints.architecture,
            bitness: hints.bitness,
            model: hints.model,
            ua_full_version: String::new(),
        }),
        navigator,
        screen,
        video_card,
        http: HttpProfile { headers },
        exit,
    };
    refresh_version_fields(&mut profile, version);
    profile
}

/// One of `values`, drawn by its probability.
fn pick<'a, T>(rng: &mut ChaCha20Rng, values: &'a [(T, f64)]) -> &'a T {
    let total: f64 = values.iter().map(|(_, probability)| probability).sum();
    let mut anchor = f64::from(rng.next_u32()) / f64::from(u32::MAX) * total;
    for (value, probability) in values {
        if anchor < *probability {
            return value;
        }
        anchor -= probability;
    }
    &values
        .last()
        .expect("invariant: a population candidate keeps a value of each property")
        .0
}

/// `navigator.languages` for the exit's `language`: it, then its primary
/// subtag.
fn languages(language: &str) -> Vec<String> {
    match language.split_once('-') {
        Some((primary, _)) => vec![language.to_owned(), primary.to_owned()],
        None => vec![language.to_owned()],
    }
}

/// The `accept-language` header `family`'s browsers send for `languages`.
fn accept_language(family: EngineFamily, languages: &[String]) -> String {
    let weight = match family {
        EngineFamily::Chromium => "0.9",
        EngineFamily::Firefox => "0.5",
    };
    match languages {
        [language, primary] => format!("{language},{primary};q={weight}"),
        _ => languages.join(","),
    }
}
