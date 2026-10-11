// PROTOTYPE, throwaway. Screens reached from the side menu (variants E, F and G).
import * as React from "react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Collapsible, CollapsibleContent } from "@/components/ui/collapsible"
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty"
import { ItemGroup } from "@/components/ui/item"
import { Progress } from "@/components/ui/progress"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { Breadcrumb, BreadcrumbItem, BreadcrumbLink, BreadcrumbList, BreadcrumbPage, BreadcrumbSeparator } from "@/components/ui/breadcrumb"
import { EVENTS, RECIPES, recipeOf, runName, type Run } from "@/data"
import { useStore, type Route } from "@/store"
import { AttentionAlerts, ClassBadge, Code, Decision, Facts, H1, ItemRow, LiveView, Muted, RunHeader, RunTabs, say, SettingsBody, StateBadges } from "./parts"

export function Crumbs({ to, label, here }: { to: Route; label: string; here: string }) {
  const { go } = useStore()
  return (
    <Breadcrumb><BreadcrumbList>
      <BreadcrumbItem><BreadcrumbLink render={<button type="button" onClick={() => go(to)} />}>{label}</BreadcrumbLink></BreadcrumbItem>
      <BreadcrumbSeparator /><BreadcrumbItem><BreadcrumbPage>{here}</BreadcrumbPage></BreadcrumbItem>
    </BreadcrumbList></Breadcrumb>
  )
}

