// PROTOTYPE, throwaway. Seven variants of the whole Console (?variant=A..G).
import { useEffect, type ReactNode } from "react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Drawer, DrawerContent, DrawerDescription, DrawerHeader, DrawerTitle } from "@/components/ui/drawer"
import { Item, ItemContent, ItemDescription, ItemGroup, ItemTitle } from "@/components/ui/item"
import { Kbd } from "@/components/ui/kbd"
import { Breadcrumb, BreadcrumbItem, BreadcrumbLink, BreadcrumbList, BreadcrumbPage, BreadcrumbSeparator } from "@/components/ui/breadcrumb"
import { Sidebar, SidebarContent, SidebarGroup, SidebarGroupContent, SidebarHeader, SidebarInset, SidebarMenu, SidebarMenuBadge, SidebarMenuButton, SidebarMenuItem, SidebarProvider, SidebarTrigger } from "@/components/ui/sidebar"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ENGINE, RECIPES, recipeOf, runName } from "@/data"
import { useTheme } from "@/components/theme-provider"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { useStore, VARIANTS, type DrawerEntry, type Route, type VariantKey } from "@/store"
import { AttentionAlerts, CodeView, CreateFlow, Decision, H1, ItemRow, LiveView, Muted, RunHeader, RunTabs, SessionDetail, SettingsBody, StateBadges } from "./parts"
import { DatasetsScreen, EventStream, HomeG, HomeScreen, LiveScreen, NeedsScreen, PlansScreen, RecipePage, RecipesScreen, RunPage, RunPageG, RunsScreen, RunsTable, SessionsScreen, SettingsScreen } from "./screens"

function TopBar() {
  const { pushDrawer, variant, items } = useStore()
  const { theme, setTheme } = useTheme()
  return (
    <header className="flex items-center gap-3 border-b bg-background px-4 py-2">
      {(variant === "E" || variant === "F" || variant === "G") && <SidebarTrigger />}
      <span className="font-semibold">Automan</span>
      <span className="hidden text-xs text-muted-foreground md:inline">{ENGINE.mode} · {ENGINE.addr} · v{ENGINE.version} · {ENGINE.clients.length} Clients</span>
      <span className="flex-1" />
      <span className="hidden text-xs text-muted-foreground md:inline">Search or run a command <Kbd>⌘K</Kbd></span>
      <ToggleGroup value={[theme]} onValueChange={(v) => v[0] && setTheme(v[0] as "light" | "dark" | "system")} variant="outline" size="sm" aria-label="Theme">
        <ToggleGroupItem value="light">Light</ToggleGroupItem><ToggleGroupItem value="dark">Dark</ToggleGroupItem><ToggleGroupItem value="system">System</ToggleGroupItem>
      </ToggleGroup>
      {variant === "F" && <Button variant="outline" size="sm" onClick={() => pushDrawer({ type: "activity" })}>Activity · {items.length}</Button>}
      <Button size="sm" onClick={() => pushDrawer({ type: "create" })}>New</Button>
      {variant !== "E" && variant !== "F" && variant !== "G" && <Button variant="outline" size="sm" onClick={() => pushDrawer({ type: "settings" })}>Engine & settings</Button>}
    </header>
  )
}

