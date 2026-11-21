use rstest::rstest;
use serde_json::json;

use super::BlockCause;
use crate::block::BlockCause as CollieBlockCause;

/// Foxhound's type knows every Block cause of both Products, since
/// Foxhound contains Collie, each on the wire as written.
#[rstest]
#[case::challenge(BlockCause::Challenge, "challenge")]
#[case::captcha(BlockCause::Captcha, "captcha")]
#[case::rate_limit(BlockCause::RateLimit, "rate_limit")]
#[case::ip_block(BlockCause::IpBlock, "ip_block")]
#[case::decoy(BlockCause::Decoy, "decoy")]
#[case::payment_required(BlockCause::PaymentRequired, "payment_required")]
#[case::consent_required(BlockCause::ConsentRequired, "consent_required")]
fn each_cause_of_both_products_is_foxhounds_on_the_wire_as_written(
    #[case] cause: BlockCause,
    #[case] wire: &str,
) {
    let written = serde_json::to_value(&cause).unwrap();
    let read: BlockCause = serde_json::from_value(json!(wire)).unwrap();

    assert_eq!((written, read), (json!(wire), cause));
}

#[rstest]
#[case::known(CollieBlockCause::Captcha, BlockCause::Captcha)]
#[case::kept_decoy(
    serde_json::from_value(json!("decoy")).unwrap(),
    BlockCause::Decoy
)]
fn a_cause_collie_read_is_foxhounds_cause_of_the_same_name(
    #[case] read: CollieBlockCause,
    #[case] cause: BlockCause,
) {
    assert_eq!(BlockCause::from(read), cause);
}

#[test]
fn a_cause_neither_product_knows_stays_unknown() {
    let read: CollieBlockCause = serde_json::from_value(json!("a_newer_cause")).unwrap();

    let cause = BlockCause::from(read);

    assert_eq!(
        (matches!(cause, BlockCause::Unknown(_)), cause.as_str()),
        (true, "a_newer_cause")
    );
}
