# Prototype du workflow durable

Le lab exécute maintenant acquisition HTTP et extraction, validation indépendante, queue de résultats bornée, écriture batchée, commit atomique et acquittement persistant. Ce prototype Node établit le contrat à comparer ; il ne choisit pas le runtime du futur noyau portable.

```sh
cd /Users/jb/Dev/automan/lab
# Utiliser le Node qualifié : 25.5.0, puis vérifier node --version.
pnpm workflow campaigns/workflow-throughput.json
pnpm workflow campaigns/workflow-recovery.json
pnpm workflow campaigns/workflow-pool-wakeup.json
```

## Contrat observable

Les jobs soumis sont persistés dans SQLite, avec leur tâche complète. Un même identifiant ne peut pas désigner une tâche différente. `claim` attribue une lease et un token neuf ; un token périmé ne peut pas committer après une nouvelle attribution. La validation des records appartient au coordinateur et à son oracle privé.

Chaque producteur attend l'ack de son résultat avant de prendre un autre job. Au plus `concurrency` jobs restent actifs et leurs résultats peuvent attendre le writer. La queue de résultats et le batch en cours partagent cette même borne. Le backlog durable correspond au corpus fini soumis, borné séparément par les budgets du runner ; ce prototype n'implémente pas encore une admission continue du backlog.

Un seul batch peut être en cours. Il part quand `batchJobs` résultats sont disponibles ou quand `maxBatchWaitMs` expire. `batchJobs` doit rester inférieur ou égal à `concurrency`. Le batch de résultats et les états `committed` des jobs sont enregistrés dans une même transaction. Ensuite, une seconde transaction enregistre les acks à partir des receipts retournés. Aucun job n'est annoncé terminé avant cet ack.

Le sink utilise WAL, `synchronous=FULL`, `foreign_keys=ON`, `busy_timeout=5000`, `wal_autocheckpoint=1000`. Les PRAGMAs effectifs et la version SQLite sont conservés dans le résumé. L'oracle final rouvre la base, vérifie les IDs/prix/doublons/omissions et contrôle l'intégrité structurelle et les clés étrangères.

## Reprise et erreurs

Après un crash avant le commit, aucun résultat partiel du batch ne reste visible. Après expiration de la lease, le job est repris avec un nouveau token. Après un commit dont la réponse est perdue, le coordinateur valide les records persistés puis acquitte le receipt existant. Un rejeu identique retourne ce receipt ; un rejeu avec des records différents échoue. Un job déjà acquitté n'est pas acquis une seconde fois lorsque le même corpus est soumis à nouveau.

Un résultat incomplet ou incorrect est enregistré comme `failed`, avec la raison, sans insertion dans `results`. Les erreurs d'acquisition et pertes de worker interrompent la génération ; la reprise entière est limitée par `restartLimit`. Il n'y a pas de boucle de retry illimitée.

Les campagnes de reprise injectent SIGKILL dans le writer, pendant la transaction ou après COMMIT et avant la réponse. Deux tests fonctionnels arrêtent aussi un coordinateur et ses workers appartenant au test, puis lancent un **nouveau processus coordinateur** : après claim et entre commit et ack. Le serveur de fixtures et l'oracle indépendant restent disponibles à l'extérieur de ces processus.

## Mesures

Chaque cellule crée une base et des fixtures neuves. Les combinaisons sont randomisées par bloc. Le démarrage est séparé ; `workMs` inclut acquisition, extraction, validation, attente de queue, commit, ack, reprise éventuelle et drain des workers. Le débit utile compte les jobs validés après réouverture et inclut aussi `finalizationMs`. Le débit incluant le démarrage est également fourni.

La latence d'ack va de la première prise de lease à l'ack, y compris les retries du writer. Acquisition et extraction restent une mesure combinée. Les timings des jobs réconciliés sont inclus dans les événements lorsque leur première prise de lease appartient au même coordinateur ; la latence traversant un arrêt complet du coordinateur n'est pas encore mesurée. Les tests de redémarrage de coordinateur qualifient un résultat fonctionnel.

Le RSS couvre **le coordinateur, les fixtures, l'oracle et ses descendants**, contrairement à la piste de stockage qui mesure seulement les processus écrivains. L'échantillonnage à 250 ms peut manquer des pics. Les événements sont réellement écrits pendant la mesure : l'instrumentation et les échanges JSONL font partie du coût du prototype.

Les dossiers contiennent manifest, sources figées, résumé, rapport HTML et, par cellule, événements, tentatives et état relu. Le CLI échoue si une cellule échoue ou si des sources/exécutables changent pendant la campagne.

## Limites qui restent à tester

Le schéma n'a pas de migration ; réutiliser uniquement une base créée par cette version du prototype. Les leases utilisent l'horloge murale et n'ont pas de heartbeat. Les données de fixture et leur oracle ne sont pas un système de découverte durable. Les tests de reprise concernent des lectures HTTP idempotentes ; ils ne prouvent pas l'exécution exactement une fois d'un clic, paiement ou effet externe.

Restent notamment : disque plein, cancellation, crash pendant l'ack, arrêt arbitraire répété, heartbeat, lecteurs concurrents durables, scheduling multi-domaines, workers distribués, sorties externes/outbox, grand corpus streaming, CPU/I/O, noyaux intégrés Rust et Go, backend Chromium et plans déterministes. Aucun test de coupure électrique n'est exécuté.

Pour Crawlee pooled, `poolWakeModes` permet de comparer `poll` et `notify` dans une campagne distincte. `notify` utilise l’API publique après l’ajout de nouvelles requêtes, dès qu’un premier handler a démontré la fin de l’initialisation native. Le mode historique `poll` reste le défaut.
