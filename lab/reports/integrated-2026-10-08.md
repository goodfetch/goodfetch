# Qualification transversale Automan — 8 octobre 2026

**Rust/Tokio reste le candidat pour le noyau portable.** Node/TypeScript est le témoin expérimental et la voie de compatibilité. Le workflow natif Rust atteint un débit proche de Node avec moins de CPU et de footprint mesurés ; aucune de ces campagnes ne qualifie encore un pilote Chromium/CDP Rust.

Le prototype exécute **acquisition → extraction → validation → queue bornée → writer batché → commit → ACK**, séparément dans Node et Rust. Le writer est un processus dédié. Les deux ledgers chargent la même bibliothèque SQLite 3.53.4 et le même shim SQL C générique, en WAL FULL ; leur logique de jobs est indépendante. L'extraction est spécialisée pour le corpus du lab.

La première suite couvre les cinq domaines demandés, y compris les transports et la saturation. Une seconde suite requalifie les corrections de configuration Rust, stabilité des clics et initialisation SQLite. Les campagnes sont séquentielles ; les données et diagnostics initiaux restent conservés.

[Première suite](../results/qualification-integrated-2026-10-07T23-21-58-974Z-6b64b0/summary.json), [requalification](../results/qualification-current-2026-10-08T00-11-54-553Z-21d1f2/summary.json). Sources/binaires inchangés pendant les campagnes retenues. [Analyse reproductible](integrated-analysis.json), [catalogue SHA-256](integrated-evidence-catalog.json), [audit des sources](integrated-source-audit.json), [audit des contrats](integrated-verification.json).

## Runtime complet : Rust conserve son intérêt

**30/30 cellules vérifiées**, trois blocs randomisés, **47,232 jobs / 1,511,424 records**. C × 192 jobs par cellule, 16 pages × 2 records/job, délai serveur 4 ms/page, C1/4/16, batch 1/4 avec B ≤ C. Fenêtres de travail d'environ quinze secondes. Débit pondéré = jobs exacts / temps travail + finalisation. Le débit complet ajoute startup et oracle externe.

| C | Jobs/batch | Node jobs/s | Rust jobs/s | Node complet/s | Rust complet/s | Node CPU ms/job | Rust CPU ms/job |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 1 | 11,39 | 11,88 | 11,20 | 11,83 | 9,48 | 2,63 |
| 4 | 1 | 47,48 | 48,90 | 46,54 | 48,55 | 4,61 | 1,77 |
| 4 | 4 | 45,29 | 46,94 | 44,41 | 46,57 | 4,18 | 1,44 |
| 16 | 1 | 197,70 | 196,66 | 190,93 | 192,48 | 2,78 | 1,42 |
| 16 | 4 | 189,11 | 195,58 | 182,96 | 191,46 | 2,52 | 1,23 |

À C16/B1 : environ 197 jobs/s pour les deux implémentations ; Rust utilise 1,42 ms CPU/job contre 2,78 pour Node, soit environ 49 % de CPU mesuré en moins. Footprint maximal échantillonné par cellule : Rust 136–178 MiB, Node 314–325 MiB. Cela soutient Rust pour l'efficacité à garanties testées comparables, sans prouver un scraper universellement plus rapide.

La copie de tout le corpus de configuration par producteur Rust a été remplacée par un partage immuable `Arc`. La première série C16/B1 donnait 205,55 Rust / 205,30 Node jobs/s et 204–248 / 306–327 MiB de footprint. Les deux séries ont été réalisées à des moments différents sur un Mac partagé : le changement est réel dans le code, mais cette différence de mémoire et de débit n'est pas un essai causal isolé du seul `Arc`. La première série et son binaire sont préservés. Le batching B4 du workflow complet ne gagne pas systématiquement sur B1 : acquisitions, taille réelle des batches et drain comptent aussi.

