// PROTOTYPE, throwaway: simulated engine state for the Console prototype (Jbmanllr/automan#80).
// In memory only. No engine, no network.

export type OpClass = "read" | "idempotent write" | "effect"
export type StepState = "ok" | "warn" | "err" | "wait" | "run"
export type Step = {
  op: string
  cls: OpClass
  st: StepState
  dur: string
  facts: [string, string][]
  code: string
}
export type Session = {
  id: string
  mode: "standard" | "identified" | "stealth"
  proxy: string
  where: string
  state: string
}
export type Life = "running" | "stopped" | "finished" | "failed"
export type Run = {
  id: string
  recipe: string | null
  adhoc?: string
  client: string
  started: string
  life: Life
  verdict: "verified" | "needs-review" | "failed" | null
  comp: "complete" | "incomplete" | "unverified" | null
  epoch: number
  records: number
  progress?: number
  marks: string[]
  wait?: { kind: "MFA" | "Confirmation"; expires: string; why: string; effect?: string } | null
  uncertain?: { effect: string; why: string; evidence: string }
  repair?: { from: string; to: string; auto: boolean; why: string }
  stopReason?: string
  agentSaid?: string
  error?: string
  sessions: Session[]
  usage: { requests: number; browser: string; tokens: string; proxy: string; solver: number; cost: string | null }
  limit?: { used: number; cap: number } | null
  steps: Step[]
}
export type Recipe = { id: string; name: string; pillars: string; plan: string; effects: boolean; desc: string }
export type ItemKind =
  | "Human pause · MFA"
  | "Human pause · Confirmation"
  | "Uncertain Effect"
  | "Plan diff review"
  | "Limit reached"
  | "Result needs review"
export type AttentionItem = { id: string; run: string; kind: ItemKind; title: string; sub: string; urgent: boolean }

export const ENGINE = {
  mode: "Daemon",
  addr: "127.0.0.1:7317",
  version: "1.0.0",
  layout: "one file per run",
  durability: "power-cut safe (FULL)",
  idle: "stops after 10 idle minutes unless kept alive",
  clients: [
    { who: "Python SDK", detail: "supplier_export.py · workers=4", since: "09:58" },
    { who: "MCP server", detail: 'Claude Code · profile "browse"', since: "10:05" },
    { who: "CLI", detail: "automan run equipment-catalog", since: "10:20" },
    { who: "Console", detail: "this tab", since: "10:31" },
  ],
}

export const RECIPES: Recipe[] = [
  { id: "catalog", name: "Equipment catalog", pillars: "Crawl + Extraction", plan: "—", effects: false, desc: "References, prices and stock of every product on catalog.example.invalid." },
  { id: "supplier", name: "Supplier export", pillars: "Automation", plan: "v4", effects: true, desc: "Log in to the supplier portal and download the weekly export." },
  { id: "reorder", name: "Reorder consumables", pillars: "Automation", plan: "v3", effects: true, desc: "Place an order for consumables under the stock threshold." },
  { id: "pricing", name: "Pricing pages", pillars: "Extraction", plan: "v8", effects: false, desc: "Prices from every page of pricing.example.invalid." },
  { id: "docs", name: "Docs crawl", pillars: "Crawl", plan: "—", effects: false, desc: "Clean Markdown of docs.example.invalid for an AI index." },
]

const S = (op: string, cls: OpClass, st: StepState, dur: string, facts: [string, string][], code: string): Step => ({ op, cls, st, dur, facts, code })

