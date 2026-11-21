## Review checklist

A pull request's tests meet this list, a standard the `code-review` skill applies:

1. Each test sits at a seam the spec names.
2. The Evidence section lists each new test with its red failure.
3. One behaviour per test, named as a behaviour in glossary terms, in the right place for its level.
4. Through the interface: no private call, no `#[cfg(test)]` in product code, no `pub` added for tests, no mock of owned code.
5. Expected values come from an independent source; failures are matched on Error codes.
6. Deterministic and isolated: the rules of [Injection and determinism](injection-and-determinism.md), no real sleep, its own data.
7. Parameterized cases are named.
8. Snapshots are redacted, cover structured output, and every change to one is intended.
9. Fixtures are under 256 KiB, and generated ones have their `.source.toml` file.
10. No helper is duplicated; shared setup is in test support, and a new test adapter crate is on CI's list ([Shared helpers](shared-helpers.md)).
11. A new decoder, listener, Plugin kind, widening setting, Secret kind or output has its tests under [Security](security.md).
12. Each declared Feature is one the test asserts ([Feature coverage](feature-coverage.md)).
13. Each mutant that survives in changed code is killed, or the Evidence section explains it.
14. Each new exception names evidence no pinned test can replace.
15. A spec change and its `features/stories/` file agree.
16. Every changed Reference screenshot is intended ([Accessibility, localization and visual regression](accessibility-localization-and-visual-regression.md)).
17. A new or changed Chromium patch has its row in `chromium/surfaces.toml` and its tests, and each new expectation entry has its category and evidence ([Goodfetch Chromium](goodfetch-chromium.md)).
18. A new message or code from a party Goodfetch did not write has its named case in its translation point's table, which also holds one case for a failure it does not recognize (ADR 0127).
19. A new lifecycle state or event adds a named case for each pair it forms, with the outcome copied from its area spec's table (ADR 0128).
20. A Proving ground page or task a feature's test needs comes in the same pull request as the feature and its test (ADR 0032).
