# Sinks, pool de conteneurs et coupure simulée, avec Docker (D-sinks, D-cloud-topology), 9 octobre 2026

Trois tests annoncés « hors de portée sans cloud » se font en partie avec Docker Desktop : les sinks et le pool de conteneurs entièrement, la coupure de courant de façon approchée. Mesures en `local-dev`, Mac en usage.

## Sinks

La lane de stockage écrit trois formats de plus, depuis le même worker Node et sous le même oracle de relecture indépendant :
- **`postgres-copy`** : PostgreSQL 17.11 dans un conteneur Docker local (`postgres:17-alpine`, `synchronous_commit`, `fsync` et `full_page_writes` actifs). Chaque batch passe par `COPY` dans une transaction, et l'acquittement suit le `COMMIT`.
- **`duckdb-table`** : un fichier DuckDB alimenté par l'appender, un commit par batch. Un fichier DuckDB n'accepte qu'un processus écrivain.
- **`parquet-batch`** : un fichier Parquet par batch, encodé par DuckDB dans un fichier temporaire, synchronisé sur disque, renommé, puis le répertoire est synchronisé à son tour.

Runs retenus :
- `storage-sinks-2026-10-09T19-55-10-753Z-e5bb86` : 5 blocs, 40 batches de 32 ou 512 lignes, un writer ;
- `storage-sinks-crash-2026-10-09T19-55-28-446Z-b1c63c` : `SIGKILL` du writer après tous les acquittements.

Les 60 cellules sont vérifiées : chaque ligne acquittée est relue intacte, y compris après le crash du writer.

| Sink | Lignes vérifiées/s, batches de 512 | Lignes vérifiées/s, batches de 32 |
| --- | --- | --- |
| DuckDB | 515 441 | 77 352 |
| SQLite WAL `synchronous=FULL` | 392 802 | 133 069 |
| JSONL + `fsync` | 118 921 | 7 426 |
| PostgreSQL `COPY` | 70 400 | 17 083 |
| Parquet, un fichier par batch | 47 135 | 3 072 |

### Lecture

- **Le débit dépend d'abord de la taille du batch.** Chaque commit durable coûte un aller-retour au disque, voire à un serveur. À 32 lignes, Parquet paie un fichier, deux synchronisations et un renommage par batch ; JSONL paie un `fsync` complet.
- **Les niveaux de durabilité ne sont pas les mêmes sur macOS, et ce tableau ne compare pas des promesses égales.**
  - Le `fsync` de Node (libuv) demande `F_FULLFSYNC`, qui vide aussi le cache du disque. C'est ce que paient JSONL et Parquet.
  - SQLite avec `fullfsync=OFF`, le défaut, et sans doute DuckDB s'arrêtent à `fsync`, qui sur macOS ne vide pas ce cache.
  - PostgreSQL synchronise dans la VM Linux de Docker, sur un disque virtuel.

  Ce dernier point demande une vérification : une durabilité égale se fixe par configuration avant toute comparaison de débit.
- **Pour Automan**, SQLite reste le défaut local (D-storage). DuckDB est un très bon sink analytique local, mais avec un seul processus écrivain. PostgreSQL `COPY` sert l'état partagé du cloud, et son coût est dominé par l'aller-retour. Parquet convient à l'export par lots, pas à l'écriture par petits batches.

## Pool de conteneurs (D-cloud-topology)

Nouvelle lane `topology`. Une session navigateur est un tenant : il visite une page qui pose son cookie, puis la recharge. L'oracle exige trois choses :
- la page est servie ;
- le cookie est posé ;
- le tenant suivant n'arrive jamais avec le cookie du précédent.

Le navigateur est `chromedp/headless-shell:151.0.7922.109`, image épinglée par digest, piloté par CDP. Run retenu : `container-pool-2026-10-09T20-01-19-342Z-147c8e`, 5 blocs de 8 sessions, pool de 2. Le premier run est remplacé : il mesurait la mémoire du pool avant que les conteneurs de remplacement aient démarré.

| Politique | Blocs vérifiés | Acquisition (ms) | Session complète (ms) | Fuites entre tenants | Mémoire au repos |
| --- | --- | --- | --- | --- | --- |
| conteneur neuf par session | 5/5 | 180 | 350 | 0 | 0 |
| pool chaud, conteneur jeté après usage et remplacé | 5/5 | ≈ 0 | 188 | 0 | ≈ 39 Mio par conteneur |
| pool réutilisé sans remise à zéro | 0/5 | ≈ 0 | — | **30 sur 30** | — |
| snapshot Firecracker | `unsupported` | — | — | — | — |

### Lecture

- **Le pool chaud supprime le démarrage du chemin de la session** : environ 180 ms, la moitié d'une session courte. Le prix est de garder des conteneurs au repos, environ 39 Mio chacun.
- **Réutiliser un conteneur sans le remettre à zéro fait fuir la session d'un tenant vers le suivant**, à chaque fois. Un pool doit jeter ou réinitialiser, contexte navigateur compris, avant de servir un autre tenant. La lane le vérifie, et ce contrat doit rester un gate.
- Firecracker exige `/dev/kvm`, que la VM de Docker Desktop n'expose pas. La comparaison des deux topologies attend un hôte Linux avec KVM ; le pool de conteneurs est la seule option mesurée.

