# Automan

Projet de moteur de scraping et d'automatisation. Architecture produit proposée dans [docs/proposition-automan.md](docs/proposition-automan.md). La phase en cours est la [R&D pré-PRD pilotée par décisions](docs/rd-plan.md).

Le [lab comparatif opérationnel](lab/README.md) mesure crawl HTTP, interactions navigateur et écriture JSON/JSONL/SQLite avec validation indépendante. Les [recherches sourcées](research/README.md) et la [matrice de qualification](lab/docs/coverage-matrix.md) distinguent capacités installées, mesures locales et expériences futures. Un premier workflow HTTP durable Node et des writers Node/Rust sync/Go avec SQLite commun sont désormais qualifiés dans le [rapport du 8 octobre](lab/reports/workflow-2026-10-08.md). Les noyaux intégrés concurrents et le pilote CDP restent à comparer avant de figer les choix.

Voir les [résultats exécutés et la proposition d'architecture](lab/reports/qualification-2026-10-07.md), avec l'[index des campagnes et des preuves](lab/reports/index.html).

Ce dossier est un dépôt Git dédié depuis le 8 octobre 2026. Les sorties de campagnes (`lab/results/`), dépendances, navigateurs, toolchains et builds natifs ne sont pas versionnés ; les preuves retenues restent suivies dans `lab/reports/`.
