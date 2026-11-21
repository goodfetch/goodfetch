## Which stories need a Journey

Decided in [ADR 0075](../adr/0075-a-user-story-needs-a-journey-only-when-it-is-a-persons-goal-a-variant-can-reach.md). Journeys sit at the top of the pyramid, so a story gets one only when it is a person's whole goal that a Journey variant can reach. Lower tests prove the rest.

### The flag

- **Apply three tests to each story, in order, from its own "I want" and "so that".** The first test it fails sets `journey = false` and names that test in `not_journey`. A story that passes all three gets `journey = true` and no `not_journey`.
  1. **`actor`:** the actor is a person using a shipped surface (a user, a developer writing code against an SDK or the CLI, an operator, a Recipe author or user, a Plugin author, the person behind an agent). Contributors, maintainers, the owner as releaser, areas, test support, Clients and Client authors, coding agents, Goodfetch Cloud, site owners and readers of the benchmark fail it.
  2. **`part`:** the story states a whole goal, a task the person sets out to do or see. It fails when its "I want" is one setting's effect, one Error code or a refusal of a wrong input, version, environment or configuration, one event, field or operation detail, or how the engine works. A stop at a Limit the person set, and the failure a person must see during a goal, pass it.
  3. **`reach`:** the outcome shows through a surface of [Journeys](journeys.md), in an isolated variant on one host, with the Proving ground and the services test support gives every variant, and only a person's means (start, stop or kill a process, change a setting, run a command, add or remove a Daemon). It fails when the outcome needs an injected fault, an old released binary, an external account or service, Kubernetes, state outside the variant's Goodfetch home and processes, or the release pipeline's artifacts or channels. An older-format file from the Compatibility corpus is allowed.
- **Write the result in `features/stories/<area>.toml`:** `journey`, and `not_journey = "actor"`, `"part"` or `"reach"` when it is `false`. The Feature check fails on a missing, unknown or extra reason. Each reason must match the story's words, a review standard.

### Evidence

- **Give every story at least one declaring test above the unit level**: an integration test (the Simulation, and multi-process and crash tests of the binary, included), a conformance case, a Client conformance case or a Journey. In-process crash-VFS, model-based and loom tests are unit tests. An exception also meets this rule. The level is read from where the test sits: [Levels and layout](levels-and-layout.md) for unit and integration tests, `conformance/` for conformance cases, a `journeys` folder for Journeys. Otherwise the story is untested, with the cause "no test above the unit level".
- **Prove an unflagged story where its condition can be made:**
  - `actor`: the tests its contract needs (conformance or Client conformance cases, area 1's `xtask` tests, release rehearsal and smoke runs, `check` exceptions, the Proving ground's own tests);
  - `part`: an integration test of the package that owns it (the Simulation included), or a test of a Product's executable (its crash tests included), asserting the setting, Error code, event or mechanism;
  - `reach`: the Simulation, crash and partition tests, multi-process tests, skew tests, release rehearsal and deployment smoke runs, or a `live` or `manual` exception.
- **Test a flagged story's edge cases, Error codes and limits lower down too.** Its Journey proves the goal only.

### Journeys for flagged stories

- **A Journey covers a story only when one of its `[[expect]]` checks asserts the story's outcome.**
- **When a flagged story's goal is an existing Journey's goal, or is its starting step (installing what the variant drives), add the story to that Journey's `covers`** with a check, and a step if the goal needs one. Keep the ID. For any other goal, add a Journey. There is no quota of Journeys per area.
- **A flagged story is due with its Journey at the later of its own milestone and the earliest milestone of a covering variant.** Until then its other declaring tests decide it. From then on it needs one proved variant.
