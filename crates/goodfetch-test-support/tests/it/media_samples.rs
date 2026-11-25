//! `ProvingGround::codec_samples` and `font_samples`: what the commit's
//! Proving ground serves on `codecs.test` and `fonts.test` for one Identity
//! profile's codecs and font families (ADR 0074), read from its pages.

use goodfetch_test_support::{
    CODECS_HOST, FontAvailability, FontSample, MediaKind, ProvingGround, SamplesError, TestEnv,
};

/// The codecs a fixture Identity profile claims.
const CLAIMED_CODECS: [&str; 3] = ["vp9", "opus", "h264"];

/// The font families a fixture Identity profile lists: a host font and one
/// no host has.
const LISTED_FAMILIES: [&str; 2] = ["Liberation Sans", "Goodfetch Test Sans"];

/// Each claimed codec's sample, in the claimed order, each served at its
/// path.
#[test]
fn codec_samples_lists_each_claimed_codecs_sample() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 404).unwrap();

    let samples = proving_ground.codec_samples(&CLAIMED_CODECS).unwrap();

    let listed: Vec<(&str, MediaKind)> = samples
        .iter()
        .map(|sample| (sample.name.as_str(), sample.kind))
        .collect();
    assert_eq!(
        listed,
        [
            ("vp9", MediaKind::Video),
            ("opus", MediaKind::Audio),
            ("h264", MediaKind::Video),
        ]
    );
    for sample in &samples {
        let answer = ureq::get(format!(
            "http://{}{}",
            proving_ground.address(),
            sample.path()
        ))
        .header("Host", CODECS_HOST)
        .call()
        .unwrap();
        assert_eq!(answer.status(), 200, "{}", sample.path());
    }
}

/// A codec the Proving ground has no sample of could never be shown to
/// decode: listing it is refused.
#[test]
fn codec_samples_refuses_a_codec_with_no_sample() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 404).unwrap();

    let samples = proving_ground.codec_samples(&["h264", "theora"]);

    assert!(
        matches!(&samples, Err(SamplesError::NoSample(codec)) if codec == "theora"),
        "{samples:?}"
    );
}

/// Each listed family must render, and each host font the Identity profile
/// does not list must not; a listed host font is measured once, as listed.
#[test]
fn font_samples_lists_the_listed_families_then_every_other_host_font() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 404).unwrap();

    let samples = proving_ground.font_samples(&LISTED_FAMILIES).unwrap();

    let font = |family: &str, listed, expected| FontSample {
        family: family.to_owned(),
        listed,
        expected,
    };
    assert_eq!(
        samples.fonts[..3],
        [
            font("Liberation Sans", true, FontAvailability::Available),
            font("Goodfetch Test Sans", true, FontAvailability::Available),
            font("DejaVu Sans", false, FontAvailability::Unavailable),
        ]
    );
    let listed_host_font = samples
        .fonts
        .iter()
        .filter(|font| font.family == "Liberation Sans")
        .count();
    assert_eq!(listed_host_font, 1);
    assert!(
        samples.fonts[2..]
            .iter()
            .all(|font| font.expected == FontAvailability::Unavailable),
        "{:?}",
        samples.fonts
    );
}

/// A family the font page refuses, here one with a quote that would break
/// out of its style, is refused, never listed as no font.
#[test]
fn font_samples_refuses_a_family_the_page_refuses() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 404).unwrap();

    let samples = proving_ground.font_samples(&["Arial', serif"]);

    assert!(
        matches!(&samples, Err(SamplesError::Unanswered { reason, .. }) if reason.contains("400")),
        "{samples:?}"
    );
}
