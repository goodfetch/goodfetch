# Console prototype (throwaway)

Wayfinder ticket: [Prototype: V1 Console screens and flows](https://github.com/Jbmanllr/automan/issues/80). Never merged; lives on the `prototype/console-screens` branch only.

```bash
npm install
npm run dev
```

Open the printed URL and switch variants with the bottom bar, the left and right arrow keys, or `?variant=A`…`G`. Built with shadcn on Base UI, Mira style, default theme, no style overrides at call sites; all data is simulated in memory.

- **A Workbench**: runs first. **B Inbox**: decisions first. **C Library**: Recipes first. **D Live**: running Sessions and events first.
- **E Side menu**: every screen listed; Home switches between Table and Board.
- **F Drawers**: E, plus runs, Sessions and decisions in a right-hand drawer that stacks; Activity (Needs you + events) is an overlay drawer.
- **G Inline and split**: patterns confirmed at competitors: rows expand inline with "Open full page" (Inngest), a split run page with a step inspector (Trigger.dev), a Session drawer with replay and a synced action log (Steel, Browserbase), and one Needs-you list for every kind.

`console-prototype.html` (in `../console-prototype-html/`) is the earlier single-file version (variants A–F, plain CSS; F there still has the docked panel).
