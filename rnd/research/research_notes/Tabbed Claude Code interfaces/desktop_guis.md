# Standalone third-party desktop/web GUIs for Claude Code with tabbed interfaces

Research date: 2026-10-13. Repository metadata (archived flag, last push, licence, latest release) was read from the GitHub REST API (`gh api repos/<owner>/<repo>` and `/releases/latest`) on that date; the repo URL is given as the source. Feature claims come from each project's README or site unless marked otherwise. "Confirmed" means stated by a primary source (the project's own README, docs or site, or the GitHub API); "unconfirmed" means a secondary source only, or an inference from code search.

Method note: Exa and Firecrawl were out of credits during this session, so web discovery used the built-in WebSearch/WebFetch plus the GitHub API. Several comparison articles found are written by vendors of competing tools (Nimbalyst, Munder Difflin) and disclose it; they were used only for leads and cross-checks.

## Which apps exist and are still maintained as of Oct 2026?

### Takeaway
About a dozen third-party GUIs wrap Claude Code. As of 13 Oct 2026, the ones that are actively released are Nimbalyst, Superset, Emdash, CloudCLI (claudecodeui), Sculptor, Happy, CodePilot, Conductor (closed source) and Agentastic (closed development). opcode (ex-Claudia) is effectively dormant. Crystal has been replaced by Nimbalyst, 1Code is archived, Vibe Kanban is sunsetting into a community edition, and Omnara has pivoted away from being a Claude Code GUI.

### Cited Findings

Catalogue (dates from the GitHub API on 2026-10-13 unless noted):

| App | URL | Status / latest release | Licence / pricing | Platforms |
|---|---|---|---|---|
| **Nimbalyst** (successor to Crystal) | https://github.com/nimbalyst/nimbalyst | Active. Latest GitHub release v0.80.6 (2026-10-09); a "Release v0.81.1" commit landed 2026-10-12. Weekly releases claimed. | MIT; free for individuals | macOS, Windows, Linux desktop; iOS and Android companion apps |
| **Crystal** (stravu) | https://github.com/stravu/crystal | **Deprecated**: the repo description says "Crystal is now Nimbalyst". Last release v0.3.5 (2026-02-26). Not archived. | MIT | Desktop (Electron) |
| **opcode** (formerly Claudia, winfunc / Asterisk) | https://github.com/winfunc/opcode | **Dormant**: last release v0.2.0 (2025-08-31), last code commit 2025-10-16; the only 2026 commit (2026-09-18) is a README image removal. Not archived. About 22.4k stars. | AGPL-3.0; free | Desktop (Tauri) |
| **Conductor** (Melty Labs) | https://www.conductor.build | Active (commercial product, closed source) | Free plan for local workspaces; paid plans for cloud and Teams | macOS only (Apple Silicon per Nimbalyst's comparison); iOS "coming soon" per Munder Difflin |
| **Sculptor** (Imbue) | https://github.com/imbue-ai/sculptor | Active. sculptor-v0.48.0 (2026-09-21); pushed 2026-10-13. | MIT | Desktop |
| **Vibe Kanban** (Bloop) | https://github.com/BloopAI/vibe-kanban | **Sunsetting**, announced 2026-04-10. Remote services were kept for 30 days, then the project went fully local and community-maintained. Latest release v0.1.44 (2026-04-24); commits continue (v0.1.45 bump on 2026-09-19). | Apache-2.0 | Runs via `npx`; web UI |
| **CloudCLI** (aka Claude Code UI, siteboon/claudecodeui) | https://github.com/siteboon/claudecodeui | Active. v1.37.4 (2026-10-09); pushed 2026-10-12. About 14k stars. | AGPL-3.0, self-hosted free; CloudCLI Cloud hosted from €7/month | Web or mobile browser; a desktop app also exists |
| **CodePilot** (op7418) | https://github.com/op7418/CodePilot | Active. v0.67.17 (2026-09-22). | **Business Source License 1.1** (source-available, not OSI) | macOS (signed, arm64/x64), Windows x64, Linux arm64/x64; Electron + Next.js |
| **Happy** (slopus) | https://github.com/slopus/happy | Active. cli-1.2.5 (2026-09-22); pushed 2026-10-13. About 24k stars. | MIT | Mobile and web client plus a CLI wrapper; continue a terminal session from the phone |
| **Omnara** | https://github.com/omnara-ai/omnara | Active repo, but **pivoted**: now described as "The open-source managed agent platform. A self-hostable alternative to Claude Managed Agents and OpenAI's Agents API" (cluster-v0.2.2, 2026-10-10). | Apache-2.0 | Self-hosted platform |
| **Superset** | https://github.com/superset-sh/superset | Active. desktop-v1.37.0 (2026-10-08); pushed 2026-10-13. About 15k stars. | GitHub reports licence "NOASSERTION" (no standard LICENSE detected), so the licence is unconfirmed | Desktop; iPhone remote-control app |
| **Emdash** (General Action, YC W26) | https://github.com/generalaction/emdash | Active. v1.2.8 (2026-10-10). | Apache-2.0 | macOS, Windows, Linux |
| **1Code** (21st.dev) | https://github.com/21st-dev/1code | **Archived**: GitHub `archived: true`; last release v0.0.84 (2026-03-06). | Apache-2.0; Pro/Max subscription for automations | Mac desktop and web; cloud sandboxes |
| **Agentastic.dev** | https://www.agentastic.dev | Active. The site says "still in closed development"; no public repo found. | Free without limits, no sign-up; licence terms not stated on the page | macOS 14+ (Apple Silicon and Intel); iPhone app in TestFlight; CLI on macOS/Linux/WSL |
| **Munder Difflin** (lead, not deeply checked) | https://munderdiffl.in | Active per its own blog (June 2026, facts rechecked 2026-09-10) | MIT free app; Pro and Teams plans | macOS, Windows, Linux |

