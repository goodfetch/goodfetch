# A Session, a Page and a browser each have one owner that holds their state, a Page has one reader of its events, and one module of the page crate names CDP

ADR 0038 already asks for a tree of owners "from the engine process … to each run, Session and page", and for state with one owner to be "an actor that receives messages rather than a value behind a shared lock". The page crate never followed it. This ADR says how it does: a Session, a Page and a browser each get a task owner that holds their state as one value; a Page's events have one reader; and CDP is named by one module of the crate. Decided by the owner in a grilling session on 2026-11-10 ([#2254](https://github.com/goodfetch/goodfetch/issues/2254)). It builds ADR 0038's rule and extends it to the browser, and it amends ADR 0140 (C1, C2, C3, C5 and B2), ADR 0085's note on `page_events_lost`, ADR 0143's note on the Session's tracker and ADR 0020's note on the hop page.

What the page crate holds today:

- **No Session and no Page has an owner.** Every Session of the engine is behind one lock, and a Page's events, dialogs, interception, Consent and hiding state sit in tables beside it, each keyed by the Page's ID. Page code starts its tasks on the Run: a browser's supervision, the reader of its root events, one task per paused request.
- **A Page's handle is shared, with the Gesture order behind a lock** held across an `.await`. `STYLE.md` records it as its one exception to "never hold a lock across `.await`", an agent's decision still pending the owner's review.
- **Several tasks read one Page's events, and only one tells a late reader from a Page that went.** The interception's reader ends the same way for both and clears what waits, so the requests it had paused stay paused, unanswered.
- **The connection offers each event to every subscription,** kept in a list behind a lock beside the connection's actor, and a command whose caller timed out stays in the pending map until the browser answers.
  > Note (correction, 2026-11-10, from an independent review of the publication): the connection does not send each event to every subscription. For each event its driver takes the list's lock, compares every subscription's session with the event's and rebuilds the list, and it sends the event only to the subscriptions of that session (`crates/goodfetch-cdp/src/connection.rs:522-549`, the comparison at `:534`, read at `197a8c18e`). What the table removes is that work per event. The same holds for the item of the `Existing code` list below. The decision is unchanged, and [#2264](https://github.com/goodfetch/goodfetch/issues/2264)'s first criterion is restated on that work.