export const INITIAL_RUNS: Run[] = [
  { id: "R-2051", recipe: "catalog", client: "CLI", started: "10:20", life: "running", verdict: null, comp: null, epoch: 1, records: 812, progress: 63, marks: [],
    sessions: [{ id: "s-cat-1", mode: "standard", proxy: "none", where: "HTTP (no browser)", state: "active" }, { id: "s-cat-2", mode: "standard", proxy: "none", where: "HTTP (no browser)", state: "active" }],
    usage: { requests: 1904, browser: "0 s", tokens: "—", proxy: "—", solver: 0, cost: null },
    steps: [
      S("Fetch tier chosen by sample: HTTP", "read", "ok", "2.1 s", [["Compared", "HTTP vs browser on 5 pages"], ["Mismatch", "0 fields"], ["Choice", "HTTP (compared)"]], "tier: http\nchosen_by: sample_comparison\npages_compared: 5"),
      S("Crawl catalog.example.invalid", "read", "run", "running", [["Visited", "1,904"], ["In frontier", "1,122"], ["Learned rules", "2 (sort=, utm_*), 341 URLs merged"]], "frontier.visited: 1904\nfrontier.pending: 1122\nrule: drop query param \"sort\" (evidence: 40 pairs identical)"),
      S("Extract product records (dom-smoothie + schema)", "read", "run", "running", [["Records", "812"], ["Grounded", "812 / 812"]], "extractor: dom-smoothie\ngrounding: pass"),
    ] },
  { id: "R-2050", recipe: "supplier", client: "Python SDK", started: "09:58", life: "running", verdict: null, comp: null, epoch: 1, records: 0, progress: 40, marks: [],
    wait: { kind: "MFA", expires: "14:32", why: "Push approval sent to the account holder's phone. Push approvals always need a person (ADR 0006)." },
    sessions: [{ id: "s-sup-1", mode: "standard", proxy: "ISP", where: "local Chromium", state: "waiting: MFA" }],
    usage: { requests: 42, browser: "3 min 10 s", tokens: "—", proxy: "ISP 4.1 MB", solver: 0, cost: null },
    steps: [
      S("Open portal.supplier.example.invalid", "read", "ok", "1.2 s", [["Page check", "login page detected"]], "page_check: login_page"),
      S("Recovery 1: declared refresh", "idempotent write", "warn", "0.8 s", [["Result", "still a login page"]], "recovery: refresh\nresult: login_page"),
      S("Recovery 2: scripted login with %SUPPLIER_PASSWORD%", "effect", "ok", "4.0 s", [["Secret reference", "keychain:automan/supplier"], ["Typed by", "engine (never the model)"]], "secret_ref: keychain:automan/supplier\nvalue: ••••••• (never shown)\nfield_recheck: pass"),
      S("MFA: push approval", "effect", "wait", "waiting", [["Kind", "push approval"], ["Only this Session waits", "yes"], ["Browser held", "15 min, then saved and released"]], "human_pause: mfa\nsession: s-sup-1\ntimeout: E_HUMAN_TIMEOUT at 10:12"),
    ] },
  { id: "R-2049", recipe: "reorder", client: "MCP · Claude Code", started: "10:05", life: "running", verdict: null, comp: null, epoch: 1, records: 0, progress: 80, marks: [],
    wait: { kind: "Confirmation", expires: "28:10", why: "An AI-decided Effect in the payment category. The run does not allow payments without confirmation (ADR 0014).", effect: "Place order: 6 items, €184.20, card ending 4242 (stored on the site)" },
    sessions: [{ id: "s-ord-1", mode: "standard", proxy: "none", where: "local Chromium", state: "waiting: confirmation" }],
    usage: { requests: 88, browser: "2 min 02 s", tokens: "claude-sonnet-5-5: 41k in / 3k out", proxy: "—", solver: 0, cost: "$0.17 est." }, limit: { used: 0.17, cap: 1 },
    steps: [
      S("Plan v3 step 1: open cart (fingerprint match)", "read", "ok", "0.9 s", [["Target fingerprint", "match"]], "plan: reorder@v3\nstep: 1\nfingerprint: match"),
      S("Agent: add 6 items under threshold", "effect", "ok", "38 s", [["Effects", "6 x add-to-cart (journaled)"], ["Model calls", "7"]], "journal: 6 effects committed\nmodel: claude-sonnet-5-5"),
      S("Agent proposes: place order (payment)", "effect", "wait", "waiting", [["Category", "payment"], ["Amount", "€184.20"], ["Decided by", "model"]], "human_pause: confirmation\ncategory: payment"),
    ] },
  { id: "R-2048", recipe: "pricing", client: "Console", started: "09:40", life: "finished", verdict: "verified", comp: "complete", epoch: 1, records: 244, marks: [],
    repair: { from: "v7", to: "v8", auto: true, why: "Read-only Plan: promoted automatically after this verified run (ADR 0009)." },
    sessions: [{ id: "s-pr-1", mode: "standard", proxy: "none", where: "local Chromium", state: "released" }],
    usage: { requests: 31, browser: "48 s", tokens: "gpt-5.1-mini: 9k in / 1k out", proxy: "—", solver: 0, cost: "$0.01 est." },
    steps: [
      S("Plan v7 step 3: next page", "read", "warn", "0.4 s", [["Target fingerprint", "Drift: link \"Next\" became a button"], ["Repair", "1 of 1 allowed"]], "drift: step 3\nexpected: a[rel=next] \"Next\"\nfound: button \"Next page\""),
      S("Repair: recompile step 3, whole-plan check", "read", "ok", "6.1 s", [["Check", "pass on all 8 steps"], ["Used in run", "yes"]], "repair: v7 -> v8 (candidate)\ncheck: pass"),
      S("Collect 12 pages", "read", "ok", "40 s", [["Records", "244"], ["Success check", "\"244 prices, one per SKU\" pass"]], "success_check: pass\ncompleteness: complete"),
    ] },
  { id: "R-2047", recipe: "reorder", client: "MCP · Claude Code", started: "yesterday 16:12", life: "finished", verdict: "needs-review", comp: null, epoch: 2, records: 0, marks: [],
    uncertain: { effect: "Submit order (payment, €96.40)", why: "The Daemon crashed after the click and before the confirmation page. The run resumed from its checkpoint (epoch 2) and cannot tell whether the order went through.", evidence: "Last screenshot: spinner on \"Placing your order…\". No order e-mail seen yet." },
    sessions: [{ id: "s-ord-0", mode: "standard", proxy: "none", where: "local Chromium", state: "released" }],
    usage: { requests: 64, browser: "1 min 40 s", tokens: "claude-sonnet-5-5: 30k in / 2k out", proxy: "—", solver: 0, cost: "$0.12 est." },
    steps: [S("Submit order", "effect", "err", "—", [["Effect", "uncertain"], ["Epoch", "1 → 2 (resumed)"]], "effect: submit_order\nstate: uncertain\nreason: process lost between act and check")] },
  { id: "R-2046", recipe: "docs", client: "Python SDK", started: "08:02", life: "stopped", verdict: null, comp: "incomplete", epoch: 1, records: 3120, marks: [], stopReason: "Limit reached: $2.00 est. (model calls for Markdown clean-up)",
    sessions: [{ id: "s-doc-1", mode: "standard", proxy: "none", where: "HTTP (no browser)", state: "released" }],
    usage: { requests: 3290, browser: "0 s", tokens: "gpt-5.1-mini: 1.9M in / 410k out", proxy: "—", solver: 0, cost: "$2.00 est." }, limit: { used: 2, cap: 2 },
    steps: [
      S("Crawl docs.example.invalid", "read", "ok", "41 min", [["Visited", "3,290"], ["Pending", "1,804"]], "frontier.pending: 1804"),
      S("LLM Markdown clean-up (opt-in)", "read", "warn", "stopped", [["Limit", "$2.00 est. reached"], ["Next call refused", "worst case $0.004"]], "limit: currency 2.00 USD (price table, overridable)\nstate: stopped, resumable"),
    ] },
  { id: "R-2045", recipe: null, adhoc: "Warranty terms for 12 products", client: "MCP · Claude Code", started: "07:50", life: "finished", verdict: "needs-review", comp: "unverified", epoch: 1, records: 12, marks: ["needs-review"], agentSaid: "\"Done: found warranty terms for all 12 products.\"",
    sessions: [{ id: "s-ag-1", mode: "standard", proxy: "none", where: "local Chromium", state: "released" }],
    usage: { requests: 140, browser: "6 min", tokens: "claude-sonnet-5-5: 210k in / 9k out", proxy: "—", solver: 0, cost: "$0.71 est." },
    steps: [S("Agent task (no Success check declared)", "read", "warn", "6 min", [["Agent message", "\"Done\""], ["Success check", "none declared"], ["Grounding", "11 / 12 values found in their page"]], "verdict: needs-review\nreason: no success check; a model's \"done\" is a signal, not proof")] },
  { id: "R-2044", recipe: "supplier", client: "Python SDK", started: "Mon 09:58", life: "finished", verdict: "verified", comp: "complete", epoch: 1, records: 1, marks: [],
    repair: { from: "v4", to: "v5", auto: false, why: "Plan has Effects (\"Request export\"): the new version waits for a reviewed diff (ADR 0009)." },
    sessions: [{ id: "s-sup-0", mode: "standard", proxy: "ISP", where: "local Chromium", state: "released" }],
    usage: { requests: 51, browser: "2 min", tokens: "gpt-5.1-mini: 12k in / 1k out", proxy: "ISP 3.8 MB", solver: 0, cost: "$0.01 est." },
    steps: [
      S("Plan v4 step 4: \"Request export\" (Drift)", "effect", "warn", "0.3 s", [["Drift", "button moved into a menu"]], "drift: step 4"),
      S("Repair checked before the Effect", "effect", "ok", "5.2 s", [["Effect", "performed once, journaled"]], "repair: v4 -> v5 candidate\njournal: request_export committed"),
    ] },
  { id: "R-2042", recipe: "catalog", client: "CLI", started: "Sun 22:00", life: "finished", verdict: "verified", comp: "unverified", epoch: 1, records: 1260, marks: ["forced tier", "unchecked"],
    sessions: [{ id: "s-cat-0", mode: "standard", proxy: "datacenter", where: "HTTP (no browser)", state: "released" }],
    usage: { requests: 2410, browser: "0 s", tokens: "—", proxy: "datacenter 61 MB", solver: 0, cost: null },
    steps: [S("Crawl with forced HTTP tier, Page check off", "read", "ok", "14 min", [["Completeness", "unverified: offset pagination only"], ["Marks", "forced tier, unchecked"]], "completeness: unverified\nmarks: [forced_tier, unchecked]")] },
  { id: "R-2041", recipe: "supplier", client: "Console", started: "Sat 11:00", life: "failed", verdict: "failed", comp: "incomplete", epoch: 1, records: 0, marks: [], error: "E_PAGE_CHECK",
    sessions: [{ id: "s-sup-x", mode: "standard", proxy: "ISP", where: "local Chromium", state: "retired" }],
    usage: { requests: 12, browser: "40 s", tokens: "—", proxy: "ISP 0.9 MB", solver: 0, cost: null },
    steps: [S("Open export page", "read", "err", "2.0 s", [["Page check", "neither expected page nor login page"], ["Error", "E_PAGE_CHECK"]], "error: E_PAGE_CHECK")] },
]

