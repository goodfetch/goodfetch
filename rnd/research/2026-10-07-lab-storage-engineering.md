# Lab Automan — stockage JSON, JSONL et SQLite

**7 octobre 2026 — recherche et protocole proposé.** Ce document complète la [méthode générale du lab](2026-10-07-lab-testing-engineering.md). Il ne contient pas de résultats de benchmark. Les choix de matrice et de tailles sont des prescriptions locales proposées, adaptées à la demande de mesurer écriture et concurrence.

## 1. Séparer quatre coûts

Le lab doit mesurer **génération du corpus, sérialisation, écriture et publication/acquittement** séparément, puis leur combinaison dans le pipeline scraper. Une boucle qui construit de nouveaux objets, encode JSON et insère chaque ligne avec son propre commit ne mesure pas seulement la vitesse de SQLite.

| Rail proposé | Entrée | Mesure | Usage de la conclusion |
| --- | --- | --- | --- |
| Sérialisation seule | Objets déterministes en mémoire | Encodage, octets produits, CPU/mémoire | Comparer encoders et structures de données |
| Sink seul | Corpus de bytes préparés | Write/commit/sync et attente de queue | Comparer sinks et politiques de durabilité |
| Sérialisation + sink | Même corpus logique | Temps jusqu'à publication sous contrat | Comparer implémentation utile complète |
| Crawl + extraction + sink | Même graphe fixture | Résultats corrects persistés/seconde | Mesurer backpressure et goulot du système |

Inclure le coût de préparation dans une mesure distincte ; un buffer préconstruit n'est pas gratuit dans le produit. JSON/JSONL et SQLite ne fournissent pas les mêmes fonctions : exposer leurs contrats au lieu de produire un classement global « stockage le plus rapide ».

## 2. SQLite : ce que les sources établissent

