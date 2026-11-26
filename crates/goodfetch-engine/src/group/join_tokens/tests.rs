use std::time::{Duration, SystemTime};

use secrecy::ExposeSecret;

use super::{JoinTokenRefused, JoinTokens};

fn minted_at() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_hours(500_000)
}

#[test]
fn a_join_token_works_once() {
    let mut tokens = JoinTokens::default();
    let minted = tokens.mint(minted_at()).unwrap();
    let token = minted.token.expose_secret();

    assert_eq!(tokens.redeem(token, minted_at()), Ok(()));
    assert_eq!(
        tokens.redeem(token, minted_at()),
        Err(JoinTokenRefused::Unknown)
    );
}

#[test]
fn a_join_token_past_its_lifetime_is_refused_as_expired() {
    let mut tokens = JoinTokens::default();
    let minted = tokens.mint(minted_at()).unwrap();

    assert_eq!(minted.expires, minted_at() + Duration::from_mins(30));
    assert_eq!(
        tokens.redeem(minted.token.expose_secret(), minted.expires),
        Err(JoinTokenRefused::Expired)
    );
}

#[test]
fn a_token_never_minted_is_refused_as_unknown() {
    let mut tokens = JoinTokens::default();
    let minted = tokens.mint(minted_at()).unwrap();
    let mut other = minted.token.expose_secret().to_owned();
    other.replace_range(..1, if other.starts_with('0') { "1" } else { "0" });

    assert_eq!(
        tokens.redeem(&other, minted_at()),
        Err(JoinTokenRefused::Unknown)
    );
    assert_eq!(
        tokens.redeem(minted.token.expose_secret(), minted_at()),
        Ok(())
    );
}

#[test]
fn each_minted_token_is_its_own() {
    let mut tokens = JoinTokens::default();
    let first = tokens.mint(minted_at()).unwrap();
    let second = tokens.mint(minted_at()).unwrap();

    assert_ne!(first.token.expose_secret(), second.token.expose_secret());
    assert_eq!(
        tokens.redeem(second.token.expose_secret(), minted_at()),
        Ok(())
    );
    assert_eq!(
        tokens.redeem(first.token.expose_secret(), minted_at()),
        Ok(())
    );
}
