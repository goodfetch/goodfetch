use super::Moment;

const EVENTS: [&str; 3] = ["delivery.begin", "delivery.commit", "checkpoint.written"];

// The expected moments come from another ChaCha20 (Python's `cryptography`,
// key = the seed's 8 little-endian bytes then zeros, nonce and counter 0):
// the first keystream u64 modulo 3 picks the event, the second modulo 5,
// plus 1, the occurrence.

#[test]
fn a_seed_draws_its_pinned_moment() {
    let moment = Moment::drawn(4242, &EVENTS, 5);

    assert_eq!(moment, Moment::new("delivery.commit", 4));
}

#[test]
fn another_seed_draws_another_moment() {
    let moment = Moment::drawn(4243, &EVENTS, 5);

    assert_eq!(moment, Moment::new("checkpoint.written", 5));
}