export function RunsTable({ runs, extra }: { runs: Run[]; extra?: (r: Run) => React.ReactNode }) {
  const { openRun } = useStore()
  return (
    <Table>
      <TableHeader><TableRow><TableHead>Run</TableHead><TableHead>Started</TableHead><TableHead>State</TableHead>{extra && <TableHead>Needs you</TableHead>}</TableRow></TableHeader>
      <TableBody>
        {runs.map((r) => (
          <TableRow key={r.id}>
            <TableCell><Button variant="link" size="sm" onClick={() => openRun(r.id)}>{runName(r)}</Button><div className="font-mono text-xs text-muted-foreground">{r.id}</div></TableCell>
            <TableCell><div>{r.started}</div><div className="text-xs text-muted-foreground">{r.client}</div></TableCell>
            <TableCell><StateBadges run={r} /></TableCell>
            {extra && <TableCell>{extra(r)}</TableCell>}
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}

function NeedsCell({ r }: { r: Run }) {
  const { items, openItem } = useStore()
  const i = items.find((x) => x.run === r.id)
  if (!i) return <span className="text-xs text-muted-foreground">—</span>
  return <div className="flex flex-col items-start gap-1"><span className="text-xs">{i.kind}</span><Button size="xs" onClick={() => openItem(i.id)}>Open</Button></div>
}

function HomeTable() {
  const { runs, items, filter, set } = useStore()
  const pass = (r: Run) => filter === "all" || (filter === "needs" && items.some((i) => i.run === r.id)) || (filter === "running" && r.life === "running") || (filter === "done" && r.life === "finished") || (filter === "bad" && (r.life === "failed" || r.life === "stopped"))
  return (
    <div className="flex flex-col gap-3">
      <ToggleGroup value={[filter]} onValueChange={(v) => v[0] && set({ filter: String(v[0]) })} variant="outline" size="sm">
        <ToggleGroupItem value="all">All</ToggleGroupItem><ToggleGroupItem value="needs">Needs you</ToggleGroupItem><ToggleGroupItem value="running">Running</ToggleGroupItem><ToggleGroupItem value="done">Finished</ToggleGroupItem><ToggleGroupItem value="bad">Stopped or failed</ToggleGroupItem>
      </ToggleGroup>
      <RunsTable runs={runs.filter(pass)} extra={(r) => <NeedsCell r={r} />} />
    </div>
  )
}

function HomeBoard() {
  const { runs, items, openRun, openItem } = useStore()
  const lane = (r: Run) => (items.some((i) => i.run === r.id) ? 0 : r.life === "running" ? 1 : r.life === "finished" ? 2 : 3)
  return (
    <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
      {["Needs you", "Running", "Finished", "Stopped or failed"].map((l, k) => {
        const rs = runs.filter((r) => lane(r) === k)
        return (
          <div key={l} className="flex flex-col gap-2 rounded-lg bg-muted/50 p-2">
            <span className="px-1 text-xs font-medium">{l} · {rs.length}</span>
            {rs.map((r) => {
              const i = items.find((x) => x.run === r.id)
              return (
                <Card key={r.id} size="sm">
                  <CardHeader><CardTitle>{runName(r)}</CardTitle><CardDescription>{r.id} · {r.client}</CardDescription></CardHeader>
                  <CardContent><div className="flex flex-col gap-2"><StateBadges run={r} />{i && <Muted>{i.kind}: {i.sub}</Muted>}
                    <div className="flex gap-2">{i && <Button size="xs" onClick={() => openItem(i.id)}>Open</Button>}<Button size="xs" variant="outline" onClick={() => openRun(r.id)}>Run</Button></div></div></CardContent>
                </Card>
              )
            })}
          </div>
        )
      })}
    </div>
  )
}

export function HomeScreen() {
  const { runs, items, homeView, set } = useStore()
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <H1>Home</H1>
        <ToggleGroup value={[homeView]} onValueChange={(v) => v[0] && set({ homeView: v[0] as "table" | "board" | "live" })} variant="outline" size="sm" aria-label="View">
          <ToggleGroupItem value="table">Table</ToggleGroupItem><ToggleGroupItem value="board">Board</ToggleGroupItem><ToggleGroupItem value="live">Live</ToggleGroupItem>
        </ToggleGroup>
      </div>
      <Muted>The person picks the view and the Console remembers it. Table and Board show the same {runs.length} runs and {items.length} items that need you; Live shows the running ones with their live views and the event stream.</Muted>
      {homeView === "board" ? <HomeBoard /> : homeView === "live" ? <LiveScreen heading={false} /> : <HomeTable />}
    </div>
  )
}

export function NeedsScreen() {
  const { items, item } = useStore()
  const sel = items.find((i) => i.id === item) ?? items[0]
  if (!sel) return <Empty><EmptyHeader><EmptyTitle>Nothing needs you</EmptyTitle><EmptyDescription>This fills when a Session waits for a person, an Effect is uncertain, a Plan diff waits, a Limit stops a run or a result needs review.</EmptyDescription></EmptyHeader></Empty>
  return (
    <div className="grid items-start gap-4 lg:grid-cols-[22rem_minmax(0,1fr)]">
      <ItemGroup>
        {items.map((i) => <ItemRow key={i.id} item={i} />)}
      </ItemGroup>
      <Decision item={sel} />
    </div>
  )
}

export function RunsScreen() {
  const { runs } = useStore()
  return <div className="flex flex-col gap-3"><H1>Runs</H1><Muted>From every Client: SDKs, CLI, MCP and the Console.</Muted><RunsTable runs={runs} /></div>
}

export function RunPage() {
  const { runById, run } = useStore()
  const r = runById(run)
  return <div className="flex flex-col gap-3"><Crumbs to="runs" label="Runs" here={r.id} /><RunHeader run={r} /><AttentionAlerts run={r} /><RunTabs run={r} /></div>
}

export function LiveScreen({ heading = true }: { heading?: boolean }) {
  const { runs, tile, set } = useStore()
  const live = runs.filter((r) => r.life === "running")
  const sel = live.find((r) => r.sessions[0].id === tile) ?? live[0]
  return (
    <div className="grid items-start gap-4 xl:grid-cols-[minmax(0,1fr)_20rem]">
      <div className="flex flex-col gap-3">
        {heading && <H1>Live</H1>}
        <div className="grid gap-3 md:grid-cols-2 2xl:grid-cols-3">
          {live.map((r) => (
            <Card key={r.id} size="sm">
              <CardHeader><CardTitle>{runName(r)}</CardTitle><CardDescription>{r.client} · {r.sessions[0].where}</CardDescription>
                <CardAction><Button size="xs" variant={sel?.id === r.id ? "default" : "outline"} onClick={() => set({ tile: r.sessions[0].id, tab: "timeline", step: -1 })}>Focus</Button></CardAction></CardHeader>
              <CardContent><div className="flex flex-col gap-2"><StateBadges run={r} /><Progress value={r.progress ?? 0} /></div></CardContent>
            </Card>
          ))}
        </div>
        {sel && <div className="flex flex-col gap-3"><RunHeader run={sel} /><AttentionAlerts run={sel} />{sel.sessions[0].where.includes("Chromium") && <LiveView run={sel} />}</div>}
      </div>
      <EventStream />
    </div>
  )
}

export function EventStream() {
  const { openRun } = useStore()
  return (
    <Card size="sm">
      <CardHeader><CardTitle>Event stream</CardTitle><CardDescription>Every Client, newest first</CardDescription></CardHeader>
      <CardContent>
        <div className="flex flex-col gap-1">
          {EVENTS.map(([t, r, e, w], k) => (
            <div key={k} className="flex flex-col border-b py-1 text-xs last:border-0">
              <span className="font-mono text-muted-foreground">{t} · <Button variant="link" size="xs" onClick={() => openRun(r)}>{r}</Button>{w ? " · attention" : ""}</span>
              <span>{e}</span>
            </div>
          ))}
        </div>
      </CardContent>
    </Card>
  )
}

export function RecipesScreen() {
  const { runs, items, set, go, pushDrawer } = useStore()
  const card = (id: string | null) => {
    const x = recipeOf(id)
    const rs = runs.filter((r) => (x ? r.recipe === x.id : !r.recipe))
    const n = items.filter((i) => rs.some((r) => r.id === i.run)).length
    return (
      <Card key={id ?? "_adhoc"} size="sm">
        <CardHeader><CardTitle>{x ? x.name : "Runs without a Recipe"}</CardTitle><CardDescription>{x ? `${x.pillars} · Plan ${x.plan}` : "SDK scripts, CLI, agents over MCP"}</CardDescription>
          {n > 0 && <CardAction><Badge variant="outline">{n} need you</Badge></CardAction>}</CardHeader>
        <CardContent><div className="flex flex-col gap-2">{rs[0] && <StateBadges run={rs[0]} />}<Muted>{rs.length} runs{rs[0] ? ` · last ${rs[0].started}` : ""}</Muted>
          <div><Button size="xs" variant="outline" onClick={() => { set({ recipe: id ?? "_adhoc" }); go("recipe") }}>Open</Button></div></div></CardContent>
      </Card>
    )
  }
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between"><H1>Recipes</H1><Button onClick={() => pushDrawer({ type: "create" })}>New Recipe</Button></div>
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">{RECIPES.map((r) => card(r.id))}{card(null)}</div>
    </div>
  )
}

export function RecipePage() {
  const { recipe, runs, pushDrawer } = useStore()
  const x = recipe === "_adhoc" ? null : recipeOf(recipe)
  const rs = runs.filter((r) => (x ? r.recipe === x.id : !r.recipe))
  return (
    <div className="flex flex-col gap-3">
      <Crumbs to="recipes" label="Recipes" here={x ? x.name : "Runs without a Recipe"} />
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div><H1>{x ? x.name : "Runs without a Recipe"}</H1><Muted>{x ? x.desc : "Started by SDK scripts, the CLI or agents over MCP without a Recipe."}</Muted></div>
        {x && <div className="flex gap-2"><Button onClick={say(`Run started from ${x.name} (simulated)`)}>Run</Button><Button variant="ghost" onClick={() => pushDrawer({ type: "code" })}>Copy as code</Button></div>}
      </div>
      {x && <Facts rows={[["Pillars", x.pillars], ["Plan", x.plan === "—" ? "rules, no Plan" : `${x.plan} · ${x.effects ? "has Effects: repairs need a reviewed diff" : "read-only: repairs promoted automatically"}`], ["Manifest", `recipes/${x.id}/automan.toml`]]} />}
      <RunsTable runs={rs} />
    </div>
  )
}

export function DatasetsScreen() {
  const { runs, openRun } = useStore()
  return (
    <div className="flex flex-col gap-3">
      <H1>Datasets</H1><Muted>One append-only Dataset per run. Exports, Baselines and erasure act here.</Muted>
      <Table>
        <TableHeader><TableRow><TableHead>Dataset</TableHead><TableHead>Records</TableHead><TableHead>Completeness</TableHead><TableHead>Purpose</TableHead><TableHead>Personal fields</TableHead><TableHead /></TableRow></TableHeader>
        <TableBody>{runs.filter((r) => r.records > 0).map((r) => (
          <TableRow key={r.id}>
            <TableCell><Button variant="link" size="sm" onClick={() => openRun(r.id)}>{runName(r)}</Button><div className="font-mono text-xs text-muted-foreground">{r.id}</div></TableCell>
            <TableCell>{r.records}</TableCell><TableCell><Badge variant="outline">{r.comp ?? "in progress"}</Badge></TableCell><TableCell>search</TableCell>
            <TableCell>{r.id === "R-2045" ? <Badge variant="outline">1 column flagged</Badge> : "none"}</TableCell>
            <TableCell><div className="flex gap-2"><Button size="xs" variant="outline" onClick={say("Export: JSON, JSONL, CSV, XLSX, XML or Parquet (simulated)")}>Export</Button><Button size="xs" variant="outline" onClick={say("Pick a Baseline run to compare with (simulated)")}>Compare</Button></div></TableCell>
          </TableRow>))}</TableBody>
      </Table>
      <div><Button variant="destructive" onClick={say("Erasure: finds a person's data across Datasets, Baselines and Checkpoints, then erases after confirmation (simulated)")}>Erase a person's data…</Button></div>
    </div>
  )
}

export function SessionsScreen() {
  const { runs, openSession, openRun } = useStore()
  const all = runs.flatMap((r) => r.sessions.map((s) => ({ s, r })))
  return (
    <div className="flex flex-col gap-3">
      <H1>Sessions & identities</H1>
      <Table>
        <TableHeader><TableRow><TableHead>Session</TableHead><TableHead>Run</TableHead><TableHead>Access mode</TableHead><TableHead>Proxy</TableHead><TableHead>Browser runs in</TableHead><TableHead>State</TableHead></TableRow></TableHeader>
        <TableBody>{all.map(({ s, r }) => (
          <TableRow key={s.id}><TableCell><Button variant="link" size="sm" onClick={() => openSession(s.id)}>{s.id}</Button></TableCell><TableCell><Button variant="link" size="sm" onClick={() => openRun(r.id)}>{r.id}</Button></TableCell><TableCell>{s.mode}</TableCell><TableCell>{s.proxy}</TableCell><TableCell>{s.where}</TableCell><TableCell>{s.state}</TableCell></TableRow>))}</TableBody>
      </Table>
      <Card size="sm"><CardHeader><CardTitle>Saved identities</CardTitle><CardDescription>Cookies and login state saved from a Session, reusable by other runs.</CardDescription></CardHeader>
        <CardContent><Facts rows={[["Supplier purchasing account", "saved from s-sup-0 · age-encrypted bundle · used by Supplier export"]]} /></CardContent></Card>
    </div>
  )
}

export function PlansScreen() {
  const { resolved, openItem, openRun } = useStore()
  const rows: [string, string, string, string, boolean][] = [["Pricing pages", "v8", "v7 → v8 promoted automatically (read-only)", "R-2048", false], ["Supplier export", "v4", "v5 candidate awaiting review (has Effects)", "R-2044", true], ["Reorder consumables", "v3", "no repair yet", "", false]]
  return (
    <div className="flex flex-col gap-3">
      <H1>Plans & Repairs</H1>
      <Table>
        <TableHeader><TableRow><TableHead>Plan</TableHead><TableHead>Current</TableHead><TableHead>Latest Repair</TableHead><TableHead /></TableRow></TableHeader>
        <TableBody>{rows.map(([n, v, s, r, rev]) => (
          <TableRow key={n}><TableCell>{n}</TableCell><TableCell>{v}</TableCell><TableCell>{s}{r && <> · <Button variant="link" size="xs" onClick={() => openRun(r)}>{r}</Button></>}</TableCell>
            <TableCell>{rev && !resolved[r] && <Button size="xs" onClick={() => openItem(`${r}:diff`)}>Review diff</Button>}</TableCell></TableRow>))}</TableBody>
      </Table>
      <Muted>The Console recorder writes new Plans: New → Record in a browser.</Muted>
    </div>
  )
}

export function SettingsScreen() {
  return <div className="flex flex-col gap-3"><H1>Engine & settings</H1><SettingsBody /></div>
}

// ---------- G only: competitors' confirmed patterns ----------
// Home = first-class "Needs you" list (Trigger.dev waitpoints, but for every kind) + recent runs that expand inline (Inngest).
export function HomeG() {
  const { runs, items, expanded, set, openRun } = useStore()
  return (
    <div className="flex flex-col gap-4">
      <H1>Home</H1>
      <Card size="sm"><CardHeader><CardTitle>Needs you · {items.length}</CardTitle><CardDescription>Every kind in one list: pauses, uncertain Effects, Plan diffs, Limits, results to review.</CardDescription></CardHeader>
        <CardContent><ItemGroup>{items.map((i) => <ItemRow key={i.id} item={i} />)}</ItemGroup></CardContent></Card>
      <div className="flex flex-col gap-1">
        <span className="text-sm font-medium">Runs · click a row to expand it, or open the full page</span>
        <Table>
          <TableHeader><TableRow><TableHead>Run</TableHead><TableHead>Started</TableHead><TableHead>State</TableHead><TableHead /></TableRow></TableHeader>
          <TableBody>
            {runs.map((r) => (
              <React.Fragment key={r.id}>
                <TableRow onClick={() => set({ expanded: { ...expanded, [r.id]: !expanded[r.id] } })}>
                  <TableCell><div className="font-medium">{expanded[r.id] ? "▾ " : "▸ "}{runName(r)}</div><div className="font-mono text-xs text-muted-foreground">{r.id}</div></TableCell>
                  <TableCell><div>{r.started}</div><div className="text-xs text-muted-foreground">{r.client}</div></TableCell>
                  <TableCell><StateBadges run={r} /></TableCell>
                  <TableCell><Button size="xs" variant="outline" onClick={(e) => { e.stopPropagation(); openRun(r.id) }}>Open full page</Button></TableCell>
                </TableRow>
                <TableRow>
                  <TableCell colSpan={4}>
                    <Collapsible open={!!expanded[r.id]}>
                      <CollapsibleContent>
                        <div className="grid gap-3 py-2 md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                          <ItemGroup>{r.steps.map((s, j) => <div key={j} className="flex items-center justify-between gap-2 text-xs"><span>{s.op}</span><ClassBadge cls={s.cls} /></div>)}</ItemGroup>
                          <Facts rows={[["Records", r.records], ["Requests", r.usage.requests], ["Browser time", r.usage.browser], ["Sessions", r.sessions.map((s) => s.id).join(", ")]]} />
                        </div>
                      </CollapsibleContent>
                    </Collapsible>
                  </TableCell>
                </TableRow>
              </React.Fragment>
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  )
}

// Run page = split view: run header and step list on the left, a step inspector on the right (Trigger.dev, Inngest traces).
export function RunPageG() {
  const { runById, run, step, set, openSession, items, openItem } = useStore()
  const r = runById(run)
  const i = step < 0 || step >= r.steps.length ? r.steps.length - 1 : step
  const s = r.steps[i]
  const its = items.filter((x) => x.run === r.id)
  return (
    <div className="flex flex-col gap-3">
      <Crumbs to="home" label="Home" here={r.id} />
      <RunHeader run={r} />
      <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,1fr)_24rem]">
        <div className="flex flex-col gap-2">
          {its.map((x) => <ItemRow key={x.id} item={x} />)}
          <span className="text-sm font-medium">Steps</span>
          <Table>
            <TableBody>{r.steps.map((x, j) => (
              <TableRow key={j} data-state={j === i ? "selected" : undefined} onClick={() => set({ step: j })}>
                <TableCell>{x.op}</TableCell><TableCell><ClassBadge cls={x.cls} /></TableCell><TableCell>{x.dur}</TableCell>
              </TableRow>))}</TableBody>
          </Table>
          <span className="text-sm font-medium">Sessions</span>
          <div className="flex flex-wrap gap-2">{r.sessions.map((x) => <Button key={x.id} size="sm" variant="outline" onClick={() => openSession(x.id)}>{x.id} · {x.state}</Button>)}</div>
          <Muted>Data, Usage, Controls, Effect journal and Audit log sit below the steps on the real page; they are the same as in the other variants.</Muted>
        </div>
        <Card size="sm">
          <CardHeader><CardTitle>Step inspector</CardTitle><CardDescription>{s.op}</CardDescription></CardHeader>
          <CardContent><div className="flex flex-col gap-3"><Facts rows={s.facts} /><Code>{s.code}</Code>{its[0] && s.st === "wait" && <Button onClick={() => openItem(its[0].id)}>Answer this pause</Button>}</div></CardContent>
        </Card>
      </div>
    </div>
  )
}
