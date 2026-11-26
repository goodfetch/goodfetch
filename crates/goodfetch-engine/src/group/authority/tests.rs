use std::time::{Duration, SystemTime};

use rstest::rstest;

use super::{
    GroupAuthority, GroupMemberKey, GroupMemberName, GroupMemberNameInvalid, GroupMemberPublicKey,
    GroupMemberPublicKeyInvalid,
};

#[rstest]
#[case::a_word("lead")]
#[case::a_pod_name("crawler-7f9c-2")]
#[case::one_character("a")]
#[case::sixty_three_characters(&"a".repeat(63))]
fn a_dns_label_names_a_group_member(#[case] name: &str) {
    let named = GroupMemberName::new(name).unwrap();

    assert_eq!(named.as_str(), name);
    assert_eq!(
        named.server_name().to_str(),
        format!("{name}.group.goodfetch.invalid")
    );
}

#[rstest]
#[case::empty("")]
#[case::sixty_four_characters(&"a".repeat(64))]
#[case::uppercase("Lead")]
#[case::a_leading_hyphen("-lead")]
#[case::a_trailing_hyphen("lead-")]
#[case::a_dot("lead.example")]
#[case::non_ascii("lé")]
fn anything_else_names_no_group_member(#[case] name: &str) {
    assert_eq!(
        GroupMemberName::new(name),
        Err(GroupMemberNameInvalid(name.to_owned()))
    );
}

#[test]
fn a_group_members_public_key_is_its_uncompressed_point() {
    let key = GroupMemberKey::generate().unwrap();
    let raw = key.public_key().raw().to_vec();

    assert_eq!(
        GroupMemberPublicKey::from_raw(raw.clone()),
        Ok(key.public_key())
    );
    assert_eq!(raw.len(), 65);
}

#[test]
fn each_revocation_list_is_numbered_one_above_the_last() {
    let issued_at = SystemTime::UNIX_EPOCH + Duration::from_hours(500_000);
    let mut authority = GroupAuthority::create(issued_at).unwrap();

    let first = authority.revocation_list(&[], issued_at).unwrap();
    let second = authority.revocation_list(&[], issued_at).unwrap();

    assert_eq!([first.number, second.number], [1, 2]);
}

#[rstest]
#[case::too_short(vec![0x04; 64])]
#[case::compressed({ let mut point = vec![0x02]; point.extend([1; 32]); point })]
#[case::not_a_point_prefix({ let mut point = vec![0x05]; point.extend([1; 64]); point })]
fn other_bytes_are_no_group_members_public_key(#[case] raw: Vec<u8>) {
    assert_eq!(
        GroupMemberPublicKey::from_raw(raw),
        Err(GroupMemberPublicKeyInvalid)
    );
}
