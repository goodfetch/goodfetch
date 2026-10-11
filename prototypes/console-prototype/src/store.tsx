// PROTOTYPE, throwaway: one in-memory store shared by every variant.
import * as React from "react"
import { toast } from "sonner"
import { attentionItems, INITIAL_RUNS, type AttentionItem, type Run } from "@/data"

export const VARIANTS = {
  A: "Workbench: Runs first",
  B: "Inbox: decisions first",
  C: "Library: Recipes first",
  D: "Live: Sessions and events first",
  E: "Side menu: every screen listed",
  F: "Drawers: details over the page",
  G: "Inline and split: competitors' patterns",
} as const
export type VariantKey = keyof typeof VARIANTS

export type Route = "home" | "needs" | "runs" | "run" | "live" | "recipes" | "recipe" | "datasets" | "sessions" | "plans" | "settings"
export type DrawerEntry =
  | { type: "item"; id: string }
  | { type: "run"; id: string }
  | { type: "session"; id: string }
  | { type: "activity" }
  | { type: "create" }
  | { type: "code" }
  | { type: "settings" }

type State = {
  variant: VariantKey
  runs: Run[]
  resolved: Record<string, boolean>
  route: Route
  run: string
  item: string | null
  recipe: string | null
  tile: string
  tab: string
  step: number
  homeView: "table" | "board" | "live"
  filter: string
  aNav: string
  bTab: string
  cTab: string
  dBacklog: boolean
  expanded: Record<string, boolean>
  drawers: DrawerEntry[]
  control: boolean
  nextId: number
}

const initialVariant = (): VariantKey => {
  const v = new URLSearchParams(location.search).get("variant")
  return v && v in VARIANTS ? (v as VariantKey) : "A"
}

const initial: State = {
  variant: initialVariant(),
  runs: INITIAL_RUNS,
  resolved: {},
  route: "home",
  run: "R-2050",
  item: null,
  recipe: null,
  tile: "s-sup-1",
  tab: "timeline",
  step: -1,
  homeView: "table",
  filter: "all",
  aNav: "runs",
  bTab: "needs",
  cTab: "overview",
  dBacklog: false,
  expanded: {},
  drawers: [],
  control: false,
  nextId: 2052,
}

type Ctx = State & {
  set: (patch: Partial<State> | ((s: State) => Partial<State>)) => void
  items: AttentionItem[]
  runById: (id: string) => Run
  setVariant: (v: VariantKey) => void
  go: (r: Route) => void
  openRun: (id: string) => void
  openItem: (id: string) => void
  openSession: (id: string) => void
  pushDrawer: (d: DrawerEntry) => void
  closeDrawers: () => void
  resolve: (runId: string, msg: string) => void
  startRun: () => void
}

const StoreContext = React.createContext<Ctx | null>(null)

export function StoreProvider({ children }: { children: React.ReactNode }) {
  const [s, setS] = React.useState<State>(initial)
  const set: Ctx["set"] = (patch) => setS((p) => ({ ...p, ...(typeof patch === "function" ? patch(p) : patch) }))
  const items = attentionItems(s.runs, s.resolved)
  const runById = (id: string) => s.runs.find((r) => r.id === id)!
  const sessionRun = (sid: string) => s.runs.find((r) => r.sessions.some((x) => x.id === sid))!

  // Which surface a click opens depends on the variant: this is what the variants disagree about.
  const openRun = (id: string) => openRunWith(runById(id))
  const openRunWith = (r: Run) => {
    const id = r.id
    const base = { run: id, tab: "timeline", step: -1 }
    switch (s.variant) {
      case "A": return set({ ...base, aNav: "runs", drawers: [] })
      case "B": return set({ ...base, bTab: "activity", drawers: [] })
      case "C": return set({ ...base, recipe: r.recipe ?? "_adhoc", cTab: "runs", drawers: [] })
      case "D": return set({ ...base, tile: r.sessions[0].id, drawers: [] })
      case "F": return set((p) => ({ ...base, drawers: [...p.drawers.filter((d) => d.type !== "activity" && d.type !== "create"), { type: "run", id }] }))
      default: return set({ ...base, route: "run", drawers: [] })
    }
  }
  const openItem = (id: string) => {
    switch (s.variant) {
      case "B": return set({ bTab: "needs", item: id, drawers: [] })
      case "E": return set({ route: "needs", item: id, drawers: [] })
      case "F": return set((p) => ({ drawers: [...p.drawers, { type: "item", id }] }))
      default: return set({ drawers: [{ type: "item", id }] })
    }
  }
  const openSession = (sid: string) => {
    if (s.variant === "F" || s.variant === "G") return set((p) => ({ drawers: [...p.drawers, { type: "session", id: sid }] }))
    const r = sessionRun(sid)
    openRun(r.id)
    set({ tab: "sessions" })
  }

  const ctx: Ctx = {
    ...s,
    set,
    items,
    runById,
    setVariant: (v) => {
      const u = new URL(location.href)
      u.searchParams.set("variant", v)
      history.replaceState(null, "", u)
      set({ variant: v, route: "home", drawers: [], control: false })
    },
    go: (route) => set({ route, drawers: [], control: false }),
    openRun,
    openItem,
    openSession,
    pushDrawer: (d) => set((p) => ({ drawers: [...p.drawers, d] })),
    closeDrawers: () => set({ drawers: [], control: false }),
    resolve: (runId, msg) => {
      set((p) => ({
        resolved: { ...p.resolved, [runId]: true },
        runs: p.runs.map((r) => (r.id === runId && r.wait ? { ...r, wait: null, sessions: r.sessions.map((x, i) => (i === 0 ? { ...x, state: "active" } : x)) } : r)),
        control: false,
        item: null,
      }))
      toast(msg)
    },
    startRun: () => {
      const id = `R-${s.nextId}`
      const run: Run = {
        id, recipe: "catalog", client: "Console", started: "now", life: "running", verdict: null, comp: null, epoch: 1, records: 0, progress: 5, marks: [],
        sessions: [{ id: `s-new-${s.nextId}`, mode: "standard", proxy: "none", where: "HTTP (no browser)", state: "active" }],
        usage: { requests: 3, browser: "0 s", tokens: "—", proxy: "—", solver: 0, cost: null }, limit: { used: 0, cap: 1 },
        steps: [{ op: "Sample comparison of fetch tiers", cls: "read", st: "run", dur: "running", facts: [["Pages compared", "0 of 5"]], code: "tier: comparing" }],
      }
      set((p) => ({ runs: [run, ...p.runs], nextId: p.nextId + 1, drawers: [] }))
      toast(`${id} started from the Console (simulated)`)
      openRunWith(run)
    },
  }
  return <StoreContext.Provider value={ctx}>{children}</StoreContext.Provider>
}

export function useStore() {
  const c = React.useContext(StoreContext)
  if (!c) throw new Error("useStore outside StoreProvider")
  return c
}
