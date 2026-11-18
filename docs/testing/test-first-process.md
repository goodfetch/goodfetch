## Test-first process

- **An agent implementing a ticket follows the `tdd` skill.** The seams named in the spec's Testing Decisions are the seams agreed in advance ([ADR 0133](../adr/0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md)).
- **Every behaviour lands red, then green.** The pull request's Evidence section (the `pr` skill's) lists each new test with the failure it showed before the code existed as its Before, and the same test passing as its After. Commits stay green.
  _Why:_ a test never seen failing may test nothing, and green commits keep `git bisect` and the "all green" check working.
- **A bug fix starts with a test that reproduces the bug**, named by the behaviour, not the issue.