// ---------- A: Workbench (Runs are the root) ----------
function VariantA() {
  const { runs, items, aNav, set, run, runById, openRun, recipe } = useStore()
  const navs: [string, string, number | string][] = [["runs", "Runs", runs.length], ["needs", "Needs you", items.length], ["recipes", "Recipes", RECIPES.length], ["data", "Datasets", ""], ["sessions", "Sessions & identities", ""], ["plans", "Plans & Repairs", ""]]
  const list = aNav === "needs" ? runs.filter((r) => items.some((i) => i.run === r.id)) : runs
  const r = runById(run)
  const rec = recipeOf(recipe ?? "catalog")!
  return (
    <div className="grid min-h-0 flex-1 md:grid-cols-[12rem_22rem_minmax(0,1fr)]">
      <nav className="flex flex-col gap-1 border-r p-2">
        {navs.map(([k, l, c]) => <Button key={k} variant={aNav === k ? "secondary" : "ghost"} onClick={() => set({ aNav: k })}>{l}{c !== "" ? ` · ${c}` : ""}</Button>)}
      </nav>
      <div className="min-h-0 overflow-auto border-r p-2">
        {aNav === "runs" || aNav === "needs" ? (
          <ItemGroup>{list.map((x) => (
            <Item key={x.id} size="sm" variant={x.id === run ? "muted" : "default"} render={<button type="button" onClick={() => openRun(x.id)} />}>
              <ItemContent><ItemTitle>{runName(x)}</ItemTitle><ItemDescription>{x.id} · {x.started} · {x.client}</ItemDescription><StateBadges run={x} /></ItemContent>
            </Item>))}</ItemGroup>
        ) : aNav === "recipes" ? (
          <ItemGroup>{RECIPES.map((x) => <Item key={x.id} size="sm" variant={x.id === rec.id ? "muted" : "default"} render={<button type="button" onClick={() => set({ recipe: x.id })} />}><ItemContent><ItemTitle>{x.name}</ItemTitle><ItemDescription>{x.pillars} · Plan {x.plan}</ItemDescription></ItemContent></Item>)}</ItemGroup>
        ) : <Muted>Library list for this section.</Muted>}
      </div>
      <div className="flex min-h-0 flex-col gap-3 overflow-auto p-4">
        {aNav === "runs" || aNav === "needs" ? <><RunHeader run={r} /><AttentionAlerts run={r} /><RunTabs run={r} /></>
          : aNav === "recipes" ? <><H1>{rec.name}</H1><Muted>{rec.desc}</Muted><RunsTable runs={runs.filter((x) => x.recipe === rec.id)} /></>
          : aNav === "data" ? <DatasetsScreen /> : aNav === "sessions" ? <SessionsScreen /> : <PlansScreen />}
      </div>
    </div>
  )
}

// ---------- B: Inbox (decisions that need a person are the root) ----------
function VariantB() {
  const { items, item, bTab, set, runs, run, runById, openRun } = useStore()
  const sel = items.find((i) => i.id === item) ?? items[0]
  const r = runById(run)
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 p-4">
      <Tabs value={bTab} onValueChange={(v) => set({ bTab: String(v) })}>
        <TabsList>
          <TabsTrigger value="needs">Needs you · {items.length}</TabsTrigger>
          <TabsTrigger value="activity">Activity</TabsTrigger>
          <TabsTrigger value="library">Library</TabsTrigger>
        </TabsList>
        <TabsContent value="needs">
          <div className="grid items-start gap-4 lg:grid-cols-[22rem_minmax(0,1fr)]">
            <ItemGroup>{items.map((i) => (
              <Item key={i.id} size="sm" variant={sel?.id === i.id ? "muted" : "outline"} render={<button type="button" onClick={() => set({ item: i.id })} />}>
                <ItemContent><ItemDescription>{i.kind}{i.urgent ? " · waiting now" : ""}</ItemDescription><ItemTitle>{i.title}</ItemTitle><ItemDescription>{i.sub}</ItemDescription></ItemContent>
              </Item>))}</ItemGroup>
            {sel ? <Decision item={sel} /> : <div className="flex flex-col gap-2"><H1>All clear</H1><Muted>Runs keep going on their own.</Muted></div>}
          </div>
        </TabsContent>
        <TabsContent value="activity">
          <div className="grid items-start gap-4 lg:grid-cols-[22rem_minmax(0,1fr)]">
            <ItemGroup>{runs.map((x) => (
              <Item key={x.id} size="sm" variant={x.id === run ? "muted" : "default"} render={<button type="button" onClick={() => openRun(x.id)} />}>
                <ItemContent><ItemTitle>{runName(x)}</ItemTitle><ItemDescription>{x.id}</ItemDescription><StateBadges run={x} /></ItemContent>
              </Item>))}</ItemGroup>
            <div className="flex flex-col gap-3"><RunHeader run={r} /><AttentionAlerts run={r} /><RunTabs run={r} /></div>
          </div>
        </TabsContent>
        <TabsContent value="library"><RecipesScreen /></TabsContent>
      </Tabs>
    </div>
  )
}

