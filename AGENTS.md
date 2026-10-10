# Automan

Web scraping and automation framework project. The whole R&D phase (pre-test proposals, plan, proposed ADRs, sourced research, comparative lab and final report) lives in `rnd/`: start with `rnd/README.md`. The executable lab is `rnd/lab/` (see `rnd/lab/README.md`); its commands are run from that folder. `docs/` is reserved for the product's final documentation.

## Agent skills

### Issue tracker

Issues and specs live in the GitHub Issues of `Jbmanllr/automan`, via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical roles, with the default labels (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: the R&D vocabulary is in `rnd/GLOSSARY.md`; the final `GLOSSARY.md` will be created at the repo root when terms are adopted. Final ADRs are in `docs/adr/`; ADRs proposed during R&D are in `rnd/adr/`. See `docs/agents/domain.md`.