- **CDP is named by 44 production files of the page crate.**
- **A Session moves between browsers.** When its browser is recycled, its cookies and storage are captured and it is seated in another. So a browser cannot be a Session's parent.
- **Firefox and WebKit cannot arrive as "a second transport" under the CDP connection,** as [#739](https://github.com/goodfetch/goodfetch/issues/739) and ADR 0140 said. Playwright's driver at v1.65.0 speaks its own protocol and refuses a raw protocol session outside Chromium (read in Playwright's source for #2254, whose section 1.5 holds the citations).

## Decision

### The tree

- **Run → Session → Page, and Run → browser beside them.** Each Session, each Page and each browser is a `TaskOwner` child of the one above and holds its state as one value.
  > Note (owner's decisions, 2026-11-10, grilling of decision G-14 of the consolidated review: Q5 answered "Yes, all six", with its interim, and Q8 answered "Yes, all seven", each on the grilling's recommended option as put; the wording is the grilling session's, and the owner did not read it; [ADR 0148](0148-a-panic-ends-what-owned-its-task-the-engine-stops-only-for-a-part-every-run-shares-and-nothing-is-restarted-in-place.md)): a panic of a task of a Page's owner closes that Page, and the Run trace records it; one of a Session's owner closes the Session with its Pages; one of a browser's owner is treated as that browser's crash, through the path a real crash takes: its Sessions are told and its Pages raise `page_crashed`. Every later call on what closed raises `engine_panicked`, and the Run goes on; in a crawl the attempt using that Page fails and is retried under the Run's retry rule. Until each owner exists, its tasks are its Run's, and such a panic fails the Run. [#2263](https://github.com/goodfetch/goodfetch/issues/2263), [#2273](https://github.com/goodfetch/goodfetch/issues/2273) and [#2276](https://github.com/goodfetch/goodfetch/issues/2276) carry it.
- **Nothing below a Run is started on the Run itself.**
- **A browser's owner** supervises its process, reads its root events and holds the state that is the browser's own: its popups, its service workers and its hidden pages. The tasks it starts per attached worker or page are its children. A recycled browser stops its tasks by construction.
- **A Session's owner** holds the Session's state. The engine-wide table of Sessions becomes an index from ID to handle.
- **The pool's table of browsers and seats is not part of this.** Spec #2139 rewrites its rules.

### A Page's owner

- **One task holds the Page's state as one value.** Nothing of a Page stays in a table keyed by its ID outside its owner.
- **Its loop never awaits the browser or the program.** Work that waits runs as its child tasks and reports back.
- **It holds the Gesture order and grants the turn in arrival order.** Reads, waits, `evaluate` and answers to Routes and dialogs stay concurrent, as ADR 0085 decides.
- **Closing a Page is stopping its owner.**
- **Three counters join the Quality register:** the round trips to a Page's owner per page read and per action attempt, where the design gives at most one, and the tasks started per Page opened, as ADR 0142 did for the Run state's actor. The rework's first ticket sets their names and their caps from its first figures ([#2263](https://github.com/goodfetch/goodfetch/issues/2263)).

### One reader per Page

- **A Page has one subscription, which its owner reads,** handing each event to the part that wants it.
- **The connection hands an event to its session's reader through a table that its actor owns,** not by offering it to every subscription.
- **A Page whose reader is 4,096 events behind is closed.** Every call on it raises `page_events_lost`, and the Run trace records it. A listener that leaves 1,024 events untaken still loses only itself.
- **The connection never waits on a reader,** and every buffer stays bounded.

> Note (agent decisions of 2026-11-10, awaiting the owner's reading), from the same review. Each fills a gap the review found in this ADR, follows from a sentence of it, and is stated in spec [#2256](https://github.com/goodfetch/goodfetch/issues/2256) and its tickets:
>
> - **One subscription is one reader with one bounded buffer,** not one CDP session. A Page's out-of-process frames and its workers each have a session of their own, and the Page's reader is registered in the connection's table for each. A reader per frame would bring back several buffers and several places that tell a late reader from a Page that went.
> - **`Runtime`, which a handler's `page.on` enables, belongs to the request part** of a released Page's value: "what is enabled on it" is what the engine enables when it sets the page up. This follows from the owner's decision of 2026-11-10 on [#2182](https://github.com/goodfetch/goodfetch/issues/2182), whose release disables it.
> - **The actor that already holds a Session's seat in a browser** (`crates/goodfetch-page/src/pool.rs:889-893` and `:1115`, started on the Run at `:472-474`) stays an actor of its own, since the pool's table of browsers and seats is not part of this ADR, and becomes a child of the Session's owner, since nothing below a Run is started on the Run itself ([#2273](https://github.com/goodfetch/goodfetch/issues/2273)).
> - **A click's burst leaves C1's count as it is:** three input events are three command messages, so `C1.sends` stays at 1. What goes is the wait for an answer between them ([#2270](https://github.com/goodfetch/goodfetch/issues/2270)).
> - **How a subscription still starts without waiting for the connection's driver,** which the connection promises today, once the table of readers is the actor's own, is [#2264](https://github.com/goodfetch/goodfetch/issues/2264)'s to state.

### A released Page

- **One owner lives as long as the browser's page does.** Its value has two parts.
- **What belongs to the browser's page stays across a release:** its subscription, the engine's registered scripts, what is enabled on it, its out-of-process frames.
- **What belongs to the request is replaced:** the page value, the Lease check, the handler's Routes, listeners, init scripts and dialog handlers, the pending navigation.
- **Each request gets a Page ID of its own.** A call on an earlier ID raises `closed`.
- **The owner keeps reading while the Page is spare,** so a navigation its document starts on its own is seen and the Page closes (ADR 0143).

### The boundary with CDP

- **One module of `goodfetch-page` names `goodfetch_cdp`,** behind a concrete type whose narrow primitives a Page's logic calls, in ADR 0129's shape.
- **No trait yet.** It arrives with #739, when Playwright's driver is the second implementation, and is shaped by what that driver can do.
- **C1's Cost contract (1 encode and 1 send per message) applies to that driver's connection,** which is a second connection, not a second transport behind `send_raw`.

### Work done where the event is read

- **A paused request whose verdict needs no wait is answered by the Page's reader,** with no task started, in the Interception's written order. Only a Route with a handler, the Navigation gate or a Permit's wait gets a child task.
- **A click's three input events are written as one burst,** as ADR 0019 asks.
- **A command whose caller timed out or was dropped leaves the connection's pending map.**
- **Not decided here:** sending a page's independent set-up commands together. By the owner's rule of 2026-11-07, browser cuts beyond spec #2139 are decided after [#2183](https://github.com/goodfetch/goodfetch/issues/2183)'s measurement.

### When

The rework ([#2256](https://github.com/goodfetch/goodfetch/issues/2256)) is a step of its own, after spec #2138 and the spec of the `Run` handle ([#2255](https://github.com/goodfetch/goodfetch/issues/2255)), and before spec #1838's CDP tickets and spec #2139. It also edits the page crate's parameters and fields of type `Run`, once, against the identity and the powers that [ADR 0145](0145-a-live-run-is-one-small-read-only-value-and-one-value-per-power-and-the-run-handle-gains-no-method.md) defines.

### What does not change

- Every `page.*`, `locator.*`, `route.*`, `dialog.*` and `session.*` operation keeps its params, its result and its behaviour, except a Page that fell behind, which now closes.
- The Session's and the Page's transition tables stay where they are, and their owners apply them (ADR 0128).
- A program's raw CDP session and a Lent page still pass their commands as sent.
- A Browser host runs the same page crate beside its browser.

## Considered Options

Each reason is the one put to the owner before the answer (#2254, section 9, in its first comment).

- **One owner per Session, holding its Pages' values:** not chosen. Fewer tasks, but with `session.pool.session_concurrency` unset one Session serves the whole crawl, so every Page of that crawl would pass through one task.
- **One handler per browser,** as chromiumoxide v0.9.1 has: not chosen. No message between the connection and the state, but a Session changes browser when its browser is recycled, so its state would change owner each time.
- **No owner per Page, with #2181 and #2182 as written:** not chosen. The order of 2026-11-06 would not move, but the gap with ADR 0038 stays, and so do #2182's first two open questions.
- **A trait above CDP now, with Chromium as its only adapter:** not chosen. It contradicts `STYLE.md`'s "a trait only where two adapters exist", and its shape would be set by CDP alone, to be redone when Playwright's driver is the second.
- **The seam at the connection,** as #739 said: not chosen. A Page's logic would not change, but it needs an emulator of CDP in front of the driver, about 65 commands and 31 events by a count of the page crate's typed names, and several uses cannot be emulated at all.
- **Everything about the seam decided at #739:** not chosen. The rework rewrites the files that name CDP, and #739 would then rewrite them a second time.
- **Today's readers, each made to tell a late reader from a Page that went:** not chosen. The smallest change, which leaves as many buffers and wakes per event as there are readers, and as many places to decide what to do.
- **A connection that waits for its reader:** not chosen. No event is lost, but a slow Page then holds every Page of its browser, and the connection's rule that it never waits on a subscription is reversed.
- **An unbounded buffer,** as chromiumoxide and chromey have: not chosen. Nothing is lost and memory grows, against `STYLE.md`'s rule that every buffer between tasks is bounded.
- **The rework between spec #1838's CDP tickets and spec #2139:** not chosen. #1892's registry of worlds would need one more reader or one more lock, which the rework then moves (an inference: nobody designed #1892 in today's shape to check it).
- **The rework after spec #2139:** not chosen. The order of 2026-11-06 stays whole and #2183 reads its bar soonest, but #2181 and #2182 are written in the shape the rework replaces.
- **An owner per request, stopped at release:** not chosen. Nothing survives a release by construction, but nobody then reads a Spare page's events, and ADR 0143 wants such a Page closed when its document navigates on its own.
- **The same Page ID with a released state:** not chosen. A late call from the earlier handler would reach the next request's page, unless a generation is added, which comes back to an ID per request.
- **Owners for Sessions and Pages only:** not chosen. It is ADR 0038's list to the letter, but the browser's tasks stay on the Run and the tree is not whole there.
- **The browser and the pool both as owners:** not chosen. No shared lock would be left in the crate, but the rework would restructure rules that #2178 and #2179 rewrite right after it.

## Consequences

- **ADR 0038's tree gains the browser.** `STYLE.md`'s sentence on structured concurrency and the header of the engine's `tasks` module follow when the browser's owner is built.
- **`STYLE.md`'s exception for the Gesture lock goes** when the code does.
- **ADR 0140 is amended:** C1's owner sentence and B2's "a scripted Playwright-driver end counts C1's second transport"; C2 and C3 gain the round trip to a Page's owner; C5's "0 tasks spawned per refusal" tightens to every verdict that needs no wait, in the pull request that builds it, by the register's rules.
- **ADR 0085's note of 2026-10-28** on `page_events_lost`, which left the Page open, is replaced. Its note on a slow listener stands.
- **ADR 0143's note of 2026-11-10 on the Session's tracker is replaced:** a Session's tasks belong to the Session's owner, which its retirement stops and waits for.
- **#739, #810, #1121 and spec #1838** carried the "second transport" sentence and are corrected. At #739, spec #578's sentence "`connect()`, which yields the browser's CDP connection" is amended too.
- **Spec #594's Page table gains the Spare state** and the events that enter and leave it, and the event that ends a Page that fell behind.
- **#723 builds the Gesture order on the owner.** #2181 no longer builds a tracker in its smallest form; #2182's first two open questions are answered.
- **The rework's last ticket ([#2279](https://github.com/goodfetch/goodfetch/issues/2279)) blocks #1892, #1893, #1895, #2181 and #2182,** which each write a Page's or a Session's state. Their `Existing code` lists, and those of #1894, #2071, #2177 to #2180 and #2183, go stale where the rework moves their lines and are repaired after it (ADR 0144).
- **Exceptions X10, X11 and X21 of the Quality register expire on 2027-01-30,** and their tickets now run after the rework. If the date cannot hold, the register's rules send the extension to the owner.
- **Collie is measured against Playwright before the rework starts and again after it** ([#2282](https://github.com/goodfetch/goodfetch/issues/2282); the owner's decision of 2026-11-10 in [#2252](https://github.com/goodfetch/goodfetch/issues/2252)). Until its first reading exists, the browser rows of the counts benchmark not rising is the only guard on this rework.
  > Note (2026-11-10, from the same review): #2282 builds the scenario and takes no reading. The first reading is [#2300](https://github.com/goodfetch/goodfetch/issues/2300), taken on the commit the rework starts from, and the second is [#2303](https://github.com/goodfetch/goodfetch/issues/2303), taken on the commit it ends on. The repair of the `Existing code` lists named in the bullet above is [#2304](https://github.com/goodfetch/goodfetch/issues/2304): it leaves out #2177, which is built before the rework starts, and #2178 and #2183, whose lists cite no file of the page crate or of the CDP crate, and it adds #2294. #2179 and #2180 now wait for #2279 too.
- **Two tickets sit beside the rework:** browsers launch on Windows over the debugging pipe ([#2293](https://github.com/goodfetch/goodfetch/issues/2293)), and under `fetch.tier: auto` a Session keeps the page that sends its HTTP hops ([#2294](https://github.com/goodfetch/goodfetch/issues/2294)).

## Existing code

Read at `21fe3ef3d`.

- Every Session of the engine is behind one lock, with the per-Page tables beside it. `crates/goodfetch-page/src/sessions.rs:126-141#Sessions`
- A Page's handle is cloned, and its clones share the Gesture locks. `crates/goodfetch-page/src/driver.rs:1000#PageDriver` `crates/goodfetch-page/src/driver.rs:1017-1020`
- A browser's supervision is started on the Run. `crates/goodfetch-page/src/launch.rs:276`
- The reader of a browser's root events is started on the Run. `crates/goodfetch-page/src/driver/service_workers.rs:214-216#start`
- A browser's popups and service workers are kept behind locks of their own. `crates/goodfetch-page/src/driver/popups.rs:58-61#Popups` `crates/goodfetch-page/src/driver/service_workers.rs:77-86#ServiceWorkerChoices`
- A task is started per paused request, on the Run. `crates/goodfetch-page/src/interception/pipeline.rs:316-318`
- The interception's reader clears what waits when its subscription ends, whatever ended it. `crates/goodfetch-page/src/interception/pipeline.rs:328-329`
- The page's events reader alone tells a subscription that fell behind from a Page that went. `crates/goodfetch-page/src/page_events.rs:1197`
- The connection's subscribers are a list behind a lock. `crates/goodfetch-cdp/src/connection.rs:181-190#Subscribers`
- An event is offered to every subscription, and one that is full ends as fallen behind. `crates/goodfetch-cdp/src/connection.rs:522-549`
- A command enters the pending map when sent. It leaves when the browser answers it, when its answer is dropped for its size, when its session is detached or when the connection ends; its caller's timeout removes nothing. `crates/goodfetch-cdp/src/connection.rs:423` `crates/goodfetch-cdp/src/connection.rs:489` `crates/goodfetch-cdp/src/connection.rs:457#dropped` `crates/goodfetch-cdp/src/connection.rs:555#detached` `crates/goodfetch-cdp/src/connection.rs:320-322`
- A click's three input events are three awaited sends. `crates/goodfetch-page/src/driver.rs:1120-1127#click`
- A Session leaves a recycled browser with its cookies and storage. `crates/goodfetch-page/src/pool.rs:1236-1241#leave`
- The pool's table of browsers is behind a lock of its own. `crates/goodfetch-page/src/pool.rs:145`
- The ownership tree's module names a Session and a page as later owners. `crates/goodfetch-engine/src/tasks.rs:1-3`
