//! Field encryption through the module's interface (spec #564's `run_key`
//! seam): a sealed value opens back to itself only under its Run's key, at
//! its own table, column and row; moved anywhere else, under another Run,
//! or with any bit changed, it does not open. A data key wrapped by a
//! key-encryption key comes back only with that key, for its own Run, and
//! the wrapped bytes hold neither key.

use proptest::prelude::*;
use secrecy::SecretString;

use super::{KeyError, RunKey, Wrapped, sealing};
use goodfetch_protocol::run::RunId;

fn run(name: &str) -> RunId {
    RunId::parse(name).unwrap()
}

/// Whether `haystack` holds `needle` anywhere.
fn holds(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// A place a value is sealed at: its table, column and row.
fn place() -> impl Strategy<Value = (String, String, String)> {
    ("[a-z_]{1,8}", "[a-z_]{1,8}", "[0-9a-z]{0,6}")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Covers: story:4-64
    #[test]
    fn a_sealed_value_opens_back_at_its_place_and_holds_no_plaintext(
        plaintext in proptest::collection::vec(any::<u8>(), 0..256),
        (table, column, row) in place(),
    ) {
        let key = RunKey::generate(run("sealed")).unwrap();

        let sealed = key.seal(&table, &column, &row, &plaintext).unwrap();
        let opened = key.open(&table, &column, &row, &sealed).unwrap();

        prop_assert_eq!(opened.as_slice(), plaintext.as_slice());
        prop_assert!(plaintext.len() < 8 || !holds(&sealed, &plaintext));
    }

    /// Covers: story:4-64
    #[test]
    fn a_sealed_value_moved_to_another_place_does_not_open(
        plaintext in proptest::collection::vec(any::<u8>(), 0..64),
        sealed_at in place(),
        moved_to in place(),
    ) {
        prop_assume!(sealed_at != moved_to);
        let key = RunKey::generate(run("sealed")).unwrap();
        let (table, column, row) = &sealed_at;
        let sealed = key.seal(table, column, row, &plaintext).unwrap();

        let (table, column, row) = &moved_to;
        let opened = key.open(table, column, row, &sealed);

        prop_assert!(matches!(opened, Err(KeyError::DoesNotOpen)));
    }

    /// Covers: story:4-64
    #[test]
    fn a_sealed_value_with_any_bit_changed_does_not_open(
        plaintext in proptest::collection::vec(any::<u8>(), 0..64),
        bit in any::<prop::sample::Index>(),
    ) {
        let key = RunKey::generate(run("sealed")).unwrap();
        let mut sealed = key.seal("t", "c", "1", &plaintext).unwrap();
        let bit = bit.index(sealed.len() * 8);
        sealed[bit / 8] ^= 1 << (bit % 8);

        prop_assert!(matches!(key.open("t", "c", "1", &sealed), Err(KeyError::DoesNotOpen)));
    }

    /// Covers: story:4-64
    #[test]
    fn a_wrapped_data_key_comes_back_only_with_its_key_encryption_key_for_its_run(
        kek in "[ -~]{1,64}",
        other in "[ -~]{1,64}",
        bit in any::<prop::sample::Index>(),
    ) {
        prop_assume!(kek != other);
        let key = RunKey::generate(run("wrapped")).unwrap();
        let kek = SecretString::from(kek);
        let wrapped = key.wrap(&kek).unwrap();
        let sealed = key.seal("t", "c", "1", b"cookie").unwrap();

        let back = RunKey::unwrap(run("wrapped"), &wrapped, &kek).unwrap();
        let with_another = RunKey::unwrap(run("wrapped"), &wrapped, &SecretString::from(other));
        let for_another_run = RunKey::unwrap(run("another"), &wrapped, &kek);
        let mut tampered = wrapped.as_bytes().to_vec();
        let bit = bit.index(tampered.len() * 8);
        tampered[bit / 8] ^= 1 << (bit % 8);
        let tampered = RunKey::unwrap(run("wrapped"), &Wrapped::from_bytes(tampered), &kek);

        let opened = back.open("t", "c", "1", &sealed).unwrap();
        prop_assert_eq!(opened.as_slice(), b"cookie");
        prop_assert!(matches!(with_another, Err(KeyError::DoesNotOpen)));
        prop_assert!(matches!(for_another_run, Err(KeyError::DoesNotOpen)));
        prop_assert!(matches!(tampered, Err(KeyError::DoesNotOpen)));
        let bytes = sealing::unwrap("wrapped", wrapped.as_bytes(), &kek).unwrap();
        prop_assert!(!holds(wrapped.as_bytes(), bytes.as_slice()));
    }
}

/// The same key's bytes at another Run's place open none of the first
/// Run's values: the Run ID is part of each value's associated data.
/// Covers: story:4-64
#[test]
fn a_sealed_value_moved_to_another_run_does_not_open() {
    let kek = SecretString::from("kek");
    let key = RunKey::generate(run("first")).unwrap();
    let sealed = key.seal("t", "c", "1", b"token").unwrap();
    let bytes = sealing::unwrap("first", key.wrap(&kek).unwrap().as_bytes(), &kek).unwrap();

    assert!(matches!(
        (
            sealing::open_field(&bytes, "first", ["t", "c", "1"], &sealed).map(|opened| opened.to_vec()),
            sealing::open_field(&bytes, "second", ["t", "c", "1"], &sealed),
        ),
        (Ok(opened), Err(KeyError::DoesNotOpen)) if opened == b"token"
    ));
}

/// Each name is length-prefixed in the associated data, so moving a
/// character from the table's name to the column's is another place.
/// Covers: story:4-64
#[test]
fn moving_a_character_between_the_names_of_a_place_is_another_place() {
    let key = RunKey::generate(run("first")).unwrap();
    let sealed = key.seal("ab", "c", "1", b"token").unwrap();

    assert!(matches!(
        key.open("a", "bc", "1", &sealed),
        Err(KeyError::DoesNotOpen)
    ));
}

/// Two seals of one value at one place differ: each draws its own nonce.
/// Covers: story:4-64
#[test]
fn two_seals_of_one_value_differ() {
    let key = RunKey::generate(run("first")).unwrap();

    assert_ne!(
        key.seal("t", "c", "1", b"token").unwrap(),
        key.seal("t", "c", "1", b"token").unwrap()
    );
}

/// A Run key's `Debug` shows its Run, never its bytes.
#[test]
fn a_run_keys_debug_shows_no_key() {
    let key = RunKey::generate(run("first")).unwrap();

    assert_eq!(format!("{key:?}"), "RunKey { run_id: \"first\", .. }");
}
