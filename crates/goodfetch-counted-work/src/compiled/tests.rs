use super::CAPACITY;
use crate::{CountedWork, Row};

/// Whether `work` has built its selectors', its regular expressions' and
/// its globs' cache, in that order.
fn caches_built(work: &CountedWork) -> [bool; 3] {
    let compiled = &work.0.compiled;
    [
        compiled.selectors.count().is_some(),
        compiled.regexes.count().is_some(),
        compiled.globs.count().is_some(),
    ]
}

/// A `CountedWork` builds no cache as it is built, so one nothing compiles
/// through, as an owner's placeholder, builds none; the first text of a kind
/// asked for builds that kind's cache and no other's, and counting builds
/// none.
#[test]
fn a_cache_is_built_by_the_first_text_of_its_kind_and_none_before() {
    let work = CountedWork::default();
    let row = Row::PageSelectorCompilations;
    work.count(row);
    let unused = caches_built(&work);
    let _ = work.regex(row, "a+");
    let regex = caches_built(&work);
    let _ = work.glob(row, "*.rs");
    let glob = caches_built(&work);
    let _ = work.selector(row, "p");

    assert_eq!(
        (unused, regex, glob, caches_built(&work)),
        (
            [false, false, false],
            [false, true, false],
            [false, true, true],
            [true, true, true]
        )
    );
}

/// The cache holds at most [`CAPACITY`] compilations: one over it
/// evicts one kept before rather than grows.
#[test]
fn a_compilation_over_the_bound_evicts_rather_than_grows() {
    let work = CountedWork::default();
    let row = Row::PageSelectorCompilations;
    let over = CAPACITY + 64;
    for class in 0..over {
        let _ = work.selector(row, &format!(".c{class}"));
    }

    assert_eq!((work.kept() <= CAPACITY, work.read(row)), (true, over));
}
