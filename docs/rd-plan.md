# Plan de R&D pré-PRD : lab piloté par décisions

**Statut : approuvé par le porteur le 8 octobre 2026.** Ce plan réoriente le [lab existant](../lab/README.md). Il ne le reconstruit pas. Il produit quatre sorties : une stack arrêtée par décisions mesurées, des affirmations de performance prouvables, un positionnement marché, puis un PRD v2. Les paramètres ci-dessous sont un programme. Aucun résultat n'en découle encore.

## 1. Principe

Chaque expérience tranche une **décision** du registre `lab/decisions/`. Une décision déclare à l'avance :

- ses options et sa baseline ;
- sa métrique principale ;
- ses gates de contrat, qui doivent être à 100 % avant tout verdict de performance ;
- le plus petit effet d'intérêt Δ, la règle de décision et la classe d'hôte requise.

Le verdict est calculé, jamais écrit à la main. Les affirmations publiques candidates (`lab/claims/`) sont déclarées avant les mesures.

Les classes d'hôte sont `local-dev`, `local-controlled` et `reference` (Linux bare metal). Tout est exécutable en local. L'hôte de référence est reporté, et ses reruns arrivent en dernier. Une métrique non mesurable sur un hôte est déclarée `unavailable`, jamais approchée.

## 2. Refonte du harnais (préalable à toute nouvelle campagne)