[résumé](../results/runtime-native-qualified-2026-10-08T00-11-54-841Z-789d5e/summary.json), [manifest](../results/runtime-native-qualified-2026-10-08T00-11-54-841Z-789d5e/manifest.json)

[Première série native](../results/runtime-native-sustained-2026-10-07T23-21-59-298Z-bf6ecb/summary.json), [binaire Rust précédent](evidence-integrated/artifacts/automan-workflow-rust-before-arc).

## Acquisition HTTP : comparer les cycles de vie et la saturation

**24/24 cellules**, 23,040 jobs / 737,280 records dans le workflow durable. Le runtime est Node lorsqu'un backend externe est utilisé ; les ressources comprennent alors ses descendants. Rust natif est une autre implémentation complète. Un pool partagé et un crawler neuf par tâche ont des coûts différents.

| Runtime/backend | C | Jobs/s pondéré | Min–max par bloc | CPU ms/job |
| --- | --- | --- | --- | --- |
| node/crawlee-http-pooled | 16 | 20,13 | 9,56–49,83 | 43,55 |
| node/crawlee-http-pooled | 4 | 26,17 | 20,44–31,04 | 45,29 |
| node/native | 16 | 154,58 | 109,57–195,40 | 4,45 |
| node/native | 4 | 47,63 | 47,47–47,95 | 5,14 |
| node/spider-http | 16 | 187,89 | 186,79–189,20 | 5,36 |
| node/spider-http | 4 | 37,07 | 26,02–47,07 | 9,22 |
| rust/native | 16 | 142,19 | 90,66–198,77 | 2,02 |
| rust/native | 4 | 49,82 | 49,71–50,05 | 1,55 |

Des écarts importants entre blocs apparaissent, particulièrement pour Crawlee pooled et certaines cellules C16. Le diagnostic du host indique d'autres activités lourdes ; il documente un facteur de confusion possible et ne prouve pas la cause de chaque ralentissement. Aucune pénalité générale de runtime ne peut être déduite de ces seules variations.

[résumé](../results/runtime-http-backends-2026-10-07T23-29-49-072Z-2b2b75/summary.json), [manifest](../results/runtime-http-backends-2026-10-07T23-29-49-072Z-2b2b75/manifest.json)

[Diagnostic de charge](host-load-2026-10-08.json).

Les campagnes open-loop offrent 20/100/250 tâches/s, concurrence 4, trois fenêtres de 12 s, huit pages avec 3 ms de délai. Elles concernent l'acquisition seule, sans ledger durable. Une tâche refusée compte dans le dénominateur ; les tâches admises peuvent finir après la fenêtre. `queueMs` mesure le retard du générateur, sans queue métier cachée.

| Offert/s | Backend | Exactes | Offertes | Rejetées | Exactes/s | p95 succès ms |
| --- | --- | --- | --- | --- | --- | --- |
| 20 | node-http | 719 | 720 | 1 | 20,00 | 40,90 |
| 20 | spider-http | 720 | 720 | 0 | 20,03 | 44,96 |
| 20 | crawlee-http | 563 | 720 | 157 | 15,50 | 453,23 |
| 20 | crawlee-http-pooled | 717 | 720 | 3 | 19,82 | 142,47 |
| 100 | node-http | 3593 | 3600 | 7 | 99,61 | 32,64 |
| 100 | spider-http | 3590 | 3600 | 10 | 99,55 | 33,14 |
| 100 | crawlee-http | 844 | 3600 | 2756 | 23,27 | 215,84 |
| 100 | crawlee-http-pooled | 2433 | 3600 | 1167 | 67,29 | 72,69 |
| 250 | node-http | 4330 | 9000 | 4670 | 120,12 | 32,84 |
| 250 | spider-http | 4331 | 9000 | 4669 | 120,10 | 33,18 |
| 250 | crawlee-http | 982 | 9000 | 8018 | 27,05 | 187,58 |
| 250 | crawlee-http-pooled | 2559 | 9000 | 6441 | 70,80 | 72,76 |

