// PROTOTYPE, throwaway. Shared building blocks for every variant.
// Rule for this prototype: shadcn components are used as shipped (Mira style, default theme);
// no className is passed to them. Layout comes only from wrapper <div>s.
import * as React from "react"
import { toast } from "sonner"
import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Checkbox } from "@/components/ui/checkbox"
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Item, ItemActions, ItemContent, ItemDescription, ItemGroup, ItemTitle } from "@/components/ui/item"
import { Label } from "@/components/ui/label"
import { Progress } from "@/components/ui/progress"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Separator } from "@/components/ui/separator"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import { ENGINE, lifeLabel, recipeOf, runName, type AttentionItem, type OpClass, type Run } from "@/data"
import { useStore } from "@/store"

export const say = (m: string) => () => toast(m)

export function Facts({ rows }: { rows: [string, React.ReactNode][] }) {
  return (
    <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
      {rows.map(([a, b]) => (
        <React.Fragment key={a}>
          <dt className="text-muted-foreground">{a}</dt>
          <dd className="font-medium">{b}</dd>
        </React.Fragment>
      ))}
    </dl>
  )
}

export function Code({ children }: { children: string }) {
  return <pre className="overflow-auto rounded-md bg-muted p-3 font-mono text-xs whitespace-pre-wrap">{children}</pre>
}

export function H1({ children }: { children: React.ReactNode }) {
  return <h1 className="text-lg font-semibold">{children}</h1>
}
export function Muted({ children }: { children: React.ReactNode }) {
  return <p className="text-xs text-muted-foreground">{children}</p>
}

export function StateBadges({ run }: { run: Run }) {
  const { resolved } = useStore()
  const [label, variant] = lifeLabel(run, resolved)
  return (
    <div className="flex flex-wrap items-center gap-1">
      <Badge variant={variant}>{label}</Badge>
      {run.comp && <Badge variant="outline">{run.comp}</Badge>}
      {run.marks.filter((m) => m !== "needs-review").map((m) => <Badge key={m} variant="ghost">mark: {m}</Badge>)}
    </div>
  )
}

export function ClassBadge({ cls }: { cls: OpClass }) {
  return <Badge variant={cls === "effect" ? "outline" : "ghost"}>{cls === "effect" ? "Effect" : cls}</Badge>
}

export function RunCommands({ run }: { run: Run }) {
  const { pushDrawer } = useStore()
  if (run.life === "running")
    return (
      <div className="flex flex-wrap gap-2">
        <Button variant="outline" size="sm" onClick={say(`Checkpoint taken (simulated): 4.2 MB, digest 9f1c…, epoch ${run.epoch}`)}>Checkpoint</Button>
        <Button variant="outline" size="sm" onClick={say("Admission paused: in-flight work finishes, nothing new starts (simulated)")}>Pause admission</Button>
        <Button variant="destructive" size="sm" onClick={say("Abort sent. In-flight Effects are checked; the run stops incomplete (simulated)")}>Abort</Button>
      </div>
    )
  return (
    <div className="flex flex-wrap gap-2">
      <Button variant="outline" size="sm" onClick={say("Opens a linked continuation: a new run from the last checkpoint (simulated)")}>Continue as new run</Button>
      <Button variant="outline" size="sm" onClick={say("Export: JSON, JSONL, CSV, XLSX, XML or Parquet (simulated)")}>Export…</Button>
      <Button variant="ghost" size="sm" onClick={() => pushDrawer({ type: "code" })}>Copy as CLI / Python / TS</Button>
    </div>
  )
}

export function RunHeader({ run, commands = true }: { run: Run; commands?: boolean }) {
  const rec = recipeOf(run.recipe)
  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="flex flex-col gap-0.5">
          <div className="flex items-baseline gap-2"><H1>{runName(run)}</H1><span className="font-mono text-xs text-muted-foreground">{run.id}</span></div>
          <Muted>Started {run.started} by <b>{run.client}</b> · epoch {run.epoch}{rec && rec.plan !== "—" ? ` · Plan ${run.repair ? run.repair.from : rec.plan}` : ""}</Muted>
        </div>
        <StateBadges run={run} />
      </div>
      {commands && <RunCommands run={run} />}
    </div>
  )
}

export function AttentionAlerts({ run }: { run: Run }) {
  const { items, openItem } = useStore()
  return (
    <>
      {items.filter((i) => i.run === run.id).map((i) => (
        <Alert key={i.id}>
          <AlertTitle>{i.kind}</AlertTitle>
          <AlertDescription>{i.sub}</AlertDescription>
          <AlertAction><Button size="sm" onClick={() => openItem(i.id)}>Open</Button></AlertAction>
        </Alert>
      ))}
    </>
  )
}

