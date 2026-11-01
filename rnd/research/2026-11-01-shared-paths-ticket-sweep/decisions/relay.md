# Decision log: relay spec

Read-only: nothing was built or run. Code read at `989050b0a`. Web facts checked on 2026-11-01; sources at the end.

## Reversible choices (choice; reason; alternative)

1. All five issues absorbed as tickets; none belongs elsewhere. #1641, #1642, #1646, #1647 and #1675 keep their numbers.
2. The term **Client relay**, setting prefix `client_relay.*` and module `client_relay`; because "relay" already names ADR 0124's TLS relay and the Stub gateway's forwarding; alternative plain `relay`. #1655's channel kind should follow.
3. One WebSocket per Connection plus one control link; because each Client gets its own backpressure and a relay stays very small; alternative one multiplexed link, which needs credits per stream.
4. A new `pairings` module and Pairing store, shared by both ways in; because both need a revocable credential with accounts off; alternative a named token for #1647 (below) and a relay-only store.
5. #1647's credential is a Pairing, not a named token; this overrides its brief; because named tokens need the Accounts store, which is off on a local Daemon, and a bound key cannot be copied out; a named token still works on a Daemon with accounts. So #1647 drops its #1151 blocker.
6. #1647 adds a loopback listener on a machine's Daemon when the list is non-empty; this corrects its brief ("no second listener"); because code shows the launched Daemon has `websocket: None`. See `OWNER` 2.
7. SPAKE2 per RFC 9382 over P-256, built on `p256` and `@noble/curves`, then `Noise_XXpsk3`; the brief asked for a check first, and it failed: no maintained browser SPAKE2 exists, and RustCrypto's `spake2` has its stable release from 2023, a group and transcript I recall as magic-wormhole's rather than the RFC's (from memory, for the `sealing` ticket to confirm), and a pre-release since. Alternatives: `@cipherman/pake-js`'s CPace, which says not to deploy before an audit; a SAS comparison, which is not the typed code the issue asks for. **Least certain:** this is our own short protocol code over audited curves.
8. `Noise_XX_25519_AESGCM_SHA256` with a signed static key (libp2p's pattern), using `snow`; because a Noise pattern is analysed and a hand-made handshake isn't; alternative a SIGMA handshake on P-256, which fits WebCrypto better. **Least certain:** Safari's non-extractable X25519 version is unconfirmed, and the `sealing` ticket must check it. If it fails, P-256 with a hand-specified handshake is the fallback, and `@libp2p/noise` as a library was rejected for its bundle size and its tie to libp2p.
9. Fragments of at most 64 KiB, each sealed; because that is Noise's limit. The word-for-word criterion on #1641 says "one seal and one open per message": true up to 64 KiB, and the row added beside it counts per fragment.
10. "Call key", not "session key", as the person-facing log asked.
11. Signature at the framing, in a binary frame, not in `_meta`; this overrides #1642's brief; because signing parsed JSON needs canonical JSON and a second decode, and #1642's own Shared paths criterion says verified "in `rpc`'s framing" with "no second decode".
12. A sealed Connection takes no signature per call; because the seal already proves each message; alternative both, which adds a verify with no party it stops. This narrows #1642's "each call is signed". **Least certain** of the security choices, so it is easy to review on its own.
13. `accounts.person_check.call_key`, default `browser`; because a CLI over TLS should not need signing to work; alternative `always`.
14. A check resumes on a new Connection within its window; because a phone drops Connections often and a tap per drop would drive people to turn it off. It is held in memory, so the person-facing rule that a restart asks again stands; in a group it is per Group member.
15. `client_relay.person_check` defaults to `required`, and `off` exists only with accounts off; because a compromised interface host must not drive an engine while the person is away; alternative default `off`, which would leave the issue's need to the person's care.
16. On a single-user Daemon the passkey lives with the Pairing, and resolves to the OS user; because the person-facing spec says accounts off means no store; alternative open the Accounts store with one account, which breaks its story 77. The pairing code doubles as #1125's one-time code, as #1642's brief says.
17. The Live view channel is a second sealed Connection of the Pairing, and signalling stays on the channel, not on catalog operations as #1646's text asks; because #999 and the person-facing spec already put it there, and the need is that it's sealed.
18. A relayed viewer must be paired; because a grant's `Origin` check can't see through a relay. **Least certain:** a developer's unpaired end users cannot use a relay.
19. A grant's ICE servers are judged as addresses a page can reach; because a Client supplies them and TURN could otherwise reach a private range; the brief is silent.
20. Control-grant step-up is on minting; a take keeps the attended spec's `first`; narrows #1646's brief, agreeing with attended's decision 13.
21. `@goodfetch/collie-remote` is a new published package; because the promise that a relay can't read depends on correct Client cryptography, and the brief's real-browser tests need this code anyway; alternative only a document. It is not a browser build of the SDKs, so that brief's out-of-scope stands. Publishing is the release's, and it is reversible until the first publish.
22. #1675 drops its #1643 edge; because #1643's own table covers each person-only operation, has no home spec, and would stall this one, as the person-facing log says it should not block; alternative keep the edge. Its #1131 edge becomes #1135 and #1151, which sit after it.
23. #1642 keeps its #1870 gate and gains #1125 and #1138, as asked. Its #1125 and #1138 are person-facing tickets by number, as the brief says for absorbed ones.
24. Prefactors `pairings` and `sealing` come first; `link` before the tracer, because a control link is testable alone and #1641 is already large.
25. Stories run from `story:12-235`; because 139 to 151 are the person-facing spec's, and 201 to 234 hold Foxhound's and others. **Reversible:** the person-facing rule said "from 139", and a publisher may renumber before any test declares them.
26. Stories are `journey = false`, reason `reach`, with conformance cases instead; because ADR 0063 has no third-party surface; alternative a new Journey surface, which is more than this spec needs.
27. All tickets are M4; because their blockers include M4 tickets. #1645 and #1643 have no milestone; see below.
28. E2's gate #1872 goes on `pairings` only, and later Audit entries follow it transitively; E1's #1870 on #1641 and #1642. #1675 and the two grant tickets take none, as #1675's and #1646's briefs found no count added.
29. Paid external review of the cryptography: declined under the Money rule (ADR 0076). Vectors, interop tests, fuzzing and the hostile fixture stand in.

## OWNER

> **Decided by the owner on 2026-11-01 (« Oui, les trois »):** items 1, 2 and 3 as recommended. The listener also opens while Live view is on. The spec and its tickets now state these as decided.

1. **Whether a person gets a relay to run.** Goodfetch hosts none, and the reference relay never ships, so a person can use this only with a relay someone wrote. Recommendation: publish no release artefact; give a small sample run from the repository beside the protocol document, as the person-facing spec does for its viewer page; a shipped binary can be added later without breaking anything. The owner accepted this on 2026-11-01.
2. **A loopback WebSocket on a machine's Daemon when an origin is allowed.** It extends ADR 0049's note that the machine's Daemon serves no WebSocket, and sits beside ADR 0137's rejection of a Daemon endpoint for a local tool, because a page on another origin can now reach a loopback port after the browser's prompt. It differs: it is off unless listed, serves no file, sets no cookie, and every Connection needs a Pairing. Recommendation: accept. The owner accepted this on 2026-11-01. Refusing it would leave only the relay for a local Daemon, and #1647 would become Exposed Daemons only.
3. **The relay protocol as a public contract.** Once published, third-party relays depend on it, so it's a one-way commitment. Recommendation: add it to ADR 0030's contract formats and write it as stable at the first publish, with nothing versioned before (ADR 0066). The owner accepted this on 2026-11-01.

## CLOSE

None.

## ADRs and glossary edits this needs (not written here)

- **New ADR:** a Daemon may connect out to a Client relay the person configures, and admit allowed origins, with a Pairing, sealed Connections and a Call key. Amends ADR 0031 (remote access is no longer only exposure; "a relay through Goodfetch Cloud" stays the Cloud map's, and this one is a configured destination), ADR 0049 (the loopback listener), ADR 0017 (a new configured kind, and the handshake's list), ADR 0024 (a `hello` with no token on a sealed carriage), ADR 0055 (the Pairing store on the Lead Daemon and statements per Group member), ADR 0043 (a note), ADR 0030 (`OWNER` 3).
- **ADR 0137:** a note that this differs from its rejected local endpoint (`OWNER` 2).
- **ADR 0140's register:** `E1.seals` and `E1.signature_checks`, by their tickets.
- **GLOSSARY.md, new:** Client relay (and "relay" alone to be avoided for it), Pairing, Pairing store, Pairing code, Sealed Connection, Call key, Allowed origin.

## For the reviewing session

- **A finding that affects the person-facing spec.** Its log's item 9 left unverified whether a local Daemon has the `daemon.address` listener. It does not: the launched Daemon has `websocket: None`, and only `daemon run` serves one, by reading the code. So #959's "on the WebSocket of `daemon.address` with no new listener" holds only for a foreground Daemon. Decide there whether `live_view.enabled` opens that loopback listener on a machine's Daemon too; this spec's listener opens only for allowed origins, and whichever opens it must share it. I did not edit that spec.
- **On publish, remove the native edges** #1675 → #1643, #1675 → #1131, #1647 → #1151, #1647 → #667 (now `origin-exposed`'s). Add #1642 → #1125 and #1138.
- **The attended spec's `relay` ticket** blocks on #1641, #1642 and #1645. Replace #1642 with `paired-person`, the ticket that gives a relayed Connection a checked person, and add `remote-client` only if its test needs a browser (it doesn't by its criteria). It also says "the surface `third_party` with its name", which holds.
- **The person-facing spec's `#1125`** gains a Call key binding on `account.check` from #1642, with no change to its own criteria. **#1020's component** takes a carriage from #1646.
- **#1645 and #1643 have no milestone or home spec.** #1645 blocks #1641 and #1647, so it needs M4 or earlier. #1643 no longer blocks anything here.
- **#1655 and #1684** stay blocked by #1641, with no milestone yet. #1655's frame is left room, and should read `client_relay`.
- **#1641's and #1642's kept criteria** use the words "session key" and "relay", because the brief asks for them word for word.
- **Browser facts not relied on:** Chromium's official guide says `ws://` to loopback is exempt from mixed content and gated by a permission, with no version given; one third-party post says Chrome 142, and another says WebSockets joined at 147. Firefox 151 and Safari's block are the issue's, unverified. The `relay-guide` ticket checks each against its vendor. No decision depends on them.