Node fetch et Spider HTTP constituent de bons chemins simples sur ce corpus. Le pool évite certains coûts d'un crawler neuf par tâche, sans effacer ses garanties et frais d'orchestration. Les rejets à 250/s exposent une capacité bornée ; une faible latence sur les seuls succès ne signifie pas que toute la charge a été servie.

[résumé](../results/http-open-20-2026-10-07T23-52-50-976Z-19b9fb/summary.json), [manifest](../results/http-open-20-2026-10-07T23-52-50-976Z-19b9fb/manifest.json)

[résumé](../results/http-open-100-2026-10-07T23-55-27-645Z-9f6596/summary.json), [manifest](../results/http-open-100-2026-10-07T23-55-27-645Z-9f6596/manifest.json)

[résumé](../results/http-open-250-2026-10-07T23-58-01-322Z-66a668/summary.json), [manifest](../results/http-open-250-2026-10-07T23-58-01-322Z-66a668/manifest.json)

Transport : Node/Crawlee neuf/Crawlee pooled vérifient chacun 15 conformités et 45 mesures dans la première série. Spider refuse les trois redirections loopback : sa politique SSRF native bloque les sauts vers une adresse privée, même si le départ local est intentionnel. Ce n'est pas une preuve qu'il ne sait pas suivre les redirections publiques. La politique n'a pas été contournée ; le [source Spider 2.53.9 conservé](evidence-integrated/artifacts/spider-2.53.9-website.rs) contient `is_ssrf_redirect` et `ssrf_limited_policy`. La série séparée normal/gzip/chunked/grande réponse donne **60/60 lignes vérifiées** (45 mesures normales ; les variantes sont des contrôles de conformité).

[résumé](../results/http-transport-2026-10-07T23-52-40-042Z-be9fbc/summary.json), [manifest](../results/http-transport-2026-10-07T23-52-40-042Z-be9fbc/manifest.json)

[résumé](../results/spider-http-transport-supported-2026-10-08T00-26-16-511Z-7ce401/summary.json), [manifest](../results/spider-http-transport-supported-2026-10-08T00-26-16-511Z-7ce401/manifest.json)

## Navigateur : le gain brut de Stagehand exige des garanties explicites

Six backends × neuf variantes × trois blocs. La politique native n'ajoute que l'attente d'initialisation du document. Le guard commun impose unicité, visibilité, activation, stabilité sur deux intervalles entre trois observations RAF espacées d'au moins 16 ms, puis hit-test. L'oracle vérifie cible, ordre, phase et événement trusted. Pour les trois variantes négatives, il exige refus + zéro effet ; une cellule vérifiée n'implique donc pas toujours un clic.

Le guard initial avait quatre faux succès sur cible mobile. Un test public répété les a reproduits ; la correction a passé seize interactions puis la nouvelle matrice. La première tentative limitée à déplacer l'observation dans RAF a encore échoué : son log diagnostique reste conservé.

| Backend | Natif : conformes/27 | Guard corrigé : conformes/27 | Variantes natives non conformes |
| --- | --- | --- | --- |
| stagehand-batch | 3 | 27 | ambiguous, never-uncovered, hidden, moving, never-enabled, overlay, rerender, delayed |
| playwright | 27 | 27 | — |
| crawlee-stagehand | 27 | 27 | — |
| stagehand | 4 | 27 | rerender, hidden, never-uncovered, ambiguous, moving, never-enabled, overlay, delayed |
| crawlee-playwright | 27 | 27 | — |
| puppeteer | 16 | 27 | rerender, overlay, never-uncovered, ambiguous |

| Backend gardé | p50 tâche normale ms | p50 cible mobile ms | p50 startup normal ms |
| --- | --- | --- | --- |
| puppeteer | 180,85 | 340,61 | 505,58 |
| crawlee-playwright | 1 008,81 | 1 288,09 | 519,98 |
| stagehand | 185,36 | 338,87 | 834,62 |
| stagehand-batch | 179,87 | 359,75 | 856,81 |
| playwright | 323,78 | 489,70 | 530,72 |
| crawlee-stagehand | 792,72 | 974,65 | 855,28 |

