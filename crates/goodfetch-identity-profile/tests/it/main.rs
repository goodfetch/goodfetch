//! `goodfetch-identity-profile` through its public interface: what a drawn
//! Identity profile presents (story 8-36), and how the `stealth.identity.*`
//! settings narrow the population it is drawn from (story 8-38). The
//! expected values are written out, not read from the crate's data
//! (TESTING.md, Levels and layout).

mod drawing;
mod narrowing;

use goodfetch_identity_profile::{Constraints, EngineFamily, ExitFacts, IdentityProfile};

/// `values` as a setting's words.
fn words(values: &[&str]) -> Vec<String> {
    values.iter().map(|&value| value.to_owned()).collect()
}

/// `family`'s whole population.
fn whole(family: EngineFamily) -> Constraints {
    Constraints::narrowed(family, &[], &[], &[]).expect("a family's whole population is drawn from")
}

/// The facts of an exit in France.
fn french_exit() -> ExitFacts {
    ExitFacts {
        country: Some("FR".to_owned()),
        language: Some("fr-FR".to_owned()),
        time_zone: Some("Europe/Paris".to_owned()),
        ..ExitFacts::default()
    }
}

/// The value `profile` sends for the header `name`.
fn header<'a>(profile: &'a IdentityProfile, name: &str) -> Option<&'a str> {
    profile
        .http
        .headers
        .iter()
        .find(|(sent, _)| sent == name)
        .map(|(_, value)| value.as_str())
}
