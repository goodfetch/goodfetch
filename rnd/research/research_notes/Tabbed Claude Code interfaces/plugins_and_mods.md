# Claude Code plugins and mods that add tabbed UI

Research date: 2026-10-13. Mods launched on 2026-10-01 (Claude Code v2.1.287), so everything here is less than two weeks old. The official reference describes the API "as of v2.1.290". Community repos change daily, and most have 0 to 2 stars.

## What can the official mods UI API do (panes, bands, tabs, components), and can it embed a browser, render a diff, or host or open another session?

### Takeaway
A mod can open **panes** (a docked right sidebar, or a framed region above the prompt). When several panes are open, Claude Code puts **one tab per pane**. A mod can also draw the **band above the prompt** and restyle Claude Code's own rows. Inside a pane it draws a fixed element tree: text, buttons, inputs, Markdown, syntax-highlighted code, **unified diffs**, SVG (Desktop only), and pixel images (terminal only). A pane **cannot embed a webview, an HTML page, a terminal/PTY, or a live interactive Claude session**. The closest it gets is a terminal-only `Image` that a mod can repaint with framebuffer pixels (the type docs show a `'browser'` example), plus messaging and reading of subagent status.

### Cited Findings
**Surfaces and tabs**
- A mod can draw "a pane beside the transcript or a band above the prompt, with tabs, buttons, and text fields". It can also redraw Claude Code's own parts: tool rows, the spinner, the question dialog. — [Mods overview](https://code.claude.com/docs/en/plugins/mods)
- Mods run in the CLI and in the Desktop app's Code tab. Their UI does **not** appear in the VS Code extension chat panel, in `claude -p` or the Agent SDK, or in cloud sessions. WSL Desktop sessions don't load plugins at all. — [Mods overview, "Where mods run"](https://code.claude.com/docs/en/plugins/mods)
- A pane is "a sidebar beside the transcript in a wide fullscreen terminal, or a framed region above the prompt otherwise. With several panes open, each gets a tab that shows its title." — [Draw in the interface](https://code.claude.com/docs/en/plugins/mods/interface)
- The `title` field of `$.ui.open` is "The pane's tab label when more than one pane is open". Other fields: `focus`, `closeOnEscape`, `holdToasts`, `rows`, `columns`. — [Draw in the interface](https://code.claude.com/docs/en/plugins/mods/interface)
- Tabs *inside* one pane are not a widget. The official `hello-tabs` example fakes them with a row of `plain` Buttons plus module state ("The tabs are two buttons in a row. The mod keeps track of which one is active"). — [Draw in the interface](https://code.claude.com/docs/en/plugins/mods/interface)
- A pane the mod opens unprompted only appears at 144 or more terminal columns (110 after the user has opened it once). One the user opens appears at any width. Ctrl+X then an arrow key resizes it, and Ctrl+X then X closes it. — [Draw in the interface](https://code.claude.com/docs/en/plugins/mods/interface); [Reference, Limits](https://code.claude.com/docs/en/plugins/mods/reference)
- Render sites: `Pane` and `AbovePrompt` (both surfaces), plus Claude Code's own `UserMessage`, `AssistantMessage`, `ToolUse`/`ToolResult`/`ToolGroup`, `CommandOutput`, `AskUserQuestion`, `Spinner`, `SessionMode`, `PromptHint` (both surfaces), and `ToolProgress`, `TurnDuration`, `InfoNotice` (terminal only). The permission prompt is not a render site. — [Reference, Render sites](https://code.claude.com/docs/en/plugins/mods/reference)

**Elements (the complete list)**
- `Box`, `Text`, `Button`, `Link`, `Code`, `Markdown`, `Input`, `Select`, `Client`: terminal and Desktop. `Svg` (up to 131,072 characters): Desktop only. `Raster` (up to 512×256 cells) and `Image` (PNG/RGBA up to 2 MiB, up to 255×255 cells): terminal only. If a tree uses an element or prop the surface lacks, Claude Code draws its own version of that site instead. — [Reference, Elements](https://code.claude.com/docs/en/plugins/mods/reference); [Draw in the interface](https://code.claude.com/docs/en/plugins/mods/interface)
- **Diffs are supported**. `Code({ format: 'diff', source: '<unified hunks>' })` draws line numbers, red/green shading and word-level highlights. — [Interface gallery, "Code as a diff"](https://code.claude.com/docs/en/plugins/mods/gallery)
- Built-in `/diff` is itself a mod (`cc-plugin-diff`, terminal sessions). Its source is public: "the `/diff` pane, with buttons bound to keyboard actions and scrolling the mod handles itself". — [Mods overview, built-in mods](https://code.claude.com/docs/en/plugins/mods); [anthropics/claude-code/mods/diff](https://github.com/anthropics/claude-code/tree/main/mods/diff)
- `Client` is "A region drawn by a second file of yours, for animation and pointer input. That file gets no mods API". It draws with the same element table (minus `Client`, `Raster`, `Image`) and is limited to 20,000 nodes and 32 levels deep. — [Draw in the interface](https://code.claude.com/docs/en/plugins/mods/interface); [claude-code.d.ts](https://github.com/anthropics/claude-code/blob/main/mods/types/claude-code.d.ts)
- `Image` shows real pixels only in kitty 0.28 or later and in Ghostty. It shows alt text in tmux/screen, in other terminals, and in background sessions. — [Interface gallery](https://code.claude.com/docs/en/plugins/mods/gallery)
- `Link` in the Desktop app is clickable only for `https:` or `http://localhost` URLs. — [Draw in the interface](https://code.claude.com/docs/en/plugins/mods/interface)

**Browser / webview**
- No element accepts HTML, a URL to render, or a webview/iframe. I found none of those words anywhere in the elements reference or the 517 KB type file. (I grepped the type file: no "webview" or "iframe".) — [Reference, Elements](https://code.claude.com/docs/en/plugins/mods/reference); [claude-code.d.ts](https://github.com/anthropics/claude-code/blob/main/mods/types/claude-code.d.ts)
- The types document `$.ui.blit`, which swaps a keyed `Image`'s source at up to 120 a second, and give this example: `$.ui.blit({ requestId: 'browser', key: 'view', source: { shm: name, format: 'rgb', width, height } })`. `Image` also accepts a POSIX shared-memory source (`{ shm, format, width, height }`, up to 4096×4096 px). The `Client` pointer events carry sub-cell `fine` coordinates, described as "What a module showing another program's pixels forwards." Together these imply Anthropic intends a terminal pane that streams pixels from an external browser process. — [claude-code.d.ts](https://github.com/anthropics/claude-code/blob/main/mods/types/claude-code.d.ts) (the `blit`, `ImageProps`/`ImageSource` and `ClientPointerEvent` docs)
- The Desktop app already has a **native** tabbed Browser pane: "a tabbed browser, so you can open documentation, issue trackers, or any other site next to your running app". — [Desktop docs](https://code.claude.com/docs/en/desktop)

**Sessions and subagents**
- `$.agent.spawn` starts a background subagent and returns `{ model, agentId }`. Its answer arrives as its `turn.complete`. `$.agent.list()` returns this session's subagents and teammates with a status (`pending`, `running`, `waiting`, `idle`, `completed`, `failed`, `killed`). `$.agent.register` defines new agent types. — [Reference, Mods API methods](https://code.claude.com/docs/en/plugins/mods/reference); [claude-code.d.ts](https://github.com/anthropics/claude-code/blob/main/mods/types/claude-code.d.ts)
- Event payloads carry an `agentId` (for example `tool.call`, `tool.check`, `turn.complete`, plus `parentAgentId` on spawn). A mod can therefore attribute each tool call and turn to its subagent and build a per-agent view. — [Reference, Events](https://code.claude.com/docs/en/plugins/mods/reference); [claude-code.d.ts](https://github.com/anthropics/claude-code/blob/main/mods/types/claude-code.d.ts)
- `$.session.send({ to })` delivers plain text to another of your sessions (`{ sessionId }`), to a subagent or teammate (`{ agentId }`), or back to a sender. `session.receive` and `session.send` events observe the traffic. — [Use the mods API](https://code.claude.com/docs/en/plugins/mods/api)
- `$.session.messages()` returns **this** session's transcript only, as `{ role, text, toolUses }`, newest 4,096 entries. I found no API that returns another session's or a subagent's message list. — [Use the mods API](https://code.claude.com/docs/en/plugins/mods/api); [Reference, Limits](https://code.claude.com/docs/en/plugins/mods/reference)
- `$.fs.read` (4 MiB per file) and `$.process.run`/`spawn` are available. A mod could read a session's JSONL transcript file from disk itself (the type docs refer to `transcript_path` and say the session id is "the transcript file's name"). — [Use the mods API](https://code.claude.com/docs/en/plugins/mods/api); [claude-code.d.ts](https://github.com/anthropics/claude-code/blob/main/mods/types/claude-code.d.ts)
- `$.process.spawn` "streams a long-running command's output". I found no PTY or terminal-emulator element. — [Use the mods API](https://code.claude.com/docs/en/plugins/mods/api)
- The hooks module has "no Node.js APIs, no timer globals… and no network or file access of its own". Everything goes through `$`, and each `$` call is itself an interceptable event. — [Use the mods API](https://code.claude.com/docs/en/plugins/mods/api)
- Redraws are throttled to 10 a second (30 for the visible terminal pane). Hooks get 10 s each. One tree draws at most 100,000 characters. — [Reference, Limits](https://code.claude.com/docs/en/plugins/mods/reference)

### Inferences
- **A "sub-agent tab" with a full interactive session is not possible as a mod.** A mod can list subagents, show their status, last tool call and answer, and send them text through `$.session.send` (an `Input` plus `send` gives a crude chat box). But it cannot embed Claude Code's own transcript or prompt for another agent or session. The engine-drawn rows (`UserMessage`, `ToolUse`…) are drawn only for the session's own transcript. The best approximation is a pane that rebuilds a read-only transcript from events or JSONL as Markdown/Code, plus a send box.
- **Diff tabs are fully feasible** (`Code` with diff format, and the built-in `/diff` mod as a model). **File tabs** are feasible as read-only `Code` views plus a tree (see filetree below). Editing would mean writing your own editor out of `Input`.
- **A browser tab** is only possible as a terminal-only pixel stream. The mod would run a headless browser via `$.process.spawn`, write frames to shm, `blit` them into an `Image`, and forward `Client` pointer events. It works only in kitty or Ghostty. On Desktop the mod would have to fall back to the app's native Browser pane, which a mod cannot drive per the docs.
- Multiple mod panes appear as tabs in one pane slot. That is the nearest thing to "Codex-style tabs" the mod API offers.

### Gaps
- The docs don't say how the Desktop app arranges several mod panes (as tabs, or as separate dockable panes), or whether a mod pane can be dragged or split like native panes. Issue #100716 (below) shows mod panes exist in Desktop and can be resized.
- I couldn't confirm whether a mod can open, focus or drive the Desktop's native Browser, Diff, Files or Subagent panes. The docs are silent and I found no such API in the reference.
- I found no confirmation of whether `$.ui.open` limits how many panes can be open at once.

## Which published mods or plugins add tabs, sub-agent viewers, file browsers, diff viewers or browsers?

### Takeaway
There are no official Anthropic plugins of this kind beyond the built-in `/diff` mod and three playground samples. The `claude-plugins-official` directory has no tab, subagent or browser mods. Within days of launch the community published dozens of mods. The relevant ones are **subagent boards** (agent-flow, agent-board, named-subagents-mod, pulse-progress, akaz01 agent-board), a **file tree** (data-goblin/claude-code-filetree, about 169 stars, the most popular one found), a **git sidebar**, and a **session browser**. I found **no** mod that embeds a browser, gives each subagent an interactive session tab, or offers a multi-tab file/diff workspace.

### Cited Findings
**Official**
- Built-in mods: `cc-plugin-diff` (the `/diff` pane), `agents-md`, `sec-default`, `telemetry`, `plugin-authoring`, and `you-should-know` (a side agent that shows notes above the prompt, off by default). — [Mods overview](https://code.claude.com/docs/en/plugins/mods); [anthropics/claude-code/mods](https://github.com/anthropics/claude-code/tree/main/mods) (directory lists `agents-md`, `diff`, `sec-default`, `telemetry`, `types`)
- The official sample mods are only `token-weather` (context forecast band), `blast-radius` (holds risky commands) and `replay-theater` (`/replay` steps through last turn's edits), "shared as they are, without support". — [anthropics/claude-code-playground/claude-code/mods](https://github.com/anthropics/claude-code-playground/tree/main/claude-code/mods)
- `anthropics/claude-plugins-official` `plugins/` holds LSPs, code-review, feature-dev, playground, session-report and similar. The only plugin there with a pane-like UI description is `code-modernization` ("The modernization pane and estate map… the fleet view"). I found nothing about tabs, subagents or browsers. — [claude-plugins-official/plugins](https://github.com/anthropics/claude-plugins-official/tree/main/plugins); [code-modernization hooks.json](https://github.com/anthropics/claude-plugins-official/blob/main/plugins/code-modernization/hooks/hooks.json)

**Community: subagent viewers**
- **agent-flow** (in `davila7/claude-code-templates`, installed with `npx claude-code-templates@latest --mod ui/agent-flow`) is a side pane that draws the main loop with its context window, then a tree of subagents. Selecting a row shows `↓ in` (the context handed down), `⚙` (requests, tokens, recent tools) and `↑ out` (the final answer). It does not show a full transcript or an interactive session. — [aitmpl agent-flow](https://www.aitmpl.com/component/mods/ui/agent-flow); [claude-code-templates mods/ui](https://github.com/davila7/claude-code-templates/tree/main/cli-tool/components/mods/ui)
- The same catalog's UI category also has `git-sidebar`, `tool-timing-badge` and `turn-inspector`. Its games category has `doom`, `diff-invaders` and others, which shows that pixel and animation panes work. — [claude-code-templates mods](https://github.com/davila7/claude-code-templates/tree/main/cli-tool/components/mods)
- **tjanuki/claude-mod-agent-board** is a docked pane of subagent cards. Each card shows status, task, type and model, elapsed time, steps and tool calls, the last tool call while running and the first lines of the answer when done. Children nest under their parent. It is built from `agent.spawn`, `tool.call`, `turn.step` and `turn.complete`. — [GitHub](https://github.com/tjanuki/claude-mod-agent-board)
- **Bobby-cell-commits/named-subagents-mod** gives each subagent a themed name and adds a pane listing the running agents. — [GitHub](https://github.com/Bobby-cell-commits/named-subagents-mod)
- **MrValraven/pulse-mods `pulse-progress`** is a progress band plus a `/pulse` panel of subagents ("It only watches"). It is based on savvy-progress. — [GitHub](https://github.com/MrValraven/pulse-mods)
- **akaz01/claude-mods** has an `agent-board` panel (subagents, skills, MCP, background tasks) and `session-collab`, which talks to other Claude Code windows on the same PC. — [GitHub](https://github.com/akaz01/claude-mods)
- **YeonwooSung/my-claude-code-mods** (269 stars, the most-starred mods repo found) has `agent-profiler` (time spent in subagents and elsewhere, plus OTLP export) and `time-machine` (a branching timeline of tool calls with shadow-git snapshots and a file view at any step). — [GitHub](https://github.com/YeonwooSung/my-claude-code-mods)

**Community: files, diffs, sessions**
- **data-goblin/claude-code-filetree** (about 169 stars, v0.2.25) is an IDE-style file tree in the right sidebar. It has search, git status with +/- counts, and highlights for files Claude reads, writes or commits. It runs git/gh operations from the pane. "The Desktop app docks it on its own". In the terminal it needs `/tui fullscreen` and at least 110 columns. Several people have forked it (pulse-filetree, bracyw, DominikSch004, gauravruhela07). — [GitHub](https://github.com/data-goblin/claude-code-filetree)
- **max-gom/claude-session-remover** adds a `/sessions` pane to browse and delete sessions. — [GitHub](https://github.com/max-gom/claude-session-remover)
- Other pane mods: CI checks (colorfield/claude-ci-watch), Lagoon environments (colorfield/claude-lagoon-watch), token meter (Dumorro/token-meter), an idle RPG (treestar84/vibemud), plus many personal marketplaces (EMCC, cockpit, handily). — [GitHub search results, 2026-10-13](https://github.com/search?q=claude+code+mod&type=repositories)

### Inferences
- What the community has built clusters around **observing subagents**: status cards and trees. That matches the API's ceiling. Nobody has shipped interactive per-agent tabs, which supports the conclusion that the API can't host them.
- No mod embeds a browser. The pixel-streaming route in the type docs is so far unused, or at least I found no evidence of it.
- The ecosystem is days old, has almost no stars, and has no curation beyond aitmpl/claude-code-templates and EMCC. Treat every community entry as unvetted. Mods are unsandboxed.

### Gaps
- I couldn't enumerate npm packages for mods. I did not search npm directly.
- I didn't verify any community mod by running it. All descriptions come from READMEs or catalog pages.
- The claudemarketplaces.com and awesome-claude-code lists were not checked, because I ran out of tool-call budget.

## Are there GitHub issues or feature requests on anthropics/claude-code asking for tabs, sub-agent tabs or tabbed panes? What is their status?

### Takeaway
The requests that exist are about **tabbed or multi-project sessions** (CLI #94920, open) and **VS Code session tabs** (#37354 closed; #96382 open, the tabbed sidebar was removed in 2.1.268). The Desktop app already has split view, a tabbed browser and a subagent pane, and several open bugs show **mod panes and bands only render in the primary split pane**. I found no issue asking specifically for "a tab per subagent with a full interactive session".

### Cited Findings
- #94920 "Feature request: Tabbed/multi-project sessions with a project sidebar" is OPEN (2026-09-17), labels `enhancement` and `area:tui`, 0 comments. It asks for "tab-like sessions, each bound to its own project directory". — [#94920](https://github.com/anthropics/claude-code/issues/94920)
- #96382 is OPEN (2026-09-23): in the VS Code extension "since 2.1.268 there is no way to see, switch between, or close ACTIVE sessions… (tabbed sidebar removed with no opt-out)". #99432 is OPEN (2026-10-04), asking for a setting to not register the single-session sidebar view. — [#96382](https://github.com/anthropics/claude-code/issues/96382); [#99432](https://github.com/anthropics/claude-code/issues/99432)
- #37354 "Tabbed conversations in VS Code extension panel" is CLOSED (2026-03-22). — [#37354](https://github.com/anthropics/claude-code/issues/37354)
- Mod panes in Desktop: #100716 "Mods pane cannot be resized when Desktop window is maximized" (OPEN, 2026-10-09). #99868 "Desktop split view: plugin AbovePrompt band renders only in the leftmost pane" (OPEN). #101748 "Desktop split view: plugin AbovePrompt band and side pane only render in the primary pane" (OPEN, 2026-10-13). — [#100716](https://github.com/anthropics/claude-code/issues/100716); [#99868](https://github.com/anthropics/claude-code/issues/99868); [#101748](https://github.com/anthropics/claude-code/issues/101748)
- Desktop split view: #100080 (OPEN): "+ New", Ctrl+N, deep links and pop-out dock all overwrite the leftmost pane, and there's "no way to open a new session in a new pane". — [#100080](https://github.com/anthropics/claude-code/issues/100080)
- The CLI already has a native **subagent view**. Its bugs and requests: #97857 (OPEN, `/compact` in the subagent view compacts main), #83512 (OPEN, statusLine always reports main "even when a subagent or teammate transcript is focused"), #96895 (OPEN, expose the focused subagent view in the statusLine payload), #81131 (CLOSED, Esc in the subagent view interrupted the agent). — [#97857](https://github.com/anthropics/claude-code/issues/97857); [#83512](https://github.com/anthropics/claude-code/issues/83512); [#96895](https://github.com/anthropics/claude-code/issues/96895); [#81131](https://github.com/anthropics/claude-code/issues/81131)
- #88363 is CLOSED (2026-08-20): Desktop should reuse the open Browser pane for links. — [#88363](https://github.com/anthropics/claude-code/issues/88363)
- Desktop native panes: chat, diff, browser (tabbed), terminal (multiple tabs), file editor, plan, tasks, subagent, and iOS Simulator. Panes can be dragged, resized and popped out. "Click any entry [in the tasks pane] to see its output in the subagent pane". — [Desktop docs](https://code.claude.com/docs/en/desktop)

### Inferences
- Native Desktop panes already cover the browser, diff, files and subagent output. The gap versus Codex-style tabs is mainly a **full interactive session per subagent tab**, and the CLI's lack of session tabs.
- Mod UI in Desktop split view is still immature: several bugs filed in the last week.

### Gaps
- `gh search` returns at most a handful of results per query, so older or differently worded requests (for example "workspace tabs") may be missing.
- I couldn't tell whether the native Desktop subagent pane lets you type into the subagent. The docs only say "see its output … or stop it".

## Are there community hacks (status lines, hooks plus external windows) that approximate sub-agent tabs?

### Takeaway
Before mods, the main approximation was **agent-team split panes** (tmux, iTerm, Ghostty), plus external dashboards such as amux. These give each teammate a real interactive terminal session, which is closer to "an interactive tab per agent" than any mod. Since mods, pane-based subagent boards are the in-app approximation, but they are view-only.

### Cited Findings
- Agent teams already run teammates in split panes. Related issues: #24384 (OPEN) asks for a Windows Terminal split-pane backend for `teammateMode`. #35351 (CLOSED) asked for a Ghostty split-pane backend. #71799 (CLOSED) asked for a configurable tmux teammate layout. #24771 (CLOSED) reported tmux panes disconnected from messaging. — [#24384](https://github.com/anthropics/claude-code/issues/24384); [#35351](https://github.com/anthropics/claude-code/issues/35351); [#71799](https://github.com/anthropics/claude-code/issues/71799); [#24771](https://github.com/anthropics/claude-code/issues/24771)
- **mixpeek/amux** (about 533 stars) is an "Open-source control plane for AI coding agents… parallel Claude Code, Codex, and Gemini workers with a shared board… One dashboard, or your phone". **theyashgupta/dispatch** is a local kanban board that turns tickets into live Claude Code sessions in worktrees. — [amux](https://github.com/mixpeek/amux); [dispatch](https://github.com/theyashgupta/dispatch)
- akaz01's `session-collab` mod uses `$.session.send` to talk to other Claude Code windows on the same machine. That is cross-session messaging, not an embedded view. — [akaz01/claude-mods](https://github.com/akaz01/claude-mods)
- Anthropic's own blog compares subagents to browser tabs: a place to chase a tangent without losing the main thread. — [How and when to use subagents](https://claude.com/blog/how-and-when-to-use-subagents-in-claude-code)
- Launch coverage: mods were announced on 2026-10-01 and grew out of a 2026-09-03 "function hooks" proposal. Community examples appeared within hours. These are secondary sources. — [The New Stack](https://thenewstack.io/anthropic-claude-code-mods-plugins/); [pasqualepillitteri.it](https://pasqualepillitteri.it/en/news/19871/claude-code-mods-plugins)

### Inferences
- For "each sub-agent has a full interactive session in a tab", the working patterns are outside the mod API. Options are agent teams in terminal multiplexer splits, separate sessions opened in Desktop split view, or external orchestrators (amux, dispatch). Mods add in-app observation and messaging on top.

### Gaps
- I didn't find specific Reddit, Hacker News or X threads that discuss tabbed sub-agent hacks. A search only turned up a July 2026 Reddit post about running Claude Code inside Chrome as an extension, which is unrelated to mods.
- Status-line-based subagent hacks: I found only the issues above about the statusLine ignoring the focused subagent view, and no community project.