// ---------- C: Library (Recipes are the root) ----------
function VariantC() {
  const { recipe, set, runs, items, cTab, run, runById, openRun } = useStore()
  if (recipe) {
    const x = recipe === "_adhoc" ? null : recipeOf(recipe)
    const rs = runs.filter((r) => (x ? r.recipe === x.id : !r.recipe))
    const its = items.filter((i) => rs.some((r) => r.id === i.run))
    const sel = rs.find((r) => r.id === run) ?? rs[0]
    return (
      <div className="flex flex-col gap-3 p-4">
        <Breadcrumb><BreadcrumbList><BreadcrumbItem><BreadcrumbLink render={<button type="button" onClick={() => set({ recipe: null })} />}>Library</BreadcrumbLink></BreadcrumbItem><BreadcrumbSeparator /><BreadcrumbItem><BreadcrumbPage>{x ? x.name : "Runs without a Recipe"}</BreadcrumbPage></BreadcrumbItem></BreadcrumbList></Breadcrumb>
        <div><H1>{x ? x.name : "Runs without a Recipe"}</H1><Muted>{x ? `${x.desc} · ${x.pillars}` : "Started by SDK scripts, the CLI or agents over MCP without a Recipe."}</Muted></div>
        <Tabs value={cTab} onValueChange={(v) => set({ cTab: String(v) })}>
          <TabsList variant="line"><TabsTrigger value="overview">Overview</TabsTrigger><TabsTrigger value="runs">Runs · {rs.length}</TabsTrigger><TabsTrigger value="plan">Plan & Repairs</TabsTrigger><TabsTrigger value="data">Data & changes</TabsTrigger></TabsList>
          <TabsContent value="overview"><div className="flex flex-col gap-3">{its.map((i) => <ItemRow key={i.id} item={i} />)}{rs[0] && <><RunHeader run={rs[0]} /><RunTabs run={rs[0]} /></>}</div></TabsContent>
          <TabsContent value="runs"><div className="flex flex-col gap-3"><ItemGroup>{rs.map((q) => <Item key={q.id} size="sm" variant={q.id === sel?.id ? "muted" : "default"} render={<button type="button" onClick={() => openRun(q.id)} />}><ItemContent><ItemTitle>{q.id} · {q.started}</ItemTitle><StateBadges run={q} /></ItemContent></Item>)}</ItemGroup>{sel && <><RunHeader run={runById(sel.id)} /><AttentionAlerts run={sel} /><RunTabs run={sel} /></>}</div></TabsContent>
          <TabsContent value="plan"><PlansScreen /></TabsContent>
          <TabsContent value="data"><DatasetsScreen /></TabsContent>
        </Tabs>
      </div>
    )
  }
  return (
    <div className="flex flex-col gap-3 p-4">
      <H1>Library</H1><Muted>Each Recipe with its runs, Plan, data and Sessions.</Muted>
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
        {[...RECIPES.map((r) => r.id), null].map((id) => {
          const x = recipeOf(id)
          const rs = runs.filter((r) => (x ? r.recipe === x.id : !r.recipe))
          const n = items.filter((i) => rs.some((r) => r.id === i.run)).length
          return (
            <Card key={id ?? "_adhoc"} size="sm">
              <CardHeader><CardTitle>{x ? x.name : "Runs without a Recipe"}</CardTitle><CardDescription>{x ? `${x.pillars} · Plan ${x.plan}` : "SDK scripts, CLI, agents over MCP"}</CardDescription>{n > 0 && <CardAction><Badge variant="outline">{n} need you</Badge></CardAction>}</CardHeader>
              <CardContent><div className="flex flex-col gap-2">{rs[0] && <StateBadges run={rs[0]} />}<Muted>{rs.length} runs</Muted><div><Button size="xs" variant="outline" onClick={() => set({ recipe: id ?? "_adhoc", cTab: "overview" })}>Open</Button></div></div></CardContent>
            </Card>
          )
        })}
      </div>
    </div>
  )
}