Le résultat historique de 50 clics (Stagehand 0,95 s / Playwright 2,71 s) utilisait des conditions communes de disponibilité. La nouvelle matrice montre pourquoi il ne faut pas le transformer en remplacement immédiat de Playwright : les primitives natives Stagehand testées échouent souvent sur overlay, activation, mutation et ambiguïté. Playwright, Crawlee/Playwright et l'extension Crawlee/Stagehand v3 passent cette matrice native ; le handler de l'extension utilise une page Playwright attachée par CDP.

Le guard corrigé fournit une autre configuration explicitement testée ; il ne prouve pas toutes les garanties de Playwright ni la suppression de la course entre garde et clic. Les refus et les clics réussis ont des latences distinctes. Stagehand v4 n'expose pas le même timeout de locator : budget des guards/Playwright/Puppeteer 800 ms, deadline extérieure Stagehand 8 s. Les startups sont séparés. Aucun appel LLM n'est effectué ; observe/act/extract et self-heal restent hors mesure.

[résumé](../results/browser-actionability-2026-10-07T23-38-33-163Z-ccad1f/summary.json), [manifest](../results/browser-actionability-2026-10-07T23-38-33-163Z-ccad1f/manifest.json)

[résumé](../results/browser-actionability-2026-10-08T00-21-16-186Z-b21a28/summary.json), [manifest](../results/browser-actionability-2026-10-08T00-21-16-186Z-b21a28/manifest.json)

[Contrat navigateur](../docs/browser-actionability.md), [régression rouge](qualification-guard-red.log), [tentative RAF encore en échec](qualification-guard-green.log), [régression corrigée](qualification-guard-calibrated-green.log).

## SQLite : batches, writers, readers et checkpoints

**108/108 cellules vérifiées**, **3,456,000 records**. Node N-API, Rust FFI synchrone et Go/cgo chargent le même artefact C : W1/4, B100/1000, lecteurs absents/courts/longs, trois blocs. Chaque cellule contient 32 000 lignes de 512 octets, WAL FULL, huit producteurs. Les fenêtres d'écriture sont courtes ; l'oracle relit chaque record et l'intégrité après fermeture.

| Binding | Writers | Batch lignes | Lignes/s écriture | Lignes/s cycle complet | p95 RPC ACK ms |
| --- | --- | --- | --- | --- | --- |
| go-sync | 1 | 100 | 208 926 | 113 184 | 8,61 |
| go-sync | 1 | 1000 | 244 607 | 116 784 | 42,30 |
| go-sync | 4 | 100 | 196 643 | 101 639 | 13,09 |
| go-sync | 4 | 1000 | 242 298 | 117 333 | 127,11 |
| node-shared | 1 | 100 | 166 064 | 82 091 | 13,11 |
| node-shared | 1 | 1000 | 207 894 | 86 986 | 58,52 |
| node-shared | 4 | 100 | 189 052 | 83 242 | 13,43 |
| node-shared | 4 | 1000 | 183 632 | 83 065 | 140,92 |
| rust-sync | 1 | 100 | 197 406 | 108 411 | 8,44 |
| rust-sync | 1 | 1000 | 268 954 | 126 678 | 37,31 |
| rust-sync | 4 | 100 | 178 049 | 96 881 | 20,76 |
| rust-sync | 4 | 1000 | 269 638 | 125 840 | 104,07 |

B1000 augmente le débit W1 dans les trois bindings. W4 n’est pas systématiquement plus lent : Node/B100 gagne dans cette série courte et Rust/B1000 est proche de W1. Les gros batches W4 augmentent toutefois nettement la latence RPC p95, sans gain de débit cohérent ; les résultats historiques et cette série motivent le writer unique comme base, pas une règle universelle.