## Sources

- [RFC 9382](https://www.rfc-editor.org/rfc/rfc9382.html): the P-256 suite in section 6 and vectors in Appendix B.
- [crates.io `spake2`](https://crates.io/api/v1/crates/spake2): 0.5.0-pre.0 from January 2026, stable 0.4.0; [docs](https://docs.rs/spake2) and [lib.rs](https://www.lib.rs/crates/spake2): 2023 release, no independent audit, probably not constant-time.
- [npm `spake2`](https://socket.dev/npm/package/spake2), [`@cipherman/pake-js`](https://www.pkgstats.com/pkg:@cipherman/pake-js), [`cpace-ts`](https://www.pkgstats.com/pkg:cpace-ts); these come from search snippets, not their repositories.
- [`pake-cpace`](https://crates.io/api/v1/crates/pake-cpace): 0.1.7, from 2023.
- [noble-curves](https://github.com/paulmillr/noble-curves): P-256 and x25519, with audits listed up to v2.3.0.
- [snow](https://crates.io/api/v1/crates/snow), 0.10.0, unaudited by its own statement; [@libp2p/noise](https://libp2p.github.io/js-libp2p/modules/_libp2p_noise.index.html), XX with its bundle size.
- [Igalia on Secure Curves](https://blogs.igalia.com/jfernandez/?p=1913) and [Mozilla's bug](https://bugzilla.mozilla.org/show_bug.cgi?id=1904836): browser X25519.
- [Chrome's Local Network Access guide](https://developer.chrome.com/docs/modern-web-guidance/guides/security/local-network-access).