// ---------- D: Live (running Sessions and the event stream are the root) ----------
function VariantD() {
  const { items, dBacklog, set } = useStore()
  return (
    <div className="flex flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center gap-2"><Muted>{items.filter((i) => !i.urgent).length} finished runs need you</Muted><Button size="xs" variant="outline" onClick={() => set({ dBacklog: !dBacklog })}>{dBacklog ? "Hide" : "Show"}</Button></div>
      {dBacklog && <ItemGroup>{items.filter((i) => !i.urgent).map((i) => <ItemRow key={i.id} item={i} />)}</ItemGroup>}
      <LiveScreen />
    </div>
  )
}

// ---------- E, F, G: side menu with every screen ----------
const MENU: [Route, string][] = [["home", "Home"], ["needs", "Needs you"], ["runs", "Runs"], ["live", "Live"], ["recipes", "Recipes"], ["datasets", "Datasets"], ["sessions", "Sessions & identities"], ["plans", "Plans & Repairs"], ["settings", "Engine & settings"]]
function SideMenuVariant() {
  const { route, go, items, runs, variant } = useStore()
  const nav = route === "run" ? (variant === "G" ? "home" : "runs") : route === "recipe" ? "recipes" : route
  const menu = variant === "G" ? MENU.filter(([k]) => k !== "needs") : MENU
  const count: Partial<Record<Route, number>> = { needs: items.length, runs: runs.length, live: runs.filter((r) => r.life === "running").length, recipes: RECIPES.length, home: variant === "G" ? items.length : undefined }
  const screens: Record<Route, () => ReactNode> = {
    home: () => (variant === "G" ? <HomeG /> : <HomeScreen />), needs: NeedsScreen, runs: RunsScreen, run: () => (variant === "G" ? <RunPageG /> : <RunPage />), live: () => <LiveScreen />,
    recipes: RecipesScreen, recipe: RecipePage, datasets: DatasetsScreen, sessions: SessionsScreen, plans: PlansScreen, settings: SettingsScreen,
  }
  return (
    <SidebarProvider>
      <Sidebar>
        <SidebarHeader><span className="px-2 text-sm font-semibold">Automan</span></SidebarHeader>
        <SidebarContent>
          <SidebarGroup><SidebarGroupContent><SidebarMenu>
            {menu.map(([k, l]) => (
              <SidebarMenuItem key={k}>
                <SidebarMenuButton isActive={nav === k} onClick={() => go(k)}>{l}</SidebarMenuButton>
                {count[k] !== undefined && <SidebarMenuBadge>{count[k]}</SidebarMenuBadge>}
              </SidebarMenuItem>
            ))}
          </SidebarMenu></SidebarGroupContent></SidebarGroup>
        </SidebarContent>
      </Sidebar>
      <SidebarInset>
        <TopBar />
        <div className="flex flex-col gap-3 p-4">{screens[route]()}</div>
      </SidebarInset>
    </SidebarProvider>
  )
}

// ---------- one drawer host for every variant (Base UI Drawer, from the right) ----------
const title = (d: DrawerEntry) =>
  d.type === "run" ? `Run ${d.id}` : d.type === "session" ? `Session ${d.id}` : d.type === "item" ? "Needs you" : d.type === "activity" ? "Activity" : d.type === "create" ? "New" : d.type === "code" ? "Same run from code" : "Engine & settings"

