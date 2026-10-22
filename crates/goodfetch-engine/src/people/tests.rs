//! The sign-in driver behind its interface (spec #592's Seam 3): the
//! single-user link signs the OS user in with a cookie of its own, and
//! nothing else signs anyone in.

use goodfetch_test_support::runtime;

use super::{Principal, SignInDriver, SignInFailure, SignInRefused, SingleUserLink};

const LINK_TOKEN: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";

fn driver() -> SignInDriver {
    SignInDriver::new(SingleUserLink::new(LINK_TOKEN.to_owned()))
}

/// Covers: story:12-1
#[test]
fn the_single_user_link_signs_the_os_user_in_with_a_cookie() {
    runtime().block_on(async {
        let driver = driver();

        let signed_in = driver
            .with_link(LINK_TOKEN)
            .await
            .expect("the link's own token signs in");

        assert_eq!(
            driver.principal(signed_in.cookie()).await,
            Some(Principal::OsUser)
        );
        assert_eq!(
            signed_in.cookie().len(),
            64,
            "32 bytes of the OS generator, in hex"
        );
        assert_ne!(
            signed_in.cookie(),
            LINK_TOKEN,
            "the cookie is not the link's token"
        );
    });
}

#[test]
fn each_sign_in_has_a_cookie_of_its_own() {
    runtime().block_on(async {
        let driver = driver();

        let first = driver.with_link(LINK_TOKEN).await.expect("signs in");
        let second = driver.with_link(LINK_TOKEN).await.expect("signs in again");

        assert_ne!(first.cookie(), second.cookie());
        assert_eq!(
            driver.principal(first.cookie()).await,
            Some(Principal::OsUser)
        );
        assert_eq!(
            driver.principal(second.cookie()).await,
            Some(Principal::OsUser)
        );
    });
}

#[test]
fn a_token_the_link_does_not_carry_signs_nobody_in() {
    runtime().block_on(async {
        let driver = driver();
        let wrong = LINK_TOKEN.replace('0', "1");

        assert_eq!(
            driver.with_link(&wrong).await.err(),
            Some(SignInFailure::Refused(SignInRefused::TokenWrong))
        );
        assert_eq!(
            driver.with_link("").await.err(),
            Some(SignInFailure::Refused(SignInRefused::TokenMissing))
        );
    });
}

#[test]
fn a_cookie_no_sign_in_was_given_is_nobodys() {
    runtime().block_on(async {
        let driver = driver();
        let _ = driver.with_link(LINK_TOKEN).await.expect("signs in");

        assert_eq!(
            driver.principal(LINK_TOKEN).await,
            None,
            "the link's token is no cookie"
        );
        assert_eq!(driver.principal("").await, None);
    });
}
