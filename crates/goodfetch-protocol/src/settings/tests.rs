use std::collections::BTreeSet;

use crate::catalog::ValueKind;
use crate::entries::MODULES;

/// Covers: story:3-86
#[test]
fn every_key_of_the_schema_names_its_area_first() {
    let without_area: Vec<&str> = MODULES
        .iter()
        .flat_map(|entries| entries.settings)
        .filter(|setting| {
            let segments: Vec<&str> = setting.name.split('.').collect();
            // A map's entries name the next segment, as `sinks.<name>`.
            let least = if matches!(setting.kind, ValueKind::Map(_)) {
                1
            } else {
                2
            };
            // snake_case: a lowercase letter first, then lowercase letters,
            // digits and underscores, as `egress.rotation_block.ipv4`.
            segments.len() < least
                || segments.iter().any(|segment| {
                    !segment.starts_with(|first: char| first.is_ascii_lowercase())
                        || !segment.bytes().all(|byte| {
                            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                        })
                })
        })
        .map(|setting| setting.name)
        .collect();

    assert_eq!(without_area, Vec::<&str>::new());
}

#[test]
fn every_default_is_a_value_of_its_settings_kind() {
    let wrong: Vec<&str> = MODULES
        .iter()
        .flat_map(|entries| entries.settings)
        .filter(|setting| {
            let default = (setting.default)();
            !(default.is_null() && setting.clearable) && setting.kind.check(&default).is_err()
        })
        .map(|setting| setting.name)
        .collect();

    assert_eq!(wrong, Vec::<&str>::new());
}

#[test]
fn no_key_is_declared_twice_or_holds_another() {
    let names: BTreeSet<&str> = MODULES
        .iter()
        .flat_map(|entries| entries.settings)
        .map(|setting| setting.name)
        .collect();
    let nested: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| {
            names
                .iter()
                .any(|other| other.starts_with(&format!("{name}.")))
        })
        .collect();
    let declared = MODULES.iter().flat_map(|entries| entries.settings).count();

    assert_eq!((nested, declared), (Vec::<&str>::new(), names.len()));
}
