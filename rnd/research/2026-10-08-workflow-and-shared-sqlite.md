# Workflow durable et comparaison avec un SQLite commun

Relevé et essais locaux du 8 octobre 2026, heure de Paris. Les résultats exécutés sont consolidés dans le [rapport du lab](../lab/reports/workflow-2026-10-08.md). Cette note explique les contrôles, sans attribuer un avantage au seul langage.

## Contrôler le moteur effectivement chargé

Comparer Node `DatabaseSync` et Rust SQLx ne contrôle pas automatiquement la version, les options de compilation, les transactions ni les allocations. La nouvelle piste construit **une bibliothèque SQLite 3.53.4 unique**, avec un shim C commun, puis trois clients synchrones : Node N-API, Rust FFI et Go/cgo. Chaque insertion fait un appel au même shim ; chaque connexion réutilise son statement. L'API C SQLite expose ces opérations ; Node-API et cgo fournissent les frontières avec les runtimes. [API C SQLite](https://www.sqlite.org/c3ref/intro.html), [Node-API 25.5.0](https://nodejs.org/download/release/v25.5.0/docs/api/n-api.html), [cgo](https://pkg.go.dev/cmd/cgo).

L'archive officielle est épinglée par SHA-256. Son `sqlite3.c` est également vérifié contre le SHA3-256 de la release officielle. Les sources du shim/bindings, toolchains, flags et artefacts compilés sont consignés. Le runner vérifie le chemin réellement chargé par `dladdr`, le SHA du fichier, le source ID, les options et les PRAGMAs effectifs. Les symboles internes SQLite sont masqués pour éviter l'interposition avec le SQLite incorporé à Node. [Release SQLite 3.53.4 et hashes officiels](https://www.sqlite.org/releaselog/3_53_4.html).

Le premier témoin utilisait 3.46.0, comme la piste SQLx historique. Une nouvelle vérification de la documentation a identifié le WAL-reset bug, corrigé à partir de 3.51.3. Le témoin est conservé ; la comparaison principale est répétée sur 3.53.4. Le cas documenté concerne plusieurs connexions écrivant/checkpointant simultanément. Des tests locaux réussis sur l'ancienne version ne constituent donc pas une qualification de ce correctif. [SQLite : WAL-reset bug](https://www.sqlite.org/wal.html#the_wal_reset_bug).

## Ce que les timings comparent encore

Les trois clients fabriquent les mêmes records, encodent du JSON puis insèrent dans SQLite. Les encodeurs, représentations des records, allocations et transitions FFI diffèrent. L'expérience compare donc trois implémentations de ce pipeline d'écriture sous un moteur SQL commun. Elle n'isole pas une propriété intrinsèque de Rust, Node ou Go. Une autre comparaison devra séparer génération/encodage, coût de binding et moteur SQL ; un appel FFI par batch modifierait aussi le workload.

Les processus écrivains ouvrent une même base par cellule. SQLite WAL n'a qu'un écrivain actif à un instant donné ; ajouter des processus peut accroître l'attente de verrou. FULL et NORMAL restent deux contrats distincts. L'autocheckpoint est explicitement fixé à 1 000 pages ; la campagne de gros batches accroît le volume écrit avec ce réglage, sans compter directement chaque checkpoint. [Fonctionnement WAL, concurrence et checkpoint](https://www.sqlite.org/wal.html).

## Prouver le contrat avant de mesurer le workflow

Le prototype HTTP Node relie acquisition/extraction, oracle privé, résultats bornés, batch, transaction de données et états des jobs, puis transaction d'ack. Les tests ciblent les frontières : lease expirée et token remplacé, même job rejoué avec données différentes, faux résultat correct en apparence, rollback atomique, réponse perdue après commit, reprise d'un nouveau processus coordinateur. Les tâches concernent des lectures HTTP idempotentes. Ces tests ne prouvent pas qu'un effet externe est exactement une fois, ni la durabilité après coupure électrique.

Les campagnes utilisent des bases neuves, un ordre randomisé par blocs, des sources figées et un oracle final après réouverture. L'instrumentation, IPC et drain restent visibles dans les durées. Les acks réconciliés comptent dans la latence du même coordinateur ; le redémarrage complet du coordinateur est actuellement une qualification fonctionnelle. Les métriques du [contrat workflow](../lab/docs/workflow.md) précisent leurs dénominateurs et la différence de périmètre RSS entre pistes.

Le prototype intégré utilise Node ; les nouveaux workers Rust/Go constituent uniquement la piste d'écriture. Le coût intégré d'un noyau Rust/Go et celui d'un pilote CDP Rust contre Node co-localisé restent des expériences à mener à garanties égales.

## Réveil du pool dans le workflow

La série intégrée a révélé le coût de l’attente périodique lorsque le writer laisse momentanément la queue du pool vide. L’API publique `AutoscaledPool.notify()` permet une vérification immédiate. Un appel prématuré pendant l’initialisation a été détecté par le test réel et corrigé ; la variante ne notifie qu’après l’entrée dans un premier handler. Une série A/B séparée randomise les modes poll/notify et conserve tous les oracles. Le gain observé à C1 ne se généralise pas à toutes les concurrences. [API Crawlee 3.18](https://crawlee.dev/js/api/core/class/AutoscaledPool).