Les 31 échecs de démarrage de la première matrice Rust/Go W4 surviennent avant insertion. Le schéma et le mode WAL étaient créés simultanément. L'initialisation commune est maintenant effectuée une seule fois avant d'ouvrir les vrais writers concurrents ; le test W4 avec reader long est passé de rouge à vert. Les writers restent indépendants pendant la fenêtre mesurée : la correction ne sérialise pas artificiellement leurs écritures.

Un reader long doit continuer à voir zéro ligne pendant les inserts. PASSIVE laisse alors des frames non checkpointées ; après libération, TRUNCATE réussit avec WAL à zéro. Les lecteurs courts effectuent des transactions de 2 ms toutes les 10 ms, avec compte croissant borné. Tous ces invariants sont contrôlés dans l'audit. Un writer dédié et des transactions batchées restent le point de départ soutenu par ces expériences ; plusieurs writers ne suppriment pas le verrou d'écriture unique de SQLite. [SQLite WAL](https://www.sqlite.org/wal.html).

Les p95 RPC viennent des ACKs bruts. Les mesures CPU/footprint de cette piste incluent le coordinateur déjà vivant, dont les compteurs initiaux sont soustraits ; ses allocations antérieures empêchent de classer la mémoire intrinsèque des langages. Les résultats JSON/JSONL, snapshot durable, NORMAL/FULL et SQLx historiques restent dans les rapports antérieurs, avec leurs contrats et builds séparés.

[résumé](../results/sqlite-readers-2026-10-08T00-25-35-434Z-92d4f3/summary.json), [manifest](../results/sqlite-readers-2026-10-08T00-25-35-434Z-92d4f3/manifest.json)

[Contrat readers/checkpoints](../docs/sqlite-readers.md), [diagnostic des 31 échecs](sqlite-initialization-diagnostics.json), [régression rouge](qualification-writers-red.log), [six cas corrigés](qualification-writers-green.log), [écritures historiques](workflow-2026-10-08.md).

## Durabilité : deux runtimes, mêmes invariants observables

**42/42 cellules vérifiées**, 1,008 jobs / 8,064 records, 42 redémarrages de writer et 12 redémarrages de coordinateur. SIGKILL avant commit, après commit, pendant ACK, après ACK ; nouveau coordinateur après claim ou commit ; trois fautes répétées avec writer ralenti. La série initiale avait également 42/42 cellules vérifiées.

| Runtime/cas | Cellules | Redémarrages writer | Redémarrages coord. | Jobs/s cycle complet | p50 reprise coord. ms |
| --- | --- | --- | --- | --- | --- |
| node/native/4/2/after-ack/0 | 3 | 3 | 0 | 17,22 | — |
| node/native/4/2/after-commit/0 | 3 | 3 | 0 | 17,36 | — |
| node/native/4/2/batch-committed/0 | 3 | 0 | 3 | 17,30 | 282,57 |
| node/native/4/2/before-commit+during-ack+after-ack/20 | 3 | 9 | 0 | 6,78 | — |
| node/native/4/2/before-commit/0 | 3 | 3 | 0 | 17,42 | — |
| node/native/4/2/during-ack/0 | 3 | 3 | 0 | 17,32 | — |
| node/native/4/2/job-claimed/0 | 3 | 0 | 3 | 17,11 | 1 057,86 |
| rust/native/4/2/after-ack/0 | 3 | 3 | 0 | 21,23 | — |
| rust/native/4/2/after-commit/0 | 3 | 3 | 0 | 21,09 | — |
| rust/native/4/2/batch-committed/0 | 3 | 0 | 3 | 20,32 | 59,95 |
| rust/native/4/2/before-commit+during-ack+after-ack/20 | 3 | 9 | 0 | 7,21 | — |
| rust/native/4/2/before-commit/0 | 3 | 3 | 0 | 20,95 | — |
| rust/native/4/2/during-ack/0 | 3 | 3 | 0 | 21,18 | — |
| rust/native/4/2/job-claimed/0 | 3 | 0 | 3 | 20,60 | 1 017,54 |

Les résultats et le receipt sont commités avant ACK, puis ACK est enregistré dans une transaction distincte. Le rejeu d'un job déjà commité réutilise le receipt ; les anciens tokens de lease sont rejetés. L'oracle externe rouvre la base après arrêt et compare chaque record. Les checkpoints au moment du crash attestent un claim sans résultat ou un commit avec résultats avant la reprise.

Pour la reprise de coordinateur, les débits utilisent le temps complet extérieur : le temps de travail du processus repris seul omettrait une partie de l'incident. La queue de résultats reste bornée par C même avec writer lent. Ces essais prouvent les invariants du prototype sur crashes de processus ; ils ne qualifient pas coupure électrique, effet externe exactly-once, heartbeat ou cluster cloud.

[résumé](../results/runtime-recovery-qualified-2026-10-08T00-20-09-515Z-ad3e70/summary.json), [manifest](../results/runtime-recovery-qualified-2026-10-08T00-20-09-515Z-ad3e70/manifest.json)

[Workflow et reproduction](../docs/runtime-workflow.md).

## Vérification, reproductibilité et choix encore ouverts

L’audit post-campagne passe **510 contrôles**, dont effets trusted/refus, bornes, redémarrages, checkpoints et sources figées. Il complète les oracles de records exécutés dans les campagnes.

Les **57 tests de contrat passent**, zéro échec/skip, dont faux succès, protocole/timeout, reprise des deux runtimes, fencing, calibration CPU macOS, clic mobile et startup W4. TypeScript et format Rust passent ; le binaire Rust actuel a été reconstruit avant requalification. [Log des tests](qualification-current-tests.log), [build](qualification-current-build.log).

Les manifests fixent versions, seeds, environnement, SHA des sources et binaires. Les sources sont copiées avant les essais et auditées à nouveau après. Le catalogue référence les preuves originales sans exporter les inputs privés de l'oracle ; les bases et exports complets restent dans les dossiers de résultats. L'analyse agrège les événements bruts pour les quantiles, sans moyenner des p95 de blocs. Les erreurs des comparateurs et les erreurs initiales du lab restent distinguées.

La sonde macOS est calibrée contre getrusage et convertit les compteurs Mach avec `mach_timebase_info`. Elle mesure le coordinateur natif et descendants, exclut fixture/oracle dans la piste native, échantillonne à 100 ms et peut manquer des pics ou fins de processus. Le RSS peut compter des pages partagées plusieurs fois ; footprint et octets I/O sont des indicateurs distincts. La première série aux unités CPU erronées a été interrompue et exclue. Un timer de fermeture de 400 ms a été corrigé avant les campagnes retenues.

Le lab soutient aujourd'hui : poursuivre Rust pour le noyau HTTP durable ; conserver Node pour référence et compatibilité ; préférer un chemin HTTP simple lorsque le rendu n'est pas nécessaire ; garder Playwright comme référence d'interaction robuste et évaluer Stagehand avec ses garanties explicites ; commencer SQLite avec un writer et des transactions batchées. Ce sont des directions de prototype fondées sur ce corpus, pas une architecture finale figée.

Il reste à qualifier un pilote Rust/CDP co-localisé face à Node, Linux dédié, TLS/HTTP2/proxies/429 et retry, parsing général/streaming, corpus holdout, sessions multi-tenant, admission continue/heartbeat, sinks supplémentaires et perception/LLM avec budgets identiques. Les prochaines expériences doivent lever ces incertitudes précises avant toute promesse de performance universelle.

[Matrice de couverture](../docs/coverage-matrix.md), [gates et preuves](../docs/qualification-next.md), [index de toutes les expériences](index.html).