## Coupure de courant simulée

Le test se déroule ainsi :
1. Des writers tournent **dans** la VM Linux de Docker, un par format, par batches de 50 lignes. PostgreSQL tourne dans la même VM et est écrit depuis le Mac.
2. Chaque acquittement suit un commit et est noté côté Mac.
3. Après 5 s d'écriture, la VM est tuée par `SIGKILL` (`com.docker.virtualization`), ce qui revient à couper le courant de la machine invitée : ce que Linux n'avait pas vidé sur son disque virtuel est perdu.
4. Après le redémarrage de Docker, chaque batch acquitté doit être présent et entier.

Le disque du Mac n'est pas coupé : le test prouve la discipline de synchronisation de chaque sink dans la machine invitée, pas le comportement du disque physique. Le porteur a lancé lui-même les trois coupures, depuis son terminal ; les données brutes sont dans [`power-cut-2026-10-09.json`](power-cut-2026-10-09.json).

| Sink | Batches acquittés (3 coupures) | Acquittés perdus | Batches partiels | Intégrité |
| --- | --- | --- | --- | --- |
| SQLite WAL `synchronous=FULL` | 5 692 | **0** | 0 | ok |
| SQLite WAL `synchronous=NORMAL` | 63 810 | **45** (à la 2ᵉ coupure) | 0 | ok |
| JSONL + `fsync` | 5 511 | **0** | 0 | ok |
| JSONL sans `fsync` | 86 450 | **86 450, soit tout** | 0 | ok |
| DuckDB | 3 415 | **0** | 0 | ok |
| Parquet, un fichier par batch | 1 714 | **0** | 0 | ok |
| PostgreSQL `COPY` | non mesuré | — | — | le serveur se rétablit par son WAL |

### Lecture

- **Chaque sink tient exactement la promesse de sa configuration.**
  - SQLite en `synchronous=FULL`, JSONL synchronisé, DuckDB et Parquet (fichier temporaire, `fsync`, renommage, `fsync` du répertoire) ne perdent aucun batch acquitté.
  - SQLite en `NORMAL` en perd 45 à une coupure sur trois : ce mode ne synchronise pas le WAL à chaque commit. Il résiste au crash de l'application, pas à la coupure de courant, comme le dit sa documentation.
  - JSONL sans `fsync` perd tout ce que le noyau gardait en cache.
- **Aucun batch partiel et aucune base corrompue**, même pour les sinks qui perdent des données : SQLite passe `integrity_check`, et chaque batch revient entier ou pas du tout.
- La coupure est réelle. Le `package.json` de DuckDB, écrit sans `fsync` par `npm install` juste avant une première coupure préliminaire, est revenu à 0 octet. PostgreSQL, lui, redémarre en *automatic recovery* et rejoue son WAL.
- **PostgreSQL n'est pas mesuré** : le script ne passait pas les valeurs de l'insertion, si bien que chaque transaction échouait sans acquittement. C'est corrigé, et l'essai reste à refaire.

### Ce que cela dit pour l'architecture

L'architecture exige le WAL `synchronous=FULL` pour l'état local (D-storage) et un acquittement seulement après une écriture durable. La coupure le confirme : `NORMAL` ou un JSONL non synchronisé suffisent contre un crash du processus, déjà testé, mais pas contre une coupure. Tout mode « rapide » doit être affiché comme non durable face à une panne de la machine.

## Verdicts

`pnpm lab verdict`, sans éligibilité (`local-dev`) :

| Décision | Option contre référence | Résultat |
| --- | --- | --- |
| D-sinks | Parquet contre PostgreSQL `COPY` | **moins bon** à 512 lignes (0,72) ; `insufficient-n` à 32 lignes (0,19) |
| D-sinks | DuckDB contre PostgreSQL `COPY` | `insufficient-n` : 7,2 fois le débit à 512 lignes, 3,8 fois à 32 |
| D-cloud-topology | Firecracker contre pool de conteneurs | bloqué par le contrat : non mesurable sans KVM ; le pool de conteneurs tient l'isolation |

Les blocs viennent des deux runs : 5 de débit et 1 du run de crash, qui écrit la même charge. Les gates passent pour tous les sinks : aucune ligne acquittée n'est perdue, y compris après le crash du writer.

## Limites

- Hôte `local-dev`, Docker Desktop. Les débits et les durées sont indicatifs ; PostgreSQL et les conteneurs passent par la VM.
- Les durabilités des sinks ne sont pas égalisées (voir plus haut) ; une comparaison de débit à durabilité égale reste à faire.
- D-sinks reste subordonnée à la demande : ces chiffres sont prêts pour le jour où un client demande un sink.
