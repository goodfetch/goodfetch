# Readers et checkpoints SQLite

`pnpm sqlite-readers 3` exécute 108 cellules sur le même artefact SQLite :
Node partagé, Rust synchrone et Go/cgo ; un ou quatre processus writers ;
batch de 100 ou 1 000 lignes ; aucun reader, transactions de lecture courtes,
ou snapshot de lecture maintenu pendant toutes les écritures. Chaque cellule
écrit 32 000 lignes avec payload de 512 octets, WAL FULL et huit producteurs.

La piste mesure startup, écriture jusqu'aux ACKs, checkpoint PASSIVE, libération
du reader, checkpoint TRUNCATE, fermeture et oracle. Les ACKs bruts sont
conservés. Les lecteurs courts tiennent leur transaction 2 ms et recommencent
toutes les 10 ms ; leurs comptes doivent rester croissants et bornés.

Le snapshot long commence avant la première insertion : il doit continuer à
voir zéro ligne pendant que tous les writers acquittent leur travail. Un
checkpoint PASSIVE doit alors conserver des frames non checkpointées.
Après libération, TRUNCATE doit réussir et ramener le WAL à zéro. L'oracle
rouvre enfin la base et contrôle chaque record et son intégrité.

Le coordinateur crée une fois le schéma et active WAL avant les connexions
concurrentes. Les 31 erreurs de la première matrice provenaient de cette course
d'initialisation, avant toute insertion ; ce sont des diagnostics du lab,
pas des mesures de contention des writers. Les writers restent des processus
indépendants qui se disputent réellement le même verrou pendant la mesure.

Ces invariants rendent l'expérience utile même lorsque la fenêtre d'écriture
est courte. Les débits de cette piste ne sont pas une qualification soutenue.
La sonde inclut le coordinateur déjà vivant et les writers ; elle soustrait
les compteurs CPU/I/O initiaux du coordinateur. Son footprint total peut
inclure des allocations de campagnes précédentes et ne classe pas le coût
mémoire intrinsèque des trois langages.

[SQLite WAL](https://www.sqlite.org/wal.html) explique le writer actif unique,
la coexistence des lectures/écritures et l'arrêt du checkpoint derrière un
snapshot long. Cette piste vérifie concrètement ces propriétés et leur coût
dans les bindings du lab. Elle ne teste ni filesystem réseau, ni arrêt
électrique, ni durabilité matérielle au-delà du contrat WAL FULL déclaré.