Supporting citations:
- Nimbalyst repo: MIT, release v0.80.6 on 2026-10-09, latest commit 2026-10-12 — [GitHub](https://github.com/nimbalyst/nimbalyst). It replaced Crystal, which was deprecated in Feb 2026 — [DEV/Nimbalyst blog](https://dev.to/stravukarl/best-tools-for-managing-parallel-ai-coding-agents-in-2026-14l8); confirmed by Crystal's repo description — [GitHub stravu/crystal](https://github.com/stravu/crystal).
- Nimbalyst's comparison says Opcode "has had no commits since October 2025" — [Nimbalyst blog, updated 2026-09-15](https://nimbalyst.com/blog/best-claude-code-gui-tools-2026/). This is consistent with the GitHub commit log; the only later commit is a README edit — [GitHub winfunc/opcode](https://github.com/winfunc/opcode).
- The Vibe Kanban README headline is "Vibe Kanban is sunsetting." — [GitHub](https://github.com/BloopAI/vibe-kanban). The announcement is dated April 10, 2026: remote services stay 30 days, then the project goes fully local and "open source and community maintained" — [vibekanban.com/blog/shutdown](https://www.vibekanban.com/blog/shutdown).
- CloudCLI: "All options use your own AI subscriptions"; Cloud from €7/month — [GitHub README](https://github.com/siteboon/claudecodeui).
- CodePilot's LICENSE file is "Business Source License 1.1, Licensor: op7418" — [GitHub LICENSE](https://github.com/op7418/CodePilot/blob/main/LICENSE). Its CI builds signed and notarized macOS, Windows x64 and Linux packages — [GitHub README](https://github.com/op7418/CodePilot).
- Conductor: Mac app; runs Claude Code, Codex, Cursor and OpenCode in parallel, each in its own git worktree; free, bring your own Claude or Codex subscription; not open source — [Nimbalyst comparison](https://nimbalyst.com/blog/best-claude-code-gui-tools-2026/); free local plan, paid cloud/Teams — [Munder Difflin](https://munderdiffl.in/blog/best-claude-code-multi-agent-tools/); Mac referenced in get-started — [Conductor docs](https://www.conductor.build/docs/).
- Agentastic: macOS 14+, free without limits, "still in closed development", own subscriptions or API keys — [agentastic.dev](https://www.agentastic.dev).
- Anthropic's own Claude Code Desktop is described as single-session in the Nimbalyst comparison, with Agent View (multi-session) in research preview in the CLI — [Nimbalyst blog](https://nimbalyst.com/blog/best-claude-code-gui-tools-2026/). This is a vendor claim and is unconfirmed against Anthropic docs. It is out of scope (first-party) but is the baseline.
- Other leads seen but not investigated: Claude Squad (TUI, AGPL-3.0, tmux) — [Munder Difflin](https://munderdiffl.in/blog/best-claude-code-multi-agent-tools/); Agent Bar (menu bar) — [feedbagel](https://feedbagel.com/post/agent-bar-menu-bar-interface-for-claude-code-ai-projects).

### Inferences
- The most actively released, Codex-desktop-like candidates are Nimbalyst, Superset, Emdash, Conductor and Agentastic: each gives a workspace per task (worktree) with a diff viewer and a terminal, and most add an in-app browser. CloudCLI is the main web-first option.
- The two best-known early projects, opcode/Claudia and Crystal, should both be treated as superseded or dormant despite their star counts.

### Gaps
- Conductor's exact pricing tiers, licence and Apple-Silicon-only claim were not confirmed on its own pricing page.
- Superset's licence: GitHub's API returned NOASSERTION and no root LICENSE file was found.
- Agentastic's licence page was not read.
- Cline-likes (Cline, Roo, Kilo) are VS Code extensions with their own agent loop, not Claude Code wrappers, so they were not catalogued. This was not verified per product.

## Which of them show sub-agents (Task tool) as their own tabs or panes, with an interactive transcript?

### Takeaway
No third-party GUI was confirmed to open each Task-tool sub-agent in its own tab with a full, interactive session transcript. The closest are Nimbalyst, whose Agent Teams "teammates" appear in a TeammatePanel sidebar and can receive messages, and 1Code (now archived), which showed a visual sub-agent task list in a sidebar. Several apps parse the SDK's `parent_tool_use_id` field, which is what lets a client attribute messages to a sub-agent, but that does not prove a dedicated tab exists.

### Cited Findings
- Nimbalyst: "The teammate system allows Claude Code's lead agent to spawn sub-agents ('teammates') that run concurrently, communicate via messages, and appear in the Nimbalyst UI". Each teammate is its own Agent SDK `query()` call. Messages are injected with `streamInput()`. The UI is a `TeammatePanel (sidebar display)` — [Nimbalyst docs/TEAMMATE_IMPLEMENTATION.md](https://github.com/nimbalyst/nimbalyst/blob/main/docs/TEAMMATE_IMPLEMENTATION.md). This is Agent Teams, not plain Task-tool sub-agents.
- 1Code: "Custom Sub-agents - Visual task display in sidebar" and "Sub-agents - Visual task list for sub-agents in the details sidebar" — [GitHub README](https://github.com/21st-dev/1code). The repo is archived.
- Agentastic offers "Manager Agents", which start and steer worker agents over days or weeks. The site does not use the term sub-agent — [agentastic.dev](https://www.agentastic.dev).
- GitHub code search for `parent_tool_use_id` (the SDK's sub-agent attribution field) finds hits in Nimbalyst (22, including SessionManager.ts), Sculptor (21, state/messages), Happy (19, including docs/session-protocol-claude.md), Superset (10, including chat-runtime claudeAdapter.ts), CloudCLI (6, including claude-runtime.provider.js and docs/architecture/06-tool-view.md), CodePilot (4, claude-client.ts) and Crystal (3, ClaudeMessageTransformer.ts). It finds 0 in opcode and Emdash — [GitHub code search via API, 2026-10-13](https://github.com/search?q=parent_tool_use_id&type=code).
- Platform background: Task-tool sub-agent transcripts are stored as separate JSONL files under a per-session subagents folder, and in claude.ai/code and the desktop sidebar they appear as separate sessions (reported as a bug) — [claudeissues #80476](https://claudeissues.com/issue/80476-bug-task-tool-subagent-transcripts-are-mirrored-to-the-claude-ai-code-session-li). There is an open request for a sub-agent transparency UI — [claudeissues #5974](https://claudeissues.com/issue/5974-feature-request-ui-for-subagent-execution-transparency-and-debugging). In Claude Desktop, sub-agent detail buttons for Agent Teams are sometimes unresponsive — [claudeissues #61263](https://claudeissues.com/issue/61263-bug-agent-teams-sub-agents-persist-in-ui-after-completion-sub-agent-detail-butto).
- Non-GUI tools that give per-sub-agent views: Agent Monitor, a Claude Code skill with one tab per sub-agent in a terminal UI that spawns when the Task tool dispatches — [Smithery](https://smithery.ai/skills/victorstein/agent-monitor); June, a TUI sub-agent transcript viewer grouped by branch — [GitHub sky-xo/june](https://github.com/sky-xo/june).
- Munder Difflin's survey does not describe sub-agent display for any tool. Nimbalyst's comparison lists Conductor as "Parallel agents, not subagents" — [Munder Difflin](https://munderdiffl.in/blog/best-claude-code-multi-agent-tools/); [Nimbalyst blog](https://nimbalyst.com/blog/best-claude-code-gui-tools-2026/).

### Inferences
- "Sub-agent as a tab with a full, interactive session" looks like an unfilled niche among third-party GUIs. Most tools treat parallelism as several top-level sessions or worktrees, not as a parent/child hierarchy.
- Task-tool sub-agents are not interactive in Claude Code itself: they return only a summary to the parent. A tab could therefore show a live read-only transcript (from `parent_tool_use_id` stream messages or the sidecar JSONL). Interactivity would need the Agent Teams / teammate model, as in Nimbalyst's `streamInput()` approach.

### Gaps
- No screenshots or docs were checked to see how CloudCLI, Superset, Sculptor, Happy or CodePilot render sub-agent messages: nested, collapsible or separate pane.
- Conductor's and Agentastic's sub-agent handling is not documented on the pages read.

## Which have built-in browser, file explorer/editor, and diff viewer tabs?

### Takeaway
Diff viewers and terminals are near-universal. Built-in browsers are confirmed in Superset, Vibe Kanban, Emdash, 1Code, Agentastic and CloudCLI (agent browser use). File explorers or editors are confirmed in CloudCLI, CodePilot (preview), Agentastic (native editor) and Nimbalyst (visual editors plus a code editor). Superset and CloudCLI explicitly use tabs for terminals and plugins.

### Cited Findings

Feature matrix. Y = confirmed by the project's own README or site. ? = not stated in sources read. Sessions = several concurrent agent sessions or workspaces.

| App | Sessions (multi) | Sub-agent view | Browser | Files | Diff | Terminal | Source |
|---|---|---|---|---|---|---|---|
| Nimbalyst | Y (kanban of sessions, 6+ parallel) | Teammate sidebar panel | ? | Y (7+ visual editors incl. code editor, markdown, Excalidraw, mockups, spreadsheets) | Y (red/green, per-change accept/reject) | Y (embedded Ghostty terminal) | [README](https://github.com/nimbalyst/nimbalyst), [blog](https://nimbalyst.com/blog/best-claude-code-gui-tools-2026/) |
| Superset | Y (workspace per worktree) | ? | Y (browser pane, per-workspace port detection, "Design" element picker) | ? (@-file mentions) | Y (built-in diff viewer, PR review) | Y ("Tabs, infinite splits, presets, and persistent sessions") | [README](https://github.com/superset-sh/superset) |
| Emdash | Y | ? | Y (app preview, per Munder Difflin) | ? | Y ("Review diffs, create pull requests, inspect CI checks") | ? | [README](https://github.com/generalaction/emdash), [Munder Difflin](https://munderdiffl.in/blog/best-claude-code-multi-agent-tools/) |
| Conductor | Y (workspace per task) | No ("parallel agents, not subagents") | ? | ? | Y (diff viewer reference page) | Y (per workspace) | [docs](https://www.conductor.build/docs/), [diff viewer](https://www.conductor.build/docs/reference/diff-viewer) |
| Agentastic | Y (worktrees, containers or VMs) | Manager Agents | Y (agent-drivable browser, Design Mode in Chromium) | Y (native code editor, tree-sitter) | Y (unified/side-by-side, agentic review) | Y (real terminal or structured chat; running-servers panel) | [site](https://www.agentastic.dev) |
| CloudCLI | Y | ? | Y ("Browser Use" sessions for agent-driven tasks) | Y (file tree, syntax highlighting, live editing) | Y (Git integration; "basic diffs" per Nimbalyst) | Y (integrated shell; plugin "Web Terminal" with multi-tab) | [README](https://github.com/siteboon/claudecodeui) |
| CodePilot | Y | ? | ? | Y (file tree with syntax-highlighted preview) | basic (per Nimbalyst) | needs CLI for terminal commands | [README](https://github.com/op7418/CodePilot) |
| Vibe Kanban | Y (workspaces per issue) | ? | Y ("built-in browser with devtools, inspect mode, and device emulation") | ? | Y (inline comments sent to agent) | Y (terminal per workspace) | [README](https://github.com/BloopAI/vibe-kanban) |
| Sculptor | Y (workspace = isolated copy) | ? | ? | ? | Y (review then merge) | Y (built-in, workspace-scoped) | [README](https://github.com/imbue-ai/sculptor) |
| 1Code (archived) | Y | Sidebar task list | Y (live browser previews) | ? | Y (diff previews, built-in Git client) | Y (Cmd+J panel, shared terminals) | [README](https://github.com/21st-dev/1code) |
| opcode (dormant) | Y (interactive sessions; tabs unconfirmed) | ? | n/a ("Visual Project Browser" is a project list, not a web browser) | ? | Y (diff between checkpoints) | ? | [README](https://github.com/winfunc/opcode) |
| Happy | Y (mobile/web) | ? | ? | ? | ? | continues the terminal session | [README](https://github.com/slopus/happy) |

- CloudCLI's plugin system "lets you add custom tabs with their own frontend UI and optional Node.js backend" — [GitHub README](https://github.com/siteboon/claudecodeui).
- Superset's terminal has "Tabs, infinite splits, presets, and persistent sessions that survive restarts" — [GitHub README](https://github.com/superset-sh/superset); see also [docs.superset.sh/browser](https://docs.superset.sh/browser) and [docs.superset.sh/diff-viewer](https://docs.superset.sh/diff-viewer).

### Inferences
- Agentastic (macOS only, closed) and Superset are the closest feature matches to the Codex desktop pattern of session, browser, diff and terminal in one window. Nimbalyst leads on file and document editing and is the only one with a confirmed sub-agent (teammate) panel. CloudCLI is the closest web-based, tab-extensible option.

### Gaps
- Whether these surfaces are literal tabs or split panes or sidebars was not verified with screenshots for most apps; the READMEs mostly say "pane" or "panel".
- File explorer/editor support for Superset, Emdash, Conductor and Sculptor is not stated in the sources read.

## How do they run Claude Code (CLI subprocess, Agent SDK, PTY), and does that affect using a Claude subscription?

### Takeaway
There are two integration styles. Terminal or PTY wrappers (Superset, Conductor, Emdash, Sculptor's "any terminal-based agent", Munder Difflin) run the unmodified `claude` CLI. Structured clients (Nimbalyst, CloudCLI, CodePilot, Happy, Superset's chat runtime) consume the stream through `@anthropic-ai/claude-agent-sdk` or the CLI's stream-JSON. Anthropic's current terms let an end user sign in to the **unmodified Claude Code binary** with their own subscription inside a third-party product. They forbid third-party apps from offering Claude.ai login themselves or routing requests through Free/Pro/Max credentials, and they tell Agent SDK product developers to use API keys. As a result, terminal-wrapping GUIs are on the safest footing for subscription use.

### Cited Findings
- Anthropic: "Developers building products or services that interact with Claude's capabilities, including those using the Agent SDK, should use API key authentication... Anthropic does not permit third-party developers to offer Claude.ai login into their own applications, or to route requests through Free, Pro, or Max plan credentials on behalf of their users." The same section says this does not "prevent an end user from signing in to the unmodified Claude Code binary with their own Claude subscription, including where a platform hosts Claude Code". Products may not modify the binary or remove its auth methods. Anthropic "may [enforce] without prior notice" — [Claude Code docs: Legal and compliance](https://code.claude.com/docs/en/legal-and-compliance).
- The same page: "Advertised usage limits for Pro and Max plans assume ordinary, individual usage of Claude Code and the Agent SDK." — [Claude Code docs](https://code.claude.com/docs/en/legal-and-compliance).
- Secondary history: enforcement began blocking Max OAuth in third-party clients around 2026-01-09, the docs were clarified in Feb 2026, and OpenClaw-style automation tools were cut from subscription limits on 2026-04-04 — [WinBuzzer](https://winbuzzer.com/2026/02/19/anthropic-bans-claude-subscription-oauth-in-third-party-apps-xcxwbn/), [GIGAZINE](https://gigazine.net/gsc_news/en/20260220-anthropic-third-party-block), [cybersecuritynews](https://cybersecuritynews.com/claude-ends-openclaw-subscriptions/). One source claims a "programmatic credit" pool for Agent SDK and `claude -p` from 2026-06-15 — [tech-insider.org](https://tech-insider.org/ie/?p=560). This is **unconfirmed** and not mentioned on Anthropic's legal page.
- Per-app integration evidence:
  - Nimbalyst: Agent SDK `query()` per session and teammate — [TEAMMATE_IMPLEMENTATION.md](https://github.com/nimbalyst/nimbalyst/blob/main/docs/TEAMMATE_IMPLEMENTATION.md). It says it works with an existing Claude Code subscription or API key — [Nimbalyst blog](https://nimbalyst.com/blog/best-claude-code-gui-tools-2026/).
  - Superset: "Run Claude Code, Codex, or another CLI agent with terminals"; "Keep your existing agent subscriptions" — [README](https://github.com/superset-sh/superset). It also has a `packages/chat-runtime` Claude adapter that references the SDK (code search hit). Unconfirmed which path the default UI uses.
  - CloudCLI: "CloudCLI provides the environment, not the AI. You bring your own Claude, Cursor, or Codex subscription" — [README](https://github.com/siteboon/claudecodeui). Its package.json references claude-agent-sdk (GitHub code search hit; unconfirmed as the primary path).
  - CodePilot: multi-provider with API keys. "Installing the Claude Code CLI... unlocks additional capabilities like direct file editing, terminal commands" — [README](https://github.com/op7418/CodePilot).
  - Happy: "Reuse your subscriptions. Sign in with the Claude, Codex, and Grok subscriptions you already pay for" — [README](https://github.com/slopus/happy). Code search found `providerAuth.ts` in happy-cli. How it authenticates is not verified, and the policy risk depends on whether it runs the unmodified CLI.
  - Sculptor: runs Claude Code and "any terminal-based" agent — [README](https://github.com/imbue-ai/sculptor).
  - Conductor and Agentastic: bring your own provider subscriptions or API keys — [Nimbalyst blog](https://nimbalyst.com/blog/best-claude-code-gui-tools-2026/), [agentastic.dev](https://www.agentastic.dev). Agentastic notes its optional hosted services may route session traffic through its servers.
  - Munder Difflin "Runs Claude Code as a real process on your machine"; its footer lists xterm.js and node-pty — [Munder Difflin](https://munderdiffl.in/blog/best-claude-code-multi-agent-tools/).
  - 1Code: BYOK custom models and providers — [README](https://github.com/21st-dev/1code).
  - Emdash: package.json references claude-agent-sdk (code search hit). Its README says "Use any provider" — [README](https://github.com/generalaction/emdash).

### Inferences
- PTY or terminal wrapping keeps the GUI inside the "unmodified binary, user's own sign-in" allowance and so preserves subscription use. However, the GUI only sees terminal bytes, so rich surfaces such as per-sub-agent tabs, structured diffs per tool call, or permission dialogs must be reconstructed by parsing or hooks.
- The Agent SDK gives structured events, including `parent_tool_use_id` for sub-agent attribution, and is the natural basis for sub-agent tabs. Anthropic's terms, however, steer SDK-based *products* to API keys. A GUI that uses the SDK with the user's own locally logged-in CLI credentials is a grey zone; the terms forbid the app offering Claude.ai login or intermediating credentials.

### Gaps
- Whether the Agent SDK, used by a locally installed third-party desktop app with the user's own `claude` login, counts as "ordinary, individual usage" or as a product routing subscription credentials. The legal page does not address this case explicitly.
- The per-app auth flow (CLI login reuse, embedded OAuth, or API key only) was not verified from source for Happy, CloudCLI, Superset, Emdash or CodePilot.
- The claimed June 2026 "programmatic credit" split is unverified against any Anthropic source.