export const EVENTS: [string, string, string, boolean?][] = [
  ["10:31:08", "R-2051", "record #812 committed"],
  ["10:31:07", "R-2049", "human_pause confirmation (payment €184.20)", true],
  ["10:31:02", "R-2051", "frontier rule learned: drop utm_* (12 URLs merged)"],
  ["10:30:55", "R-2049", "effect add_to_cart x6 journaled"],
  ["10:12:40", "R-2050", "human_pause mfa (push)", true],
  ["10:12:31", "R-2050", "secret used keychain:automan/supplier"],
  ["10:05:12", "R-2049", "run started by MCP · Claude Code"],
  ["09:58:03", "R-2050", "run started by Python SDK · workers=4"],
  ["09:41:20", "R-2048", "plan pricing v8 promoted automatically"],
  ["08:43:00", "R-2046", "limit reached $2.00 est. · stopped, resumable", true],
]

export const recipeOf = (id: string | null) => RECIPES.find((r) => r.id === id) ?? null
export const runName = (r: Run) => (r.recipe ? recipeOf(r.recipe)!.name : `${r.adhoc} (no Recipe)`)

export function attentionItems(runs: Run[], resolved: Record<string, boolean>): AttentionItem[] {
  const out: AttentionItem[] = []
  for (const r of runs) {
    if (resolved[r.id]) continue
    if (r.wait)
      out.push(r.wait.kind === "MFA"
        ? { id: `${r.id}:pause`, run: r.id, kind: "Human pause · MFA", title: `Approve the sign-in for ${runName(r)}`, sub: `Only Session ${r.sessions[0].id} waits · expires in ${r.wait.expires}`, urgent: true }
        : { id: `${r.id}:pause`, run: r.id, kind: "Human pause · Confirmation", title: `Confirm a payment: ${runName(r)}`, sub: r.wait.effect ?? "", urgent: true })
    if (r.uncertain) out.push({ id: `${r.id}:uncertain`, run: r.id, kind: "Uncertain Effect", title: `Did the order go through? ${runName(r)}`, sub: r.uncertain.effect, urgent: false })
    if (r.repair && !r.repair.auto) out.push({ id: `${r.id}:diff`, run: r.id, kind: "Plan diff review", title: `Review ${runName(r)} Plan ${r.repair.from} → ${r.repair.to}`, sub: "Plan has Effects; repaired in a verified run", urgent: false })
    if (r.limit && r.life === "stopped") out.push({ id: `${r.id}:limit`, run: r.id, kind: "Limit reached", title: `Raise the Limit and resume ${runName(r)}`, sub: r.stopReason ?? "", urgent: false })
    if (r.verdict === "needs-review" && !r.uncertain) out.push({ id: `${r.id}:review`, run: r.id, kind: "Result needs review", title: `Check the result: ${runName(r)}`, sub: `No Success check declared · agent said ${r.agentSaid}`, urgent: false })
  }
  return out
}

// State is shown as text, never colour alone (product-design.md §4). The badge variant only reinforces it.
export type BadgeVariant = "default" | "secondary" | "destructive" | "outline" | "ghost"
export function lifeLabel(r: Run, resolved: Record<string, boolean>): [string, BadgeVariant] {
  if (r.life === "running") return r.wait ? [`Running · 1 Session waiting: ${r.wait.kind}`, "outline"] : ["Running", "default"]
  if (r.life === "stopped") return ["Stopped by Limit · resumable", "outline"]
  if (r.life === "failed") return [`Failed · ${r.error}`, "destructive"]
  if (r.uncertain && !resolved[r.id]) return ["Finished · 1 Effect uncertain", "destructive"]
  return r.verdict === "verified" ? ["Finished · Verified result", "secondary"] : ["Finished · Needs review", "outline"]
}
