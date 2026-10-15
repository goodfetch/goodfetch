# A task for a developer using Goodfetch's SDK

You are a developer writing a program with Goodfetch's SDK, a library for browser automation (Collie) and crawling (Foxhound). You have not seen this SDK before: its API exists only as type stubs with doc comments, in the read-only folder `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch`. Learn the API from those files.

## Language

Rust, edition 2024, as a cargo example of a package that depends on `goodfetch-collie`, `goodfetch-foxhound` (both with their `test-util` feature on) and `goodfetch-collie-client`, plus futures-core 0.3, schemars 1.2, serde 1 (derive) and serde_json 1, and as dev-dependencies tokio 1 (macros, rt-multi-thread, signal), axum 0.8 and futures-util 0.3. The crates' stubs (signatures with `unimplemented!()` bodies and doc comments) are under `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch/rust/` (`collie-client/src`, `collie/src`, `foxhound/src`). The example is checked with `cargo check` with warnings denied, and with the test profile too if it holds tests; the workspace sets `missing_docs = "warn"`.

## The task

> On https://quotes.toscrape.com/, using steps written in plain words rather than selectors: open the tag "love" from the tag list, go to its second page, and open the "about" page of the first quote's author. Before returning, confirm without a model that the page reached is an author page (its address starts with https://quotes.toscrape.com/author/). Return the author's name, birth date and birthplace. Keep what the model resolved for each step in a file next to the program. A second run must repeat the same steps with no model call at all. If the site changes so that one kept step no longer applies, only that step goes back to the model, and the program says which step that was.

## How to work

- Your working folder is `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-rs-03`. Write the complete program to `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-rs-03/s03_replay_ai_steps.rs`.
- Read nothing outside your working folder and the library folder named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-rs-03/check` (`cargo check` with warnings denied). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
