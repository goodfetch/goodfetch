# Résultats du lab

**Depuis la réorientation du 8 octobre 2026**, la référence est le [ledger des résultats](../ledger/results-ledger.json), généré par `pnpm lab import`. Il donne pour chaque run son rôle, sa classe de validité (A contrat, B relatif même hôte, C absolu), ses décisions et son besoin de rerun. Les verdicts et rapports chiffrés seront générés depuis les données (phase P4). Les scripts `analyze-integrated.mjs`, `audit-integrated.py`, `render-integrated.py` et `update-integrated-index.py` sont **legacy** : conservés pour reproduire les rapports du 7 et du 8 octobre, mais ils contiennent des comptes et des formulations écrits à la main. Ils ne servent plus de base à une décision.

Lire d’abord les [résultats du workflow durable et du SQLite commun, 8 octobre](workflow-2026-10-08.md), puis le [rapport de qualification et d'architecture](qualification-2026-10-07.md), puis l'[index HTML des expériences](index.html).

`evidence/` conserve une sélection datée de rapports, manifests, résumés et tentatives brutes. Ces copies documentent des expériences passées ; elles ne sont pas régénérées silencieusement lorsqu'un adapter change. Les fichiers SQLite/JSON/JSONL servant de sinks restent dans les dossiers uniques sous `lab/results/`, dont les liens locaux sont donnés par l'index.

Les premières expériences de construction restent dans `lab/results/`. Certaines ont détecté des bugs du lab ou précédé l'uniformisation des viewports, des imports et de la configuration. Elles ne sont pas retenues pour conclure sur les performances finales.

Les données contrôlées ne sont pas un corpus représentatif de tous les sites. Les résultats restent exploratoires, les scopes d'acquittement/durabilité et les cycles de vie doivent être lus avec les chiffres. Répéter sur une machine réservée et un corpus indépendant avant toute publication comparative ou adoption d'architecture.

`evidence-2026-10-08/` conserve cinq nouvelles campagnes avec sources figées et son propre catalogue SHA-256 ; l’archive `evidence/` du 7 octobre est conservée intacte.