| Phase | Contenu | Acceptation |
| --- | --- | --- |
| P0 | Dépôt `Jbmanllr/automan`, skills configurés, ADR dans `docs/adr/` | Fait le 8 octobre 2026 |
| P1 | Registre de décisions et de claims, catalogue de métriques, schémas (`lab/schemas/`), import des 43 runs existants en `run.v2`, ledger avec rôle, classe de validité A/B/C et besoin de rerun | Fait le 8 octobre 2026 ([#12](https://github.com/Jbmanllr/automan/pull/12)) : import idempotent, originaux inchangés, intégrité vérifiée contre les catalogues SHA, `pnpm lab validate` vert |
| P2 | Moteur unique : provenance, preflight et postflight d'hôte, gate de conformité, warmup, blocs, finalisation. Codes d'erreur typés, isolation des pannes (`aborted` ≠ échec), oracle hors du temps mesuré, disponibilité des métriques | Fait le 8 octobre 2026 ([#13](https://github.com/Jbmanllr/automan/pull/13)) : parité des comptes avec v1 ; un adapter bloqué ne fait plus échouer le bloc ; un hôte bruité est refusé |
| P3 | Les cinq autres lanes portées sur le moteur, toutes les matrices en données | Comptes historiques reproduits (30, 42, 162, 108, 60, 24) |
| P4 | Statistiques appariées par bloc (log-ratio, bootstrap, Hodges–Lehmann, Wilcoxon, TOST), taille d'échantillon calculée, verdicts mécaniques, rapports générés depuis les données uniquement | Tests golden ; aucun chiffre écrit à la main |
| P5 | Réseau (toxiproxy, dnctl en smoke sur macOS, netem pour la référence), calibration RTT, sondes Linux, `pnpm lab ci` | Calibration dans la tolérance ; contrats verts en local |

## 3. Campagnes de décision

Les 16 campagnes proposées le 8 octobre sont reprises ici, rattachées chacune à une décision. Le n° renvoie à cette liste d'origine. L'ordre suit la valeur pour la décision, puis les dépendances.

| Ordre | N° | Campagne | Décision | Phase / hôte | Adaptation au lab refactoré |
| --- | --- | --- | --- | --- | --- |
| 1 | 4 | Pilote Rust/CDP contre référence Node/CDP directe, Playwright, Stagehand, Stagehand batch, `@crawlee/stagehand`. Navigation, résolution, clic, saisie, extraction, seuls puis en séquences ; commandes CDP, latence, CPU et mémoire navigateur compris | D-browser-driver | P6, local-controlled | La référence Node/CDP isole le langage du protocole. La garde Automan est appliquée aux deux pilotes, et le nombre de commandes CDP devient une métrique du catalogue |
| 2 | 5 | Primitives complètes (hover, double clic, clic droit, drag-and-drop, scroll, clavier, `contenteditable`, Unicode/IME), cible remplacée ou déplacée entre la dernière vérification et le clic, annulation en séquence | D-browser-driver | P6 | Ce sont les **gates de contrat** de la décision : aucun verdict de vitesse sans elles. Capacités absentes déclarées `unsupported` |
| 3 | 7 | RTT 0/20/100/300 ms, SDK local puis distant, séquences de 1/10/50 actions, batchées ou non ; ordre, effets partiels, erreurs | D-browser-driver, D-agent-interface | P5 + P6 | Ces valeurs de RTT remplacent les profils 0/40/240 du plan initial. Lien `driver-browser` |
| 4 | 2 | Capacité du workflow HTTP complet : Node natif, Rust natif, Spider et Crawlee intégrés, C1/4/16/64. Trois profils : petite réponse **sans délai**, pagination avec délai, grandes réponses | D-core-language, D-http-transport | P6, local-controlled | Le profil sans délai est indispensable : le délai serveur actuel peut masquer l'écart de calcul. La capacité se mesure en charge ouverte, et le balayage C1–C64 est libellé « charge fermée » |
| 5 | 8 | Transports et incidents HTTP : TLS, HTTP/2, réutilisation de connexions, proxies, compression, redirections publiques contrôlées, boucles ; 429/503, Retry-After, réponse tronquée, coupure, timeout, serveur lent | D-http-transport, D-identity | P6 | S'ajoutent les tests de l'[ADR-002](./adr/0002-access-identity-stealth.md) : cohérence d'empreinte HTTP → navigateur, Web Bot Auth, robots/Content-Signal/TDMRep, challenge servi en `200`. Les contrôles SSRF sont conservés : le refus loopback de Spider n'est jamais contourné |
| 6 | 3 | Parsing représentatif : CSS, HTML malformé, Unicode, champs absents, doublons, gros DOM, validation de schéma ; acquisition, parsing, validation, puis pipeline complet ; streaming 1/100/1 000 MiB avec oracle borné en mémoire | D-extraction, D-core-language | P6 (T0) puis P7 (T1) | Complété par les pages réelles figées (Web Page Replay) et la qualité d'extraction (F1 WCXB par type de page) |
| 7 | 6 | Navigation, frames et fichiers : SPA/hydration, back/forward, popup, nouvelle target, navigation interrompue, long polling, service worker ; iframes same-origin et cross-origin, shadow DOM et slots, upload/download, screenshots, PDF, snapshots DOM/AX | D-browser-driver (gates) | P6 | Extension de T0 au segment portails B2B. Les mêmes fixtures servent aux exemples de la documentation |
| 8 | 9 | Admission continue du pipeline complet : charge croissante, pics, writer ralenti ; latence depuis l'admission ; queue bornée, backpressure, rejet explicite, drainage, aucune perte parmi les jobs acceptés | D-durable-runtime | P6, local-controlled | Nouvelle décision séparée de D-storage |
| 9 | 10 | Ordonnanceur, leases, multi-domaines : domaines rapides et lents, priorités, limites globales et par domaine ; heartbeat, renouvellement, expiration pendant une acquisition longue, ancien worker en retard | D-durable-runtime | P6 | Gates : équité, absence de famine, commit à token périmé rejeté |
| 10 | 11 | Injections de panne étendues : arrêts aléatoires (acquisition, parsing, coordinateur, writer, navigateur), disque plein, erreurs d'écriture, verrou prolongé, données corrompues ; arrêt brutal d'une VM | D-durable-runtime, D-storage | Local ; VM reset en local-controlled (VM locale) ou en référence | Chaque contrat déclare ce qui doit survivre. Le hard reset de VM est le seul test de durabilité au-delà d'un crash de processus |
| 11 | 14 | Workflow navigateur durable et replay de plans : navigation/actions → extraction → validation → commit → ACK ; découverte et compilation, rejeu sans LLM, changement de texte ou de structure, réparation ; effet externe à réponse perdue | D-ai-layer, D-durable-runtime | P8 | Ajout : suite de **dérive silencieuse** (mauvaise cible encore cliquable, même mise en page avec un prix changé). L'état `effect_unknown` est vérifié, et un rejeu n'est jamais supposé sûr |
| 12 | 15 | Capacités agentiques : Stagehand IA, Browser Use, agent-browser, Skyvern, Crawl4AI… Même corpus réservé, modèle, outils et budgets ; découverte, extraction, multi-étapes, replay, réparation ; tokens, appels, coût, durée, cache froid ou chaud ; injections malveillantes dans les pages | D-ai-layer, D-agent-interface | P8 | Faux provider d'abord (oracle des tokens), puis 2 à 3 modèles réels avec plafond de coût. pass@1 et pass^k. Ajout de l'arène des interfaces : MCP, CLI, SDK, code mode |
| 13 | 12 | JSON/JSONL/SQLite décomposés : encodage, transport, écriture, synchronisation ; batches 1 à 10 000 ; records 128 o/4 KiB/64 KiB ; corpus hors cache ; index, upsert, contraintes, readers prolongés ; JSONL shardé et compressé | D-storage | P6, basse priorité | D-storage reste `decided` pour sa base (writer unique, lots, WAL FULL). Cette campagne affine les réglages seulement si les usages l'exigent |
| 14 | 13 | Autres sinks : PostgreSQL/COPY, Parquet/Arrow, DuckDB, stockage objet ; batch, flush, reprise, multipart interrompu, idempotence | D-sinks | Après la piste marché | Ouverte seulement pour les sinks demandés par les clients (D-feature-priority) |
| 15 | 16 | Sessions, isolation et endurance : cookies/login/storage, contextes réutilisés, plusieurs tenants avec données témoins ; aucune fuite ; 8 h de charge mixte avec recyclage et incidents | D-identity, D-cloud-topology, D-durable-runtime | Isolation en P6 ; endurance en local-controlled (nuit) puis en référence | L'endurance surveille mémoire, descripteurs, processus orphelins, WAL, files et débit |
| 16 | 1 | Confirmation sur machine dédiée : comparaisons Node/Rust, Spider et navigateur sur Linux ; mêmes versions, Chromium et limites CPU/RAM ; ordre randomisé et apparié par bloc | Toutes les décisions techniques | P9, reference | Condition de publication de toute affirmation chiffrée |

Décisions ajoutées par cette intégration : **D-durable-runtime** (admission, ordonnancement, leases, reprise) et **D-sinks** (formats et destinations de sortie).

## 4. Résultats existants

Les 43 runs déjà exécutés sont importés sans modification et classés dans `lab/ledger/` :

- **A, contrat** : reste acquis tant que les versions ne changent pas.
- **B, comparaison relative sur le même hôte** : sert d'orientation et de pilote pour la taille d'échantillon.
- **C, chiffre absolu** : non publiable sans rerun de référence.

Les runs superseded ou diagnostiques gardent leur raison. À relancer en priorité : runtime Node/Rust (`789d5e`), backends HTTP intégrés (`2b2b75`, hôte bruité) et timings navigateur (`b21a28`), absorbés par les campagnes 1, 3, 4 et 16 ci-dessus.

## 5. Piste « demande marché » (parallèle)

- **Workflow multi-agents** :
  - collecte GitHub, Reddit, HN, avis et rapports 2026 ;
  - déduplication et filtrage des contenus promotionnels ;
  - codage par taxonomie, puis second codage indépendant et vérification des sources ;
  - synthèse dans `research/market/`.
- **Entretiens JTBD (« switch »)** menés par le porteur : 10 à 15. Le guide et le questionnaire de prix sont fournis.
- **Alimente** D-segment, D-pricing et D-feature-priority.

## 6. Sorties de la phase

- Un ADR par décision tranchée, généré depuis son verdict, dans `docs/adr/`.
- Affirmations retenues, avec leurs preuves et leur classe d'hôte.
- Synthèse marché.
- PRD v2, puis tickets GitHub.

Le détail des fichiers et des tâches de chaque phase est suivi dans les issues GitHub du dépôt, une par phase.
