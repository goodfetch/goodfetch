# Decision log: person-facing features spec

## Reversible choices (choice; reason; alternative)

1. The passkey ceremony is the Client's and the check the engine's, with `account.enrol`/`account.check` and challenges over the Connection; because no executable serves a page (ADR 0137); alternative a loopback page from the CLI, rejected as a served page with `localhost` credentials that do not travel.
2. The CLI uses CTAP2 to roaming keys everywhere and the OS API on Windows; because a bare CLI on macOS has no entitlement for platform passkeys; alternative browser hand-off only, which needs #1647 first. The platform facts come from third-party docs only, and `cli-passkey` must check them against the vendors.
3. "Person check" names a verified person held on a Connection in memory, with a 12-hour window from #1642's brief; because "session" is the glossary's browsing identity; alternative a persisted check, rejected since a restart asking again is cheap and leaves nothing to steal.
4. An OS-user answer needs a local carriage, and a remote `cli` claim counts for nothing; because a token could otherwise answer as a person on an Exposed Daemon; alternative keep ADR 0029's plain claim. With accounts off a remote answer is refused.
5. Nothing replaces the Console key; because each of its jobs is gone or owned elsewhere (spec's table); alternative a Daemon signing key, which nothing needs.
6. A group redirect's channel token is minted by the owning member over the peer link and checked in its memory; because no shared key is then needed; alternative a group-wide key, which ADR 0055 rejects in spirit for the peer link.
7. `needs-you answer` is the one place, with each source naming its prompt; because #1207's module exists and Foxhound must add kinds unnamed (ADR 0087); alternative one command per kind only. Per-operation commands remain #1643's catalog field.
8. An SDK reads "Needs you" and answers nothing; because ADR 0029's surface list stands and #1643 owns other Clients' answers; alternative open SDK methods, against ADR 0029.
9. The Live view channel lives on `daemon.address`'s WebSocket; because ADR 0137 removes the Console's ports; alternative its own port. Unverified: that a local Daemon always has this listener up; #959 settles it.
10. A grant's origins are checked against `Origin` at the channel's handshake, and `frame-ancestors` and the embed endpoint go; because nothing is served to frame; alternative wait for #1647, which would stall grants on the relay spec.
11. The escape chord, touch and pop-up field become the component's; #1020 now comes before #987 and #995, reversing the old edge; because the Console screen they lived in is gone.
12. The Timeline in the CLI is text plus files by handle (`run timeline`), and failures are `run failures`; because a terminal needs no replay surface; alternative a viewer page.
13. New module `accounts` and settings `accounts.*`, not `people`; because #1752's criterion names no `people` module and one name reads better; alternative keep `people.*` settings. #1675's brief says `people.tokens`: its spec reads `accounts.tokens`.
14. OIDC: the Client runs the Device Authorization Grant and the engine checks the ID token over its own nonce, so `client_secret` and the callback go; because no route is served; alternative authorization code with a CLI loopback listener, which suits an IdP without device flow. Some IdPs lack device flow: a follow-up can add loopback.
15. `account.*` reaches follow #1675's brief, and `account.invite` and `account.enrol` are both kept (the operator's code and the person's use of it); because the brief supersedes #1151's text on that point; alternative #592's person-only list.
16. Prefactors `principal` and `accounts-store` are new tickets, ahead of #959 and #1125; because every later ticket asks who acts and the store is removed by #1752.
17. Every one of the 45 got a new body, so no `body_file` is null; because each has a stale `## Parent`, and all but #1055 had Console criteria or wording.
18. Story IDs are kept and new ones start at 139; because ADR 0072 forbids renumbering; alternative a new area's stories file.
19. `console` Journeys become `cli` variants with the same `covers`; because ADR 0075 needs a variant a surface can reach; alternative dropping them.
20. Blockers dropped: #817, #826, #835, #839, #846, #852, #859, #862, #880, #887, #912, #1219, #1226, #235 (merged or closing with #1583, or Console only; each chain now reaches #1752 or #1751), and #1070, #1083, #1172 (closed not planned); #166, #164, #201, #216, #228, #257, #311, #375, #379 (closed). Kept #1208 and #1211 on #1217, and #1211 on #1247.
21. New blockers: #1751, #1752, #1207, #1213, #1050, #1121, and gate tickets #1861, #1863, #1870, #1872, #1874, #1853, #1854, #1856. C5 #1868 is not used: #1032's row is covered by #1856, as the brief prefers.
22. #1220 moves from M1 to M2; because its CLI prompt needs #1207 (M2) and the milestone rule forbids the reverse. All others keep theirs.
23. #1075's picker becomes engine rules per kind, and its prompts stay #1212's; because a picker is a screen; keeping it in M3 by not blocking on #1212 (M4).
24. #1094 and #1120 stay open but only hold the engine's part, with #1213's prompt as their blocker; because #1213 does not hold the item or the hold.
25. `E1.store_reads` (0) is added by `principal`'s pull request; a tightening within the contract. Both sweep criteria on #1131 and #1151 said no such row exists yet.
26. #1105 records in a headful window or the Live view, and #1684 keeps streaming steps to a Client; because #1684's brief names that as its own.

## OWNER

1. **A Live view frame as a new Shared path.** Per-frame work that no row owns (ADR 0140's 17 paths omit it). Recommendation: add a path to ADR 0140, unit one frame, owner the Live view driver, contract 0 file opens, 0 compilations, 0 tasks spawned, 1 acknowledgement through C1, with a B2 cell, viewer on and off. The spec assumes this, and #959 is written to it.
2. **A viewer the CLI opens.** A person with the CLI only and a headless browser can see the Live view only through their own page. A one-shot loopback page from the CLI would be simpler for them, but is a served page against ADR 0137's intent. Recommendation: do not build it; a documented sample page they run themselves, and later the Cloud. The spec assumes that.
3. **ADR 0137's internal discrepancy.** Its Decision names "an SDK" among ways a person acts, and its Consequences name only the CLI and later Goodfetch Cloud. Recommendation: read as this spec does, an SDK reads and reaches the Live view, and answers wait for #1643; a one-line note on ADR 0137.

## CLOSE

None. Every one of the 45 keeps an engine or CLI part. Parts of tickets that died: the offline-cache half of #1220, the picker of #1075, the Console key of #1185.

## ADRs and glossary edits this needs (not written here)

- **New ADR:** the person's passkey with no served page, with the three questions and the local-carriage rule. Amends ADR 0029 (a remote claim counts for nothing), ADR 0031 (grants by `Origin`, OIDC by the Client, person check), ADR 0049 (a setup code, not a link), ADR 0055 (no Console key; a channel token minted by the owner).
- **ADR 0140:** OWNER item 1, if accepted, with `E1.store_reads` in the register by its ticket.
- **ADR 0137:** the note in OWNER item 3.
- **GLOSSARY.md, new:** Person check, Checked person, Step-up, Named token, Accounts store, Account, Viewer, Member, Operator (a role), Control lock, Live view grant, Timeline, Failure group.
- **GLOSSARY.md, edits:** Needs you (the engine's list, not the Console's), Live view (its channel and component; takes control under a Human pause).

## For whoever publishes, and for the spec after this one

- **#1642 must be blocked by #1125 and #1138, and narrowed.** Those two already own enrolment, the assertion check, and `step_up_required`. #1642 keeps the key per call and its relay and foreign-origin Connection. Its "session key" needs another word. Without this the two specs duplicate each other, or interlock.
- **#1675 narrows** to what #1135 and #1151 leave: profile checks, surfaces. **#1643 and #1708** stay out of this spec and block nothing here: they follow on #1096, #1138 and #1642.
- **#1654** builds on #1147's check.
- **Wording to change in tickets outside this spec:** #1394 and #1461 (`notifications.console_url`, a link opening the Live view: the Notification names the command, ADR 0031's "start the Daemon for a link" goes); #1207, #1210, #1212, #1213 ("names the Console"). #1207 builds a reconciliation prompt with no `run.reconcile` yet: #1064 builds the operation, and #1207 is earlier by milestone. Check that #1207 lands its Limit raise first and leaves the reconciliation wiring to #1064, or give it a stub-free order.
- **Soft notes on the old issues** (#732, #747, #1067, #976, #1313, #719, #754, #745, #929, #885, #941) keep their meaning with "the Console" read as a Client.
- **The ticket `principal` tightens behaviour** for any test or user driving person-only calls on a remote `cli` claim. No such caller was found by reading; that is the `unproven` fact.