WAL permet lecteurs et écrivain simultanés, mais **un seul écrivain actif par base**. Les lecteurs longs peuvent empêcher un checkpoint d'avancer ; WAL nécessite que les processus partagent la même machine et n'est pas conçu pour ce mode sur filesystem réseau. Le checkpoint automatique se déclenche par défaut à 1 000 pages, avec un défaut modifiable au build. [Documentation SQLite WAL](https://www.sqlite.org/wal.html).

En WAL, `synchronous=FULL` ajoute une synchronisation du WAL à chaque commit. `NORMAL` conserve la cohérence mais peut perdre des commits après panne de courant ou crash OS ; SQLite distingue cela du crash applicatif. `integrity_check` vérifie la structure et des contraintes, pas les erreurs de clés étrangères : celles-ci demandent `foreign_key_check`. [Référence des PRAGMAs](https://www.sqlite.org/pragma.html#pragma_synchronous), [contrôle d'intégrité](https://www.sqlite.org/pragma.html#pragma_integrity_check).

`busy_timeout` installe un handler qui attend avant de rendre `SQLITE_BUSY`, et ne transforme pas la base en moteur à plusieurs écrivains actifs. Le handler peut ne pas être appelé si SQLite détecte un risque de deadlock. [Timeout](https://www.sqlite.org/c3ref/busy_timeout.html), [handler et deadlocks](https://www.sqlite.org/c3ref/busy_handler.html).

`BEGIN IMMEDIATE` demande une transaction d'écriture dès le début et peut échouer avec `SQLITE_BUSY` ; `DEFERRED` commence autrement et une conversion lecture→écriture peut échouer. [Transactions SQLite](https://www.sqlite.org/lang_transaction.html).

La FAQ SQLite insiste sur le gain de plusieurs inserts dans une transaction. Elle indique aussi que ses anciens chiffres HDD ne constituent plus des références contemporaines : **reprendre le mécanisme, pas les anciens débits**. [FAQ, question 19 et mise à jour](https://www.sqlite.org/faq.html#q19).

## 3. Node et Rust : éviter les défauts asymétriques

| Backend examiné | Fait vérifié | Conséquence pour le lab |
| --- | --- | --- |
| Node 24.19.0 `node:sqlite` | `DatabaseSync` expose des opérations synchrones ; timeout par défaut `0` ; statements préparés disponibles ; stabilité « release candidate » 1.2 | Configurer timeout et PRAGMAs ; réutiliser la connexion et le statement ; documenter la version exacte |
| SQLx 0.8.6 SQLite | `busy_timeout` par défaut 5 s ; journal mode non fixé ; le commentaire annonce synchronous FULL mais l'initialisation du pragma vaut `None`, donc dépend du SQLite compilé | Utiliser WAL/FULL ou NORMAL et timeout explicites ; vérifier les valeurs effectives |
| SQLx 0.8.6 connection | SQLite bloquant est exécuté via un thread de fond et channels ; fermeture explicite rapporte des erreurs que le simple drop ne remonte pas | Mesurer overhead du driver/thread et attendre la fermeture |

Sources : [Node à la version 24.19.0](https://nodejs.org/download/release/v24.19.0/docs/api/sqlite.html), [options SQLx au tag v0.8.6](https://github.com/transact-rs/sqlx/blob/v0.8.6/sqlx-sqlite/src/options/mod.rs), [connexion SQLx au même tag](https://github.com/transact-rs/sqlx/blob/v0.8.6/sqlx-sqlite/src/connection/mod.rs).

**Prescription locale :** `Promise.all` autour de `DatabaseSync` dans le même thread ne prouve aucune concurrence de writers. Pour le sink intégré au crawler, placer l'écrivain synchrone Node dans un worker ou processus dédié avec queue bornée. Publier séparément cette topologie et une boucle synchrone minimale de sink ; le coût IPC est réel. Côté Rust, un pool de connexions SQLite reste limité par le verrou d'écriture de la base.

Node contre SQLx ne suffit pas à isoler un effet « langage » : versions/builds SQLite, binding synchrone ou thread/channel, statements et topologie diffèrent. Enregistrer `sqlite_version()`, `PRAGMA compile_options`, schéma et options effectives pour expliquer ces variables.

## 4. Matrice de contrats comparables

| Cellule proposée | Sortie / unité de publication | Acquittement mesuré | Atomicité annoncée | Portée |
| --- | --- | --- | --- | --- |
| JSON document buffered | Un tableau complet dans un fichier propre au run | Fin des writes/fermeture | Aucun résultat partiel utilisable garanti | Export batch exploratoire |
| JSON document publié | Fichier temporaire dans même dossier, puis rename vers un nom unique | Writes puis sync du fichier, rename, sync du dossier si supporté | Publication d'un document complet | Export complet ; garantie OS explicitée |
| JSON par objet | Un fichier à nom unique par record | Writes/close ; variante sync distincte | Publication selon stratégie de chaque fichier | Coût par objet et namespace |
| JSONL buffered | Lignes complètes appendées par un writer | Writes/flush des buffers applicatifs | Pas de transaction multi-record | Streaming ; aucune preuve power-loss |
| JSONL synchronisé par lot | Lots de `B` lignes, un writer | Toutes les lignes écrites puis sync du fichier | Pas de all-or-nothing du lot | Préfixe de lignes vérifiable après crash |
| SQLite WAL NORMAL | Lignes dans transaction de `B` records | Commit | Transaction SQLite | Série sans durabilité power-loss des derniers commits |
| SQLite WAL FULL | Même schéma et lots | Commit, sync selon SQLite/VFS | Transaction SQLite | Série de durabilité renforcée, hypothèses OS/device explicites |

Les cellules buffered, NORMAL et FULL ont des contrats différents et restent visibles comme tels. JSONL sync et SQLite FULL peuvent répondre à un objectif commun de **records acquittés conservés**, mais JSONL seul n'offre pas une transaction atomique du lot. Si all-or-nothing est requis, cette capacité absente doit apparaître ; ne pas l'ajouter implicitement au score.

Un write retourné, un buffer applicatif vidé, un fichier synchronisé et un record vérifié après reopen sont quatre observations distinctes. `close()` et un rename ne prouvent pas seuls une publication durable.

## 5. Fsync et rename : les limites importantes sur ce Mac

La documentation Linux de `rename()` décrit le remplacement atomique ; `fsync(file)` ne garantit pas que l'entrée du dossier a atteint le disque : il faut aussi synchroniser le dossier. Cela justifie une stratégie temp dans le même dossier → sync file → rename → sync directory **sur les systèmes où ces opérations sont supportées**. [rename](https://man7.org/linux/man-pages/man2/rename.2.html), [fsync](https://man7.org/linux/man-pages/man2/fsync.2.html).

Node `FileHandle.sync()` demande le flush au device avec une réalisation dépendante de l'OS/device ; `writeFile({flush:true})` appelle cette opération, pas un protocole complet de publication de namespace. Éviter plusieurs `writeFile()` concurrents sur le même fichier sans attendre leur résolution. [Filesystem Node 24.19](https://nodejs.org/download/release/v24.19.0/docs/api/fs.html#filehandlesync).

Le manuel Apple archivé distingue `fsync()` et `F_FULLFSYNC`, qui demande aussi le flush des buffers du device. Le lab ne doit pas annoncer qu'un `sync()` Node prouve cette garantie sur APFS/macOS actuel. Consigner la méthode et ses erreurs/support réel ; les options SQLite `fullfsync`/`checkpoint_fullfsync` restent des variables explicites. [Manuel Apple](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/fsync.2.html), [PRAGMA fullfsync](https://www.sqlite.org/pragma.html#pragma_fullfsync).

Ne jamais faire disparaître une erreur de sync/directory sync en la traitant comme succès. Si ce niveau n'est pas disponible, étiqueter la cellule selon les opérations réalisées.

## 6. La concurrence doit avoir plusieurs axes

| Axe proposé | Premiers points de campagne | Question |
| --- | --- | --- |
| Producteurs logiques `P` | 1 / 4 / 16 / 64 | Le writer impose-t-il backpressure au crawler ? |
| Writers réels `W` | 1 / 2 / 4 / 8 processus ou workers | Où se trouve la contention et son coût ? |
| Bases / shards `D` | 1 puis `D=W` dans une autre série | La partition permet-elle plus de débit, à quel coût ? |
| Batch `B` | 1 / 100 / 1 000 records | Amortissement contre latence et fenêtre de replay |
| Payload | ~256 B / 4 KiB / 64 KiB, octets réels enregistrés | Binding/metadata ou bandwidth ? |
| Lecture simultanée | Aucun lecteur puis lecteurs courts/longs | Impact du reader sur checkpoint et croissance WAL |

**Ne pas lancer tout le produit cartésien.** Première piste locale : `W=1`, balayage `P` et `B`, puis même base avec `W=2/4`, enfin shards séparés. La série sharded inclut merge/lecture finale, nombre de fichiers et coût global ; elle n'a pas le contrat transactionnel d'une base unique.

JSONL commence avec un seul writer recevant les producteurs. Plusieurs appenders du même fichier sont une cellule expérimentale séparée, avec validation d'interleaving/troncature ; ne pas supposer qu'une ligne sérialisée correspond toujours à un unique write atomique. JSON par objet peut paralléliser des fichiers distincts avec noms disjoints ; pas de race au même path.

Publier `P`, `W`, connexions, processus/threads, `D`, queue max, batch réel, délai de remplissage d'un batch et timeout. Le temps d'attente de la queue et du verrou appartient à la latence utile ; une queue illimitée qui gonfle la RAM ne constitue pas une amélioration soutenable.

## 7. Configuration locale minimale à essayer

Commencer avec même corpus/schéma, `W=1`, statement préparé, `BEGIN IMMEDIATE`, inserts liés, commit par lot. WAL, synchronous et busy timeout fixés à l'ouverture puis relus sur **chaque connexion** ; DB propre à la campagne, sans réutilisation silencieuse de profil antérieur. Exemple de contrat : `busy_timeout=5000 ms`, `B=100`, deadline totale supérieure et explicite ; les valeurs restent réglables.

Pour SQLite, garder le checkpoint automatique déclaré comme première baseline ; mesurer taille `.db` + WAL + SHM et checkpoint/close séparément, plus durée complète jusqu'au drainage final. Une expérience qui reporte tous les checkpoints au-delà de sa fenêtre doit afficher ce travail différé. Le checkpoint PASSIVE peut rester incomplet ; FULL/RESTART/TRUNCATE ont des comportements de blocage différents. [API officielle de checkpoint](https://www.sqlite.org/c3ref/wal_checkpoint_v2.html).

Pour comparer conservation d'un même payload, une table `id TEXT PRIMARY KEY, payload TEXT NOT NULL` est une baseline utile : JSON/JSONL et SQLite stockent les mêmes données logiques. Ajouter ensuite des colonnes/indexes métiers dans une série distincte ; leur coût et leurs capacités de lecture ont un intérêt produit réel. Ne pas remplacer un insert échoué par `INSERT OR IGNORE` qui masque un doublon.

## 8. Oracle, métriques et incidents

Corpus partagé : IDs stables, Unicode/escaping, null, collections et valeurs numériques dans une plage représentable identique. Oracle sémantique sur tous les records ; comptage seul insuffisant. Bytes identiques seulement pour rail préencodé ; sinon encoders peuvent ordonner/échapper différemment sans perdre la valeur. Vérifier IDs, payload et hash normalisé indépendant.

Après la fenêtre chronométrée : reopen depuis un processus frais, parser JSON/chaque ligne JSONL, interroger toutes les lignes SQLite, comparer ensemble/valeurs/versions exacts ; `integrity_check` et `foreign_key_check` selon schéma. Résultats de validation séparés de la durée d'écriture, plus métrique end-to-end validée.

Métriques : records validés/s, octets logiques et physiques/s, bytes d'artefacts, CPU-s, mémoire du writer et des producteurs, queue wait, lock wait, commit/sync p50/p95 avec effectifs, `BUSY`/retries/rejets, checkpoint et fin de publication. Garder résultats bruts, configurations et ordre randomisé ; ne pas comparer un débit NORMAL à FULL sans indiquer le contrat.

Faults locales : writer tué avant/après commit ou sync, avant/après publication et avant/après ack ; garder un ledger d'IDs acquittés côté superviseur. Après reopen, vérifier leur conservation et signaler records supplémentaires non acquittés comme effet possible de la coupure avant ack. JSONL peut avoir un tail incomplet : conserver l'artefact et diagnostic, appliquer une politique de recovery déclarée, pas une réparation silencieuse qui transforme l'oracle.

**Un SIGKILL teste un crash de processus, pas un power-loss ou crash kernel.** Le cache OS et le device restent actifs. Cette piste locale peut démontrer erreurs de protocole, intégrité après incidents et comportement de recovery ; elle ne valide pas expérimentalement les promesses de persistence à coupure électrique. Pas de reboot ni coupure de la machine de travail dans ce lot.

## 9. Décision proposée pour Automan

Première hypothèse : producteurs concurrents → queue bornée → writer unique batché ; JSONL pour export/log séquentiel, SQLite WAL FULL pour état local transactionnel, NORMAL comme option au contrat de perte acceptable explicite. Ce sont des candidats à mesurer, pas un choix définitivement qualifié. Les shards ou plusieurs writers n'entrent qu'après courbes de contention et coût de merge.

La piste locale doit livrer fixtures de records, Node `node:sqlite`, Rust SQLx 0.8.6 séparé des adapters crawler, fichiers JSON/JSONL, matrice limitée, validation intégrale, manifests et résultats bruts. Il n'est pas nécessaire d'introduire PostgreSQL, broker ou stockage distant pour répondre à cette question initiale.
