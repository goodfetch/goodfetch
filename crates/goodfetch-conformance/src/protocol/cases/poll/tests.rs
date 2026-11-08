use super::*;

#[test]
fn the_wait_ends_with_the_first_read_that_holds() {
    let mut reads = 0;

    let found = until(
        Duration::from_secs(60),
        Duration::ZERO,
        || {
            reads += 1;
            reads
        },
        |read| *read == 3,
    );

    assert_eq!(found, 3);
}

#[test]
fn a_wait_that_ends_answers_the_last_read() {
    let found = until(Duration::ZERO, Duration::ZERO, || "not yet", |_| false);

    assert_eq!(found, "not yet");
}

#[test]
fn a_read_that_fails_ends_the_wait_with_its_failure() {
    let found = try_until(
        Duration::from_secs(60),
        Duration::ZERO,
        || Err::<u32, _>("the observations cannot be read"),
        |_| false,
    );

    assert_eq!(found, Err("the observations cannot be read"));
}
