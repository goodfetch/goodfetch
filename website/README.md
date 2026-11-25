# Docs site

Goodfetch's docs site at `goodfetch.dev/docs` (ADRs 0030 and 0097): Docusaurus over the source tree that `xtask docs-site render` writes from `docs/` under `target/docs-site/source/`.

- `pnpm --filter @goodfetch/website build` renders the tree, then builds the site into `target/docs-site/build/`; `start` serves it while you edit.
- One docs instance holds the root (the chooser, from `docs/root/`; the release notes and the benchmark results), `collie/` and `foxhound/`. Each page of `docs/` is rendered into the section of each Product that carries it, with that Product's names. Foxhound's sidebar puts Collie's pages under the heading "Goodfetch Collie", and each says it is Collie's.
- `versions.json`, which the release's version cut writes, lists the versions built: stable at the root, `next` from `main`, and the last minor of each previous major that is still supported. `next` and the previous majors carry a banner.
- The header's switcher leads to the same page in the other Product's section, or to its home when that section lacks it.
- The build fails on a broken link, anchor or image, a duplicate route, a fenced block with no language and a directive no plugin renders.
- After the build, `src/llms-plugin.ts` writes each page's `.md` twin at its URL with `.md` (`/docs/collie/install.md`; the root's home at `/docs/index.md`), and in each kept version each section's `llms.txt` and `llms-full.txt` and the root's pair. A twin's links to other pages point at their twins from the site's root, so they read the same in `llms-full.txt`. The build then fails when a page lacks its twin, its entry in its own section's `llms.txt` (the root's for a root page, the accessibility statement included) or its text in the matching `llms-full.txt`.

## Links across sections

A page links another page of `docs/` by its file. On the site, a link the section can follow stays in the section; one to a page only the other section carries points at that section's rendering, and `foxhound/runs.md` names Foxhound's rendering of `runs.md`. Each is written as a relative file link, which Docusaurus resolves in the version being read.

An executable's embedded docs (ADR 0097) carry their own section only, so each link that leaves it is written as an absolute link to the site's stable docs, `https://goodfetch.dev/docs/<section>/<page>`, or `https://goodfetch.dev/docs/<page>` for a root page. Stable is the one version the site always keeps, where the version an executable was released with may be gone.
