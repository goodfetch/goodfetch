# A remote browser is driven through a Browser host beside it

Automan Cloud sells hosted browsers to code and agents that run elsewhere ([Cloud readiness](https://github.com/Jbmanllr/automan/issues/69)). Each of those browsers runs with a Browser host: an Automan process beside it that carries out a Session's operations (click this target, fill this field) with the guard, over a pipe, and returns one answer. A run on the user's machine opens a Session there and pays one network round trip per action, while its run state (cookies, Datasets, journals) stays in the user's engine. The Browser host speaks the same JSON-RPC operations as the engine, so one conformance suite covers both (ADR 0001), and serves one Session before it is wiped. Decided in [Adopt site-access decisions](https://github.com/Jbmanllr/automan/issues/76).

## Considered Options

- **Raw CDP from the user's engine to the hosted browser** (how Playwright reaches Browserbase): rejected as the default. A click takes about five CDP commands, each a network round trip; Browserbase measured 42 ms per round trip on one route ([Stagehand v4 post](https://www.browserbase.com/blog/stagehand-v4)).
- **An in-browser extension** (Stagehand v4): rejected (ADR 0018). The Browser host gets the same single round trip without the unsafe extension flag or a debugging port, and keeps the guard.

## Consequences

- Cookies and storage held by the hosted browser are copied back into the run's state, so a checkpoint covers them (ADR 0002).
- A raw CDP endpoint on a hosted browser stays opt-in, as decided in Cloud readiness, under ADR 0017's rules.
- Third-party browsers (Browserbase, Bright Data's Scraping Browser) are reached over raw CDP with the guard, about two round trips per action once ADR 0018's speed target is met. The [V1 benchmark](https://github.com/Jbmanllr/automan/issues/79) measures this case against Stagehand v4 with equal guarantees; if Automan is clearly slower, an in-browser runtime for such browsers becomes a ticket with the numbers.
