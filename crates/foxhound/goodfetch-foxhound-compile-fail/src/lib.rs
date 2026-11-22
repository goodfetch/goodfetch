//! Holds no code: the crate exists for its compile-fail tests
//! (`tests/it/compile_fail/`), the cases of `goodfetch-foxhound`'s crawl
//! API that must not compile. trybuild builds each test's cases in a
//! project of its own that copies this crate's dependencies, so a crate
//! of their own keeps `goodfetch-foxhound`'s dev-dependencies out of that
//! build (TESTING.md, Levels and layout; issue 3413).
