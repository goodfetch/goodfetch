# A person's passkey is checked by the engine over the Connection, its ceremony is the Client's, and only a local person surface answers as the OS user

ADR 0137 took the Console out of both Products and kept the features it showed: "Needs you" and its answers, the Live view and its grants, accounts, passkeys, named tokens, the Timeline and failures grouped by cause. Each had been designed as a screen of the Console, and passkeys were to run in a page the Daemon served, behind a cookie keyed by the Console key. No executable serves a page any more. This ADR decides how a person is known and acts with no served page: the engine checks a passkey over the Connection and the Client runs the ceremony, a person answers "Needs you" in the CLI, nothing replaces the Console key, and only a person surface on a local carriage answers as the OS user with no passkey.

Spec [#1937](https://github.com/goodfetch/goodfetch/issues/1937) builds it. Its choices were made under the owner's delegation of 2026-11-01 (« fait ce que tu peux faire, to spec, to tickets etc ») and are recorded in the spec's decision log; they stay open to the owner's review and were not read by the owner. Three were the owner's on 2026-11-01: a Live view frame is a Shared path (ADR 0140's C6), no viewer page comes from the CLI, and an SDK reads and reaches the Live view while a person's answers through it wait for [#1643](https://github.com/goodfetch/goodfetch/issues/1643) (ADR 0137's and ADR 0049's notes of that day). On 2026-11-24 the owner set aside what only several people sharing one Daemon use: this ADR records one person's account, and OIDC, the three roles and an Accounts store of several accounts replicated by a Lead Daemon are [#3662](https://github.com/goodfetch/goodfetch/issues/3662)'s, under [#586](https://github.com/goodfetch/goodfetch/issues/586).

## Decision

### A passkey's ceremony is the Client's, its check the engine's

- **The engine is the WebAuthn relying party,** named by `accounts.passkey.relying_party`, with the origins it accepts in `accounts.passkey.origins`. It issues challenges and verifies answers over the Connection, with no outside service. It serves no route and sets no cookie.
- **The CLI talks to an authenticator directly:** CTAP2 to a roaming key over USB or NFC on every OS, and the OS's WebAuthn API where it opens to an unbundled program (Windows). A bare CLI on macOS cannot reach platform passkeys, which ask for a signed application with an associated domain, so it uses a roaming key there. These platform facts come from third-party documentation; the CLI's ticket ([#1942](https://github.com/goodfetch/goodfetch/issues/1942)) checks them against the vendors' own before it fixes its library.
- **A browser Client** on an origin `accounts.passkey.origins` names uses `navigator.credentials`, the only way to a synced passkey, and calls the same operations. Letting such a Client connect is [#1647](https://github.com/goodfetch/goodfetch/issues/1647)'s, and carrying a check over a relayed or foreign-origin Connection [#1642](https://github.com/goodfetch/goodfetch/issues/1642)'s. Goodfetch Cloud has its own gateway check ([#1410](https://github.com/goodfetch/goodfetch/issues/1410)).
- **A Person check** is an assertion over an engine challenge. It puts the Checked person on that Connection, in memory, for `accounts.person_check.window` (12 hours by default). A Daemon restart asks again. Enrolled public keys persist in the Accounts store.
- **Step-up** stays as spec #592 had it: `step_up_required`'s details carry a challenge bound to the method and its params, valid 2 minutes, and the Client sends the same request again with the assertion in `_meta`. Bound to one request, it needs no other state.
- **Enrolment** of the first passkey takes [#668](https://github.com/goodfetch/goodfetch/issues/668)'s one-time setup code, `daemon.setup_token`, printed once to the logs or read from a Secret reference, in place of ADR 0049's setup link, which pointed at a page.

### Who answers as the OS user

- **A person-only answer needs no passkey only from a person surface on a local carriage:** an Attached engine's stdio or private endpoint, or a machine's Daemon's own listener with its token, from the CLI's interactive prompt or the Kennel (ADR 0141). There the surface claim is read as today.
- **On any other carriage the surface claim counts for nothing.** An Exposed Daemon's address is one even at loopback, where a TLS-terminating proxy carries other machines' Connections (ADR 0049), and so is a Connection a gateway relays, whatever socket it reaches the engine on. A person-only answer needs a Checked person, and Step-up for a write that approves. With accounts off, such a remote answer is refused with `person_required`.

### One person's account

- **Accounts are a setting,** `accounts.enabled`, off on a machine's Daemon and on for an Exposed Daemon. Off, no Accounts store opens and no principal is looked up beyond the OS user.
- **The Accounts store** is one SQLite file per user and major beside the Operator policy, opened only by the Daemon. It holds the person's one Account, its passkey public keys and the hashes of its recovery codes and Named tokens: no sign-ins, no wrapping keys, and no key file beside it. It keeps ADR 0030's default version rule in the format registry and is not versioned before the first publish (ADR 0066).
- **A principal** is resolved once, at `hello` or at a Person check, and held on the Connection: the OS user, the Account, a Named token with its scope, or a Live view grant. Dispatch reads it from memory, with no store read per message (ADR 0140's E1; [#1939](https://github.com/goodfetch/goodfetch/issues/1939) adds the row `E1.store_reads`), and Audit entries take the person from it.
- **A new `accounts` module of `goodfetch-engine`** holds who a Connection is, not the removed `people`. Its submodules are passkeys, named tokens and recovery codes. Under ADR 0129 one driver owns the rate limit, resolving the scope, the Audit entry and a Step-up's freshness, and each credential kind fills one primitive that verifies it into a principal.
- **Operations:** `account.enrol`, `account.check`, `account.regenerate_recovery_codes`, `account.end_checks`, `token.list`, `token.create` and `token.revoke`. Enrolling and creating a token are person-only under Step-up; listing and revoking are ordinary writes, in no default MCP profile. A recovery code gives one Person check. Inviting, role changes, removing an account and an operator's reset are #3662's and [#3650](https://github.com/goodfetch/goodfetch/issues/3650)'s.

### Nothing replaces the Console key

| What it did | Now |
|---|---|
| Keyed the cookie's HMAC, in the store | No cookie |
| Keyed the CSRF token | No ambient credential, so nothing to forge |
| Wrapped the offline cache's key | No browser cache |
| Rotating it signed everyone out | `account.end_checks` ends the person's checks, `token.revoke` a token, `daemon rotate-token` the local token |
| Shared across a Daemon group | A Live view redirect's single-use token is minted by the owning Group member over the peer link and checked in its memory, so no key is shared |

Challenges, grants, codes and tokens come from the OS generator (ADR 0059). No new key file exists.

### A person answers "Needs you" in the CLI

- **`needs-you answer`** lists the items, follows `needs_you.changed` while open, and on a choice runs that kind's prompt from the prompt module of [#1207](https://github.com/goodfetch/goodfetch/issues/1207). Each kind's source names its prompt, so Foxhound adds its kinds without Collie naming them (ADR 0087). `needs-you answer <item>` goes straight to one.
- **`person_required`'s `fix` and a Notification name that command,** with no credential. A confirmation or a pause reached from an agent surface follows [#1708](https://github.com/goodfetch/goodfetch/issues/1708).
- **An SDK reads the list and its events and answers nothing:** ADR 0029's list of surfaces stands.

### The Live view is a channel of the protocol

- **One channel per Session,** separate from the event stream, on the WebSocket of `daemon.address`, with no new listener or port; a machine's Daemon opens that loopback listener while `live_view.enabled` is on (ADR 0049's note of 2026-11-01). It opens with the Daemon's token, a Named token or a Live view grant.
- **A grant's origins are checked at the channel's handshake against `Origin`.** `frame-ancestors` and the embed endpoint go, since nothing is served to frame. `LiveViewGrant.url` is the channel's address. A grant listing no origin admits only a viewer that sends none. A grant admits the channel only, never the protocol's Connection.
- **`@goodfetch/collie-live-view` and its React wrapper are the one viewer,** on the developer's page; the escape chord, touch input and the pop-up keyboard field are the component's. Dialogs, file choosers and downloads are channel messages any Client may handle.
- **On the person's own machine** the `window` tier needs no channel. A person with only the CLI and a headless browser gets a grant through an interactive command and runs a documented sample page; neither the engine nor the CLI serves one.

## Considered Options

- **A loopback page served by the CLI for the ceremony:** rejected. It is a served page with `localhost` credentials that do not travel, against ADR 0137.
- **A browser hand-off only, no CTAP2 in the CLI:** rejected; it needs #1647 first and leaves a headless machine with no passkey.
- **A persisted Person check:** rejected. A restart asking again is cheap and leaves nothing to steal.
- **Keeping ADR 0029's plain surface claim on every carriage:** rejected. A token on an Exposed Daemon could then answer as a person.
- **A Daemon signing key in place of the Console key, or a group-wide key for redirects:** rejected. Nothing needs one, and ADR 0055 keeps the peer link free of shared secrets.
- **SDK methods that answer "Needs you":** rejected, against ADR 0029's surfaces; #1643 owns other Clients' answers.
- **The Live view on a port of its own:** rejected, since ADR 0137 removed the Console's ports.
- **Waiting for #1647 before checking a grant's origins:** rejected; grants would stall on the relay spec.

## Consequences

- **Amends ADR 0029.** On a carriage that is not local, a person surface's claim counts for nothing: a person-only answer needs a Checked person, and Step-up for a write that approves. The local rule stands.
- **Amends ADR 0031.** A grant is tied to its origins by the channel handshake's `Origin`, not `frame-ancestors`; a person is known on a Daemon with accounts by a Person check over the Connection, not a cookie.
- **Amends ADR 0049.** The first operator enrols a passkey on a one-time setup code over a Connection, not a setup link (its note of 2026-11-11).
- **Amends ADR 0055.** No key replaces the Console key; a Live view redirect's channel token is minted by the owning Group member over the peer link and checked in its memory.
- **ADR 0140** already holds C6 by the owner's decision of 2026-11-01; `E1.store_reads` joins the Quality register with #1939.
- **Glossary.** Account, Accounts store, Person check, Checked person, Step-up, Named token, Control lock, Live view grant, Timeline and Failure group join it; Live view names its channel and component.
- **Out of this ADR:** several accounts and roles, OIDC by the Client and a shared Accounts store in a Daemon group (#3662), the relay and foreign-origin Connections (#1641, #1642, #1646, #1647), and person-only answers from other Clients (#1643).