function DrawerBody({ d }: { d: DrawerEntry }) {
  const { items, runById, variant, set } = useStore()
  if (d.type === "item") {
    const it = items.find((i) => i.id === d.id)
    return it ? <Decision item={it} /> : <div className="flex flex-col gap-2"><H1>Answered</H1><Muted>This item no longer needs you. The answer is in the Audit log.</Muted></div>
  }
  if (d.type === "session") return <SessionDetail id={d.id} replay={variant === "G"} />
  if (d.type === "run") {
    // The Mira drawer is 24rem wide: a run shows as a summary here, with its full page one click away.
    const r = runById(d.id)
    return (
      <div className="flex flex-col gap-3">
        <RunHeader run={r} commands={false} />
        <AttentionAlerts run={r} />
        {r.wait && r.sessions[0].where.includes("Chromium") && <LiveView run={r} interactive={false} />}
        <ItemGroup>{r.steps.map((s, j) => <Item key={j} size="xs" variant="outline"><ItemContent><ItemTitle>{s.op}</ItemTitle><ItemDescription>{s.dur}</ItemDescription></ItemContent></Item>)}</ItemGroup>
        <div className="flex flex-wrap gap-2">
          {r.sessions.map((s) => <Button key={s.id} size="sm" variant="outline" onClick={() => set((p) => ({ drawers: [...p.drawers, { type: "session", id: s.id }] }))}>Session {s.id}</Button>)}
        </div>
        <div><Button onClick={() => set({ run: r.id, route: "run", drawers: [], tab: "timeline", step: -1 })}>Open as page</Button></div>
      </div>
    )
  }
  if (d.type === "activity") {
    return (
      <Tabs defaultValue="needs">
        <TabsList variant="line"><TabsTrigger value="needs">Needs you · {items.length}</TabsTrigger><TabsTrigger value="events">Events</TabsTrigger></TabsList>
        <TabsContent value="needs"><ItemGroup>{items.map((i) => <ItemRow key={i.id} item={i} />)}</ItemGroup></TabsContent>
        <TabsContent value="events"><EventStream /></TabsContent>
      </Tabs>
    )
  }
  if (d.type === "create") return <CreateFlow />
  if (d.type === "code") return <CodeView />
  return <SettingsBody />
}

function DrawerHost() {
  const { drawers, closeDrawers, set } = useStore()
  const top = drawers[drawers.length - 1]
  return (
    <Drawer swipeDirection="right" open={drawers.length > 0} onOpenChange={(o) => !o && closeDrawers()}>
      <DrawerContent>
        {top && (
          <>
            <DrawerHeader>
              <DrawerTitle>{title(top)}</DrawerTitle>
              <DrawerDescription>
                {drawers.length > 1 ? drawers.map((d, i) => (
                  <span key={i}>{i > 0 && " › "}<Button variant="link" size="xs" onClick={() => set({ drawers: drawers.slice(0, i + 1) })}>{title(d)}</Button></span>
                )) : "Opens over the page; Escape or swipe right closes it."}
              </DrawerDescription>
            </DrawerHeader>
            <div className="min-h-0 flex-1 overflow-y-auto p-4"><DrawerBody d={top} /></div>
          </>
        )}
      </DrawerContent>
    </Drawer>
  )
}

// ---------- prototype switcher: deliberately not part of the evaluated design ----------
function Switcher() {
  const { variant, setVariant } = useStore()
  const keys = Object.keys(VARIANTS) as VariantKey[]
  const i = keys.indexOf(variant)
  const step = (d: number) => setVariant(keys[(i + d + keys.length) % keys.length])
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = document.activeElement
      if (el && el.matches("input,textarea,select,[contenteditable]")) return
      if (e.key === "ArrowRight") step(1)
      if (e.key === "ArrowLeft") step(-1)
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  })
  return (
    <div className="fixed bottom-4 left-1/2 z-40 flex -translate-x-1/2 items-center gap-2 rounded-full border bg-popover px-2 py-1 shadow-lg">
      <Button variant="ghost" size="icon-sm" aria-label="Previous variant" onClick={() => step(-1)}>←</Button>
      <span className="min-w-64 text-center text-xs font-medium">{variant} · {VARIANTS[variant]}</span>
      <Button variant="ghost" size="icon-sm" aria-label="Next variant" onClick={() => step(1)}>→</Button>
    </div>
  )
}

export function Console() {
  const { variant } = useStore()
  const body = variant === "A" ? <VariantA /> : variant === "B" ? <VariantB /> : variant === "C" ? <VariantC /> : variant === "D" ? <VariantD /> : null
  return (
    <>
      {body ? <div className="flex h-svh flex-col"><TopBar /><div className="flex min-h-0 flex-1 flex-col overflow-auto pb-16">{body}</div></div> : <SideMenuVariant />}
      <DrawerHost />
      <Switcher />
    </>
  )
}
