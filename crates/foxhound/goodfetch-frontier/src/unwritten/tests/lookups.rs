//! A links batch's lookups of the visited set (#2169, G-11 Q6): a job
//! takes the answer its batch read before it, until memory notes a visit
//! of that identity, which may leave memory before the job runs.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use goodfetch_protocol::foxhound::frontier::HttpMethod;
use url::Url;

use super::{commit, holding, no_file, run_file, seed, visited};
use crate::identity::{Declared, Fixed, Key, Seen, read_lookup};

/// A job reads none of the identities its batch looked up while their
/// answers stand; once a job visits one, its answer is stale, so a job
/// reads the file again after that visit left memory, and finds it.
#[test]
fn a_looked_up_answer_stands_until_memory_notes_a_visit_of_its_identity() {
    let file = run_file();
    let commits = Arc::new(AtomicU64::new(0));
    let invariants = holding(&commits);
    let unwritten = invariants.unwritten().unwrap();
    let get = HttpMethod::get();
    let fixed = Fixed::of(
        &Key::of(&Url::parse("http://shop.test/a").unwrap(), false),
        Declared {
            method: &get,
            body: None,
            key: None,
            keeps_fragment: false,
        },
    );

    let lookup = unwritten.look_up([fixed.plain()]).unwrap();
    read_lookup(&file.reader, unwritten, &lookup).unwrap();
    let answered = visited(&no_file(), &invariants, "http://shop.test/a");
    file.writer.execute_batch("BEGIN").unwrap();
    seed(&file.writer, &invariants, "http://shop.test/a", "1");
    commit(&file, &commits, 1);
    let without_the_file = visited(&no_file(), &invariants, "http://shop.test/a");
    let on_the_file = visited(&file.reader, &invariants, "http://shop.test/a");

    assert!(matches!(answered, Ok(Seen::New)));
    assert!(without_the_file.is_err());
    assert!(matches!(on_the_file, Ok(Seen::Visited(visitor)) if visitor == "1"));
    drop(lookup);
}