export function ItemRow({ item }: { item: AttentionItem }) {
  const { openItem } = useStore()
  return (
    <Item variant="outline" size="sm">
      <ItemContent>
        <ItemDescription>{item.kind}{item.urgent ? " · waiting now" : ""}</ItemDescription>
        <ItemTitle>{item.title}</ItemTitle>
        <ItemDescription>{item.sub}</ItemDescription>
      </ItemContent>
      <ItemActions><Button size="sm" onClick={() => openItem(item.id)}>Open</Button></ItemActions>
    </Item>
  )
}

export function Timeline({ run }: { run: Run }) {
  const { step, set } = useStore()
  const i = step < 0 || step >= run.steps.length ? run.steps.length - 1 : step
  const s = run.steps[i]
  return (
    <div className="grid gap-3 lg:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)]">
      <ItemGroup>
        {run.steps.map((x, j) => (
          <Item key={j} variant={j === i ? "muted" : "outline"} size="sm" render={<button type="button" onClick={() => set({ step: j })} />}>
            <ItemContent>
              <ItemTitle>{x.op}</ItemTitle>
              <ItemDescription>{x.st === "wait" ? "Waiting for a person" : x.st === "err" ? "Problem" : x.st === "warn" ? "Needed attention" : x.st === "run" ? "In progress" : "Done"} · {x.dur}</ItemDescription>
            </ItemContent>
            <ItemActions><ClassBadge cls={x.cls} /></ItemActions>
          </Item>
        ))}
      </ItemGroup>
      <Card size="sm">
        <CardHeader><CardTitle>Evidence for this step</CardTitle></CardHeader>
        <CardContent><div className="flex flex-col gap-3"><Facts rows={s.facts} /><Code>{s.code}</Code></div></CardContent>
      </Card>
    </div>
  )
}

