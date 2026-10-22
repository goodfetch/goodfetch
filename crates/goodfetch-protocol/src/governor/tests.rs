use super::ENTRIES;

/// Every key of the governor is under `governor.`, its floors, ceilings
/// and thresholds included, so `config explain` shows them as one family
/// (ADR 0083).
#[test]
fn every_governor_key_is_under_governor() {
    let outside: Vec<&str> = ENTRIES
        .settings
        .iter()
        .map(|setting| setting.name)
        .filter(|name| !name.starts_with("governor."))
        .collect();

    assert_eq!(outside, Vec::<&str>::new());
}
