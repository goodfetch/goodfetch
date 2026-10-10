# Automan

Projet de framework de scraping et d'automatisation web. Toute la phase de R&D (propositions d'avant les tests, plan, ADR proposés, recherches sourcées, laboratoire comparatif et rapport final) vit dans `rnd/` : commencer par `rnd/README.md`. Le laboratoire exécutable est `rnd/lab/` (voir `rnd/lab/README.md`) ; ses commandes se lancent depuis ce dossier. `docs/` est réservé à la documentation définitive du produit.

## Agent skills

### Issue tracker

Les issues et specs vivent dans les GitHub Issues de `Jbmanllr/automan`, via le CLI `gh`. See `docs/agents/issue-tracker.md`.

### Triage labels

Les cinq rôles canoniques, avec les libellés par défaut (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context : `GLOSSARY.md` à la racine et ADR définitifs dans `docs/adr/`. Les ADR proposés pendant la R&D sont dans `rnd/adr/`. See `docs/agents/domain.md`.