export function SessionsTable({ run }: { run: Run }) {
  const { openSession } = useStore()
  return (
    <Table>
      <TableHeader><TableRow><TableHead>Session</TableHead><TableHead>Access mode</TableHead><TableHead>Proxy</TableHead><TableHead>Browser runs in</TableHead><TableHead>State</TableHead></TableRow></TableHeader>
      <TableBody>
        {run.sessions.map((s) => (
          <TableRow key={s.id}>
            <TableCell><Button variant="link" size="sm" onClick={() => openSession(s.id)}>{s.id}</Button></TableCell>
            <TableCell>{s.mode}</TableCell><TableCell>{s.proxy}</TableCell><TableCell>{s.where}</TableCell><TableCell>{s.state}</TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}

export function RunTabs({ run }: { run: Run }) {
  const { tab, set } = useStore()
  return (
    <Tabs value={tab} onValueChange={(v) => set({ tab: String(v), step: -1 })}>
      <TabsList variant="line">
        <TabsTrigger value="timeline">Timeline</TabsTrigger>
        <TabsTrigger value="data">Data · {run.records}</TabsTrigger>
        <TabsTrigger value="sessions">Sessions</TabsTrigger>
        <TabsTrigger value="usage">Usage & Limits</TabsTrigger>
        <TabsTrigger value="controls">Controls & marks</TabsTrigger>
        <TabsTrigger value="journal">Effect journal</TabsTrigger>
        <TabsTrigger value="audit">Audit log</TabsTrigger>
      </TabsList>
      <TabsContent value="timeline"><Timeline run={run} /></TabsContent>
      <TabsContent value="data"><DataTab run={run} /></TabsContent>
      <TabsContent value="sessions"><SessionsTable run={run} /></TabsContent>
      <TabsContent value="usage"><UsageTab run={run} /></TabsContent>
      <TabsContent value="controls"><ControlsTab run={run} /></TabsContent>
      <TabsContent value="journal"><JournalTab run={run} /></TabsContent>
      <TabsContent value="audit"><AuditTab run={run} /></TabsContent>
    </Tabs>
  )
}

function DataTab({ run }: { run: Run }) {
  if (!run.records) return <Muted>No Records yet. A Record appears here as soon as it is committed.</Muted>
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <Muted>{run.records} Records · {run.comp ?? "in progress"} · stream numbered in commit order</Muted>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={say("Compared with Baseline R-2043: +24 new, 3 changed prices, 1 removed (simulated)")}>Compare with Baseline</Button>
          <Button variant="outline" size="sm" onClick={say("Sink deliveries: PostgreSQL exactly-once, all delivered (simulated)")}>Sinks</Button>
        </div>
      </div>
      <Table>
        <TableHeader><TableRow><TableHead>#</TableHead><TableHead>sku</TableHead><TableHead>name</TableHead><TableHead>price</TableHead><TableHead>Provenance</TableHead></TableRow></TableHeader>
        <TableBody>
          {[1, 2, 3, 4].map((n) => (
            <TableRow key={n}><TableCell>{n}</TableCell><TableCell>EQ-{1040 + n}</TableCell><TableCell>Equipment item {n}</TableCell><TableCell>€{(19.9 * n).toFixed(2)}</TableCell><TableCell>fetched 10:2{n} · s-cat-1 · HTTP tier (compared) · dom-smoothie · Purpose search</TableCell></TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
}

function UsageTab({ run }: { run: Run }) {
  const u = run.usage
  return (
    <div className="flex flex-col gap-4">
      <Facts rows={[["Requests", u.requests], ["Browser time", u.browser], ["Model tokens", u.tokens], ["Proxy bytes", u.proxy], ["Solver attempts", u.solver]]} />
      {run.limit ? (
        <div className="flex max-w-md flex-col gap-1">
          <Label>Currency Limit: ${run.limit.used.toFixed(2)} of ${run.limit.cap.toFixed(2)} (estimated)</Label>
          <Progress value={Math.min(100, (run.limit.used / run.limit.cap) * 100)} />
          <Muted>Estimated from the shipped price table, which you can override. The engine counts units; it never bills.</Muted>
        </div>
      ) : <Muted>No Limit set on this run. Units only: the engine knows no prices.</Muted>}
    </div>
  )
}

function ControlsTab({ run }: { run: Run }) {
  const rows: [string, string, string][] = [
    ["Actionability guard", "on", "unguarded"],
    ["Page check", run.marks.includes("unchecked") ? "off" : "on", "unchecked"],
    ["Fetch-tier comparison", run.marks.includes("forced tier") ? "off (forced HTTP)" : "on", "forced tier"],
    ["Backoff on 429 / Retry-After", "on", "ignored"],
    ["Fingerprint check before Effects", "on", "Effects unverified"],
    ["Success checks", run.marks.includes("needs-review") ? "none declared" : "on", "needs-review"],
  ]
  return (
    <div className="flex flex-col gap-2">
      <Table>
        <TableHeader><TableRow><TableHead>Costly control</TableHead><TableHead>State</TableHead><TableHead>Mark in results when off</TableHead></TableRow></TableHeader>
        <TableBody>{rows.map(([a, b, c]) => <TableRow key={a}><TableCell>{a}</TableCell><TableCell><Badge variant={b === "on" ? "secondary" : "outline"}>{b}</Badge></TableCell><TableCell>{c}</TableCell></TableRow>)}</TableBody>
      </Table>
      <Muted>Durability: power-cut safe (FULL). Access mode: standard. Purpose: search. Respect Site preferences: off (recorded in Provenance).</Muted>
    </div>
  )
}

function JournalTab({ run }: { run: Run }) {
  const eff = run.steps.filter((s) => s.cls === "effect")
  if (!eff.length) return <Muted>No Effects in this run: everything it did was a read.</Muted>
  return (
    <Table>
      <TableHeader><TableRow><TableHead>#</TableHead><TableHead>Effect or model call</TableHead><TableHead>Decided by</TableHead><TableHead>State</TableHead></TableRow></TableHeader>
      <TableBody>{eff.map((s, j) => <TableRow key={j}><TableCell>{j + 1}</TableCell><TableCell>{s.op}</TableCell><TableCell>{run.client.startsWith("MCP") ? "model (via MCP)" : "Plan / script"}</TableCell><TableCell><Badge variant={s.st === "err" ? "destructive" : s.st === "wait" ? "outline" : "secondary"}>{s.st === "err" ? "uncertain" : s.st === "wait" ? "awaiting a person" : "committed"}</Badge></TableCell></TableRow>)}</TableBody>
    </Table>
  )
}

function AuditTab({ run }: { run: Run }) {
  return (
    <div className="flex flex-col gap-2">
      <Table>
        <TableHeader><TableRow><TableHead>Time</TableHead><TableHead>Event</TableHead><TableHead>Who</TableHead></TableRow></TableHeader>
        <TableBody>
          <TableRow><TableCell>{run.started}</TableCell><TableCell>Run started; Purpose search; Site preferences recorded, not enforced</TableCell><TableCell>{run.client}</TableCell></TableRow>
          {run.recipe === "supplier" && <TableRow><TableCell>09:59</TableCell><TableCell>Secret used: keychain:automan/supplier (value never logged)</TableCell><TableCell>engine</TableCell></TableRow>}
          {run.marks.length > 0 && <TableRow><TableCell>—</TableCell><TableCell>Costly controls switched off: {run.marks.join(", ")}</TableCell><TableCell>{run.client}</TableCell></TableRow>}
        </TableBody>
      </Table>
      <Muted>Hash-chained, append-only. Export as JSONL or OpenTelemetry.</Muted>
    </div>
  )
}

// ---------- live view (a fake browser frame) ----------
export function LiveView({ run, interactive = true }: { run: Run; interactive?: boolean }) {
  const { control, set } = useStore()
  const mfa = run.wait?.kind === "MFA"
  const url = mfa ? "https://portal.supplier.example.invalid/mfa" : run.wait ? "https://shop.example.invalid/checkout" : "https://catalog.example.invalid/products?page=12"
  return (
    <div className="flex flex-col gap-2">
      <div className="overflow-hidden rounded-md border">
        <div className="flex items-center gap-2 border-b bg-muted px-2 py-1.5">
          <Badge variant="destructive">LIVE</Badge>
          <span className="flex-1 truncate font-mono text-xs text-muted-foreground">{url}</span>
          <span className="font-mono text-xs text-muted-foreground">{run.sessions[0].id}</span>
        </div>
        <div className="flex min-h-36 flex-col gap-2 bg-background p-4 text-sm">
          {mfa ? (<><b>Supplier portal</b><span className="text-muted-foreground">Approve the sign-in on your phone</span><span>We sent a notification to the device ending in ••42. Tap Approve to continue.</span></>)
            : run.wait ? (<><b>shop.example.invalid · Checkout</b><span>6 items · Total <b>€184.20</b></span><span className="rounded border px-2 py-1 text-muted-foreground">Card •••• 4242</span></>)
            : (<><b>{runName(run)}</b><span className="text-muted-foreground">Page 12 of the crawl…</span><span className="rounded border px-2 py-1 text-muted-foreground">EQ-1044 · Equipment item · €79.60</span></>)}
        </div>
      </div>
      {interactive && (
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="outline" size="sm" onClick={() => set({ control: !control })}>{control ? "Give control back" : "Take control of this page"}</Button>
          <Muted>{control ? "You are driving this Session. The takeover is in the Audit log. Password fields stay masked: the engine types Secret references." : "Frames are coalesced; the view is read-only until you take control."}</Muted>
        </div>
      )}
    </div>
  )
}

// ---------- decision panels, one per kind of "Needs you" item ----------
export function Decision({ item }: { item: AttentionItem }) {
  const { runById, resolve, openRun } = useStore()
  const r = runById(item.run)
  const head = (
    <div className="flex flex-col gap-1">
      <Muted>{item.kind} · <Button variant="link" size="xs" onClick={() => openRun(r.id)}>{r.id} {runName(r)}</Button> · started by {r.client}</Muted>
    </div>
  )
  const ok = (msg: string) => () => resolve(r.id, msg)
  switch (item.kind) {
    case "Human pause · MFA":
      return (
        <div className="flex flex-col gap-3">{head}<H1>Approve the sign-in on the account holder's phone</H1><p className="text-sm">{r.wait?.why}</p>
          <Alert><AlertTitle>Only Session {r.sessions[0].id} waits</AlertTitle><AlertDescription>The run's other work continues. Expires in {r.wait?.expires}, then E_HUMAN_TIMEOUT. The browser is held 15 minutes, then saved and released.</AlertDescription></Alert>
          <LiveView run={r} />
          <div className="flex flex-wrap gap-2"><Button onClick={ok("Approved: the engine saw the portal home page (Page check: expected page) and resumed the Session")}>I approved it, continue</Button><Button variant="outline" onClick={say("Switches to a TOTP or e-mail code source if one is configured (simulated)")}>Use another code source</Button><Button variant="destructive" onClick={ok("Gave up: the Session's work items fail with E_HUMAN_TIMEOUT now")}>Give up</Button></div>
        </div>)
    case "Human pause · Confirmation":
      return (
        <div className="flex flex-col gap-3">{head}<H1>Confirm a payment the agent decided</H1><p className="text-sm">{r.wait?.why}</p>
          <Card size="sm"><CardContent><Facts rows={[["Effect", r.wait?.effect], ["Category", "payment"], ["Decided by", "claude-sonnet-5-5, driven by Claude Code over MCP"], ["Target fingerprint", "button \"Place order\" · checked, matches"], ["Domain policy", "shop.example.invalid allowed"]]} /></CardContent></Card>
          <LiveView run={r} interactive={false} />
          <div className="flex flex-wrap items-center gap-2"><Button onClick={ok("Approved: the Effect runs once through the Effect journal; the approval is in the Audit log")}>Approve this payment</Button><Button variant="destructive" onClick={ok("Denied: the agent gets a typed refusal and the run continues without the Effect")}>Deny</Button></div>
          <div className="flex items-center gap-2"><Checkbox id="allow" /><Label htmlFor="allow">Allow payments up to €200 for the rest of this run</Label></div>
        </div>)
    case "Uncertain Effect":
      return (
        <div className="flex flex-col gap-3">{head}<H1>Did this Effect happen?</H1><p className="text-sm">{r.uncertain?.why}</p>
          <Card size="sm"><CardContent><Facts rows={[["Effect", r.uncertain?.effect], ["Evidence", r.uncertain?.evidence], ["Epoch", "1 → 2"]]} /></CardContent></Card>
          <Alert><AlertTitle>Proposed action, not in any ADR yet</AlertTitle><AlertDescription>A person reconciles. "It happened" commits the Effect in the journal; "It did not" allows one retry in a continuation; both go to the Audit log.</AlertDescription></Alert>
          <div className="flex flex-wrap gap-2"><Button variant="outline" onClick={say("Opens the site in a fresh Session to check the order history (simulated)")}>Check on the site</Button><Button onClick={ok("Recorded: the order happened. A continuation will not repeat it")}>It happened</Button><Button variant="outline" onClick={ok("Recorded: it did not happen. A continuation may perform it once")}>It did not happen</Button><Button variant="ghost" onClick={ok("Left uncertain: the run stays Needs review")}>Leave uncertain</Button></div>
        </div>)
    case "Plan diff review":
      return (
        <div className="flex flex-col gap-3">{head}<H1>Review Plan {r.repair?.from} → {r.repair?.to}</H1><p className="text-sm">{r.repair?.why}</p>
          <div className="grid gap-2 md:grid-cols-2">
            <Card size="sm"><CardHeader><CardTitle>{r.repair?.from} · step 4</CardTitle></CardHeader><CardContent><Code>{"action: click\ntarget: button \"Request export\"\nfingerprint: toolbar › button[0]\neffect: request_export"}</Code></CardContent></Card>
            <Card size="sm"><CardHeader><CardTitle>{r.repair?.to} · step 4 (candidate)</CardTitle></CardHeader><CardContent><Code>{"action: open_menu \"More\"; click\ntarget: menuitem \"Request export\"\nfingerprint: toolbar › menu \"More\" › item[2]\neffect: request_export"}</Code></CardContent></Card>
          </div>
          <Muted>Verified in run R-2044: Success check "export file received, 1 file, &gt; 10 kB" passed. The Effect ran once. A Repair cannot lower business criteria.</Muted>
          <div className="flex flex-wrap gap-2"><Button onClick={ok("Promoted: Supplier export now runs Plan v5; v4 is kept for rollback")}>Promote {r.repair?.to}</Button><Button variant="destructive" onClick={ok("Rejected with a reason; v4 stays current")}>Reject</Button></div>
          <div className="flex items-center gap-2"><Checkbox id="auto" /><Label htmlFor="auto">Auto-approve future repairs of this Recipe</Label></div>
        </div>)
    case "Limit reached":
      return (
        <div className="flex flex-col gap-3">{head}<H1>Stopped by its Limit: raise it to resume</H1><p className="text-sm">{r.stopReason}. A run stopped by a Limit is never complete: {r.records} Records committed, 1,804 URLs still in the Frontier.</p>
          <Field><FieldLabel htmlFor="lim">New Limit (USD, estimated)</FieldLabel><Input id="lim" defaultValue="5.00" /><FieldDescription>The engine counts tokens and reserves each call's worst case. Dollars come from the shipped price table.</FieldDescription></Field>
          <div className="flex flex-wrap gap-2"><Button onClick={ok("Resumed from its checkpoint with a $5.00 Limit (epoch 2)")}>Raise to $5.00 and resume</Button><Button variant="outline" onClick={ok("Kept as incomplete. Its Records stay usable and marked incomplete")}>Keep as incomplete</Button></div>
        </div>)
    case "Result needs review":
      return (
        <div className="flex flex-col gap-3">{head}<H1>Check this result</H1><p className="text-sm">The agent said {r.agentSaid} No Success check was declared, so the engine cannot call it verified. Grounding found 11 of 12 values in their pages.</p>
          <Table><TableHeader><TableRow><TableHead>Product</TableHead><TableHead>Warranty</TableHead><TableHead>Grounding</TableHead></TableRow></TableHeader>
            <TableBody>{["Drill D-18", "Saw S-22", "Sander X-3"].map((p, i) => <TableRow key={p}><TableCell>{p}</TableCell><TableCell>{i === 2 ? "5 years" : "2 years"}</TableCell><TableCell><Badge variant={i === 2 ? "destructive" : "secondary"}>{i === 2 ? "not found in page" : "in page"}</Badge></TableCell></TableRow>)}</TableBody></Table>
          <div className="flex flex-wrap gap-2"><Button onClick={ok("Accepted by a person; recorded in the Audit log")}>Accept as reviewed</Button><Button variant="outline" onClick={say("Opens a Success check editor (simulated)")}>Add a Success check</Button><Button variant="outline" onClick={say("Saves the agent's steps as a Plan candidate (simulated)")}>Save as Plan</Button><Button variant="destructive" onClick={ok("Rejected; the reason is kept")}>Reject</Button></div>
        </div>)
  }
}

// ---------- Session detail (with an optional Steel-style replay + synced action log) ----------
export function SessionDetail({ id, replay = false }: { id: string; replay?: boolean }) {
  const { runs, openRun } = useStore()
  const run = runs.find((r) => r.sessions.some((s) => s.id === id))
  const [at, setAt] = React.useState(0)
  if (!run) return <Muted>This Session no longer exists.</Muted>
  const s = run.sessions.find((x) => x.id === id)!
  const browser = s.where.includes("Chromium")
  return (
    <div className="flex flex-col gap-3">
      <Muted>Session of run <Button variant="link" size="xs" onClick={() => openRun(run.id)}>{run.id} {runName(run)}</Button></Muted>
      <div className="flex flex-wrap items-center justify-between gap-2"><span className="font-mono text-base font-semibold">{s.id}</span><Badge variant={s.state.startsWith("waiting") ? "outline" : "secondary"}>{s.state}</Badge></div>
      {browser && run.life === "running" && <LiveView run={run} />}
      {replay && browser && run.life !== "running" && (
        <div className="flex flex-col gap-2">
          <div className="flex min-h-28 items-center justify-center rounded-md border bg-muted text-xs text-muted-foreground">Replay · frame at step {at + 1} of {run.steps.length}</div>
          <input type="range" min={0} max={run.steps.length - 1} value={at} onChange={(e) => setAt(+e.target.value)} aria-label="Replay position" />
        </div>
      )}
      <Facts rows={[["Access mode", `${s.mode} (fixed for the Session's life)`], ["Identity profile", "Chrome 141 · macOS · fr-FR · Europe/Paris"], ["Proxy", s.proxy], ["Browser runs in", s.where], ["Page check", run.id === "R-2042" ? "off (results marked unchecked)" : "declared: expected page and login page"], ["Pool score", s.state === "retired" ? "retired after E_PAGE_CHECK" : "0.94 · 0 blocks in 40 pages"], ["Cookies", browser ? "23 across 12 domains, kept in run state" : "4"]]} />
      <Separator />
      <span className="text-sm font-medium">{replay ? "Action log (click to move the replay)" : "What this Session did"}</span>
      <ItemGroup>
        {run.steps.map((x, j) => (
          <Item key={j} size="xs" variant={replay && j === at ? "muted" : "default"} render={replay ? <button type="button" onClick={() => setAt(j)} /> : undefined}>
            <ItemContent><ItemTitle>{x.op}</ItemTitle><ItemDescription>{x.dur}</ItemDescription></ItemContent>
            <ItemActions><ClassBadge cls={x.cls} /></ItemActions>
          </Item>
        ))}
      </ItemGroup>
      {replay && (
        <Tabs defaultValue="events">
          <TabsList variant="line"><TabsTrigger value="events">Events</TabsTrigger><TabsTrigger value="network">Network</TabsTrigger><TabsTrigger value="console">Console</TabsTrigger></TabsList>
          <TabsContent value="events"><Code>{"10:12:31 secret used keychain:automan/supplier\n10:12:40 human_pause mfa"}</Code></TabsContent>
          <TabsContent value="network"><Code>{"GET /login 200 18 kB\nPOST /session 302\nGET /mfa 200 9 kB"}</Code></TabsContent>
          <TabsContent value="console"><Code>{"[info] app boot\n[warn] deprecated API"}</Code></TabsContent>
        </Tabs>
      )}
      <div className="flex flex-wrap gap-2">
        <Button variant="outline" size="sm" onClick={say("Saved as an identity: cookies and login state (simulated)")}>Save as identity</Button>
        <Button variant="outline" size="sm" onClick={say("Exported as an age-encrypted bundle (simulated)")}>Export</Button>
        {browser && <Button variant="outline" size="sm" onClick={say("Raw CDP: short-lived token, audited; results marked unguarded while attached (simulated)")}>Attach raw CDP…</Button>}
        <Button variant="destructive" size="sm" onClick={say("Session retired; its work moves to a new Session (simulated)")}>Retire</Button>
      </div>
    </div>
  )
}

// ---------- New: kind → form → compiled configuration review → run ----------
const KINDS: [string, string, string][] = [
  ["collect", "Collect data", "Fields from known pages. Rules, or AI that compiles a Plan that replays without a model."],
  ["crawl", "Crawl a site", "Seeds plus a scope. A prompt compiles into a configuration you review before it runs."],
  ["automate", "Automate a flow", "Steps with an identity, inputs and an expected result. Effects are journaled."],
  ["agent", "Run an agent task", "Objective, allowed domains and tools, a Limit and a Success check."],
  ["record", "Record in a browser", "The recorder writes a Plan (and matching TS or Python) from what you do."],
]
export function CreateFlow() {
  const { startRun, pushDrawer } = useStore()
  const [step, setStep] = React.useState(0)
  const [kind, setKind] = React.useState<string | null>(null)
  if (step === 0)
    return (
      <div className="flex flex-col gap-3">
        <span className="text-sm font-medium">What do you want to do?</span>
        <ItemGroup>
          {KINDS.map(([k, t, d]) => (
            <Item key={k} variant={kind === k ? "muted" : "outline"} size="sm" render={<button type="button" onClick={() => setKind(k)} />}>
              <ItemContent><ItemTitle>{t}</ItemTitle><ItemDescription>{d}</ItemDescription></ItemContent>
            </Item>
          ))}
        </ItemGroup>
        <div><Button disabled={!kind} onClick={() => setStep(1)}>Next</Button></div>
      </div>
    )
  if (step === 1)
    return (
      <div className="flex flex-col gap-3">
        <FieldGroup>
          <Field><FieldLabel htmlFor="seed">Start URL or seeds</FieldLabel><Input id="seed" defaultValue="https://catalog.example.invalid/" /></Field>
          <Field><FieldLabel htmlFor="what">{kind === "agent" ? "Objective" : "What you want, in words"}</FieldLabel><Textarea id="what" defaultValue="Every product with sku, name, price and stock" /></Field>
          <Field><FieldLabel>Purpose</FieldLabel>
            <Select defaultValue="search"><SelectTrigger><SelectValue /></SelectTrigger><SelectContent><SelectItem value="search">search (default)</SelectItem><SelectItem value="ai-input">ai-input</SelectItem><SelectItem value="ai-train">ai-train</SelectItem><SelectItem value="collect">collect</SelectItem></SelectContent></Select></Field>
          <Field><FieldLabel>Access mode</FieldLabel>
            <Select defaultValue="standard"><SelectTrigger><SelectValue /></SelectTrigger><SelectContent><SelectItem value="standard">standard (default)</SelectItem><SelectItem value="identified">identified</SelectItem><SelectItem value="stealth">stealth</SelectItem></SelectContent></Select></Field>
          <Field><FieldLabel htmlFor="limit">Limit</FieldLabel><Input id="limit" defaultValue="$1.00 est., 5,000 requests" /></Field>
          <Field><FieldLabel htmlFor="check">Success check</FieldLabel><Input id="check" defaultValue="one Record per sku, price > 0" /><FieldDescription>Advanced (concurrency, proxies, Sessions, costly controls, Sinks, model provider) stays folded until asked for.</FieldDescription></Field>
        </FieldGroup>
        <div className="flex gap-2"><Button variant="outline" onClick={() => setStep(0)}>Back</Button><Button onClick={() => setStep(2)}>Compile and preview</Button></div>
      </div>
    )
  return (
    <div className="flex flex-col gap-3">
      <Muted>The prompt compiled into this configuration (ADR 0011). A 5-page sample ran with no Effects.</Muted>
      <Code>{"recipe: equipment-catalog\nseeds: [https://catalog.example.invalid/]\nscope: same registrable domain, /products/**\nextract: { sku, name, price, stock }\npurpose: search\naccess: standard\nlimits: { usd_estimate: 1.00, requests: 5000 }\nsuccess_check: one record per sku, price > 0"}</Code>
      <Table><TableHeader><TableRow><TableHead>sku</TableHead><TableHead>name</TableHead><TableHead>price</TableHead></TableRow></TableHeader><TableBody><TableRow><TableCell>EQ-1041</TableCell><TableCell>Equipment item 1</TableCell><TableCell>€19.90</TableCell></TableRow><TableRow><TableCell>EQ-1042</TableCell><TableCell>Equipment item 2</TableCell><TableCell>€39.80</TableCell></TableRow></TableBody></Table>
      <div className="flex flex-wrap gap-2"><Button variant="outline" onClick={() => setStep(1)}>Back</Button><Button onClick={startRun}>Save as Recipe and run</Button><Button variant="outline" onClick={startRun}>Run once (no Recipe)</Button><Button variant="ghost" onClick={() => pushDrawer({ type: "code" })}>Copy as code</Button></div>
    </div>
  )
}

export function CodeView() {
  return (
    <div className="flex flex-col gap-3">
      <Code>{"automan run equipment-catalog --limit-usd 1.00"}</Code>
      <Code>{"from automan import Client\nasync with Client() as c:   # Attached engine, or the Daemon if running\n    run = await c.runs.start(\"equipment-catalog\")\n    async for rec in run.records(): ..."}</Code>
      <Code>{"import { Client } from \"automan\"\nconst run = await new Client().runs.start(\"equipment-catalog\")"}</Code>
    </div>
  )
}

export function SettingsBody() {
  return (
    <div className="flex flex-col gap-4">
      <Card size="sm"><CardHeader><CardTitle>Engine</CardTitle><CardDescription>{ENGINE.mode} on {ENGINE.addr} · v{ENGINE.version} · loopback, token cookie</CardDescription></CardHeader>
        <CardContent><Facts rows={[["Lifetime", ENGINE.idle], ["Run storage", ENGINE.layout], ["Durability", ENGINE.durability]]} /></CardContent></Card>
      <Card size="sm"><CardHeader><CardTitle>Connected Clients</CardTitle></CardHeader>
        <CardContent><Table><TableBody>{ENGINE.clients.map((c) => <TableRow key={c.who}><TableCell>{c.who}</TableCell><TableCell>{c.detail}</TableCell><TableCell>since {c.since}</TableCell></TableRow>)}</TableBody></Table></CardContent></Card>
      <Card size="sm"><CardHeader><CardTitle>Secret references</CardTitle><CardDescription>Values are never shown, stored by the Console, or sent to a model.</CardDescription></CardHeader>
        <CardContent><Table><TableHeader><TableRow><TableHead>Placeholder</TableHead><TableHead>Reference</TableHead><TableHead>Allowed domains</TableHead></TableRow></TableHeader><TableBody>
          <TableRow><TableCell>%SUPPLIER_PASSWORD%</TableCell><TableCell>keychain:automan/supplier</TableCell><TableCell>portal.supplier.example.invalid</TableCell></TableRow>
          <TableRow><TableCell>%SUPPLIER_TOTP%</TableCell><TableCell>op://ops/supplier/totp</TableCell><TableCell>portal.supplier.example.invalid</TableCell></TableRow></TableBody></Table></CardContent></Card>
      <Card size="sm"><CardHeader><CardTitle>Providers and defaults</CardTitle></CardHeader>
        <CardContent><Facts rows={[["Model providers", "OpenRouter (env:OPENROUTER_KEY) · Ollama at 127.0.0.1:11434"], ["Proxies", "ISP via Bright Data adapter"], ["Sinks", "PostgreSQL (exactly-once) · signed webhook (at-least-once)"], ["MFA code sources", "IMAP ops@… · Twilio"], ["Defaults", "Purpose search · Site preferences off · Access standard · security on · Audit log 12 months"]]} /></CardContent></Card>
    </div>
  )
}
