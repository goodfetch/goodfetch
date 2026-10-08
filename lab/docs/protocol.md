# Protocole worker du lab, version 1

Un processus est lancé directement, sans shell, dans son propre groupe POSIX. stdin et stdout transportent un objet JSON par ligne. stderr contient les diagnostics, avec une fin de log bornée conservée par le coordinateur.

```json
{"version":1,"id":1,"method":"hello","params":{}}
{"version":1,"id":1,"result":{"backend":"example","protocol":1,"capabilities":["http-catalog"],"versions":{}}}
```

`id` est un entier et corrèle les réponses, qui peuvent arriver hors ordre. Une réponse contient `result` ou `error`. Méthodes : `hello`, `prepare`, `run`, `close`. `prepare` précède tous les runs ; `close` suit leur achèvement. Le client accepte au plus 128 appels actifs. Les requêtes sont bornées à 128 000 caractères côté Node / bytes côté Rust, et le buffer stdout Node à 8 millions de caractères. Un adapter doit réserver stdout au protocole.

Pour le navigateur, `prepare` reçoit `executablePath`, `headless`, `concurrency`. `run` reçoit scenario, URL de fixture, seed, pages/actions, variant et timeout. `hello.capabilities` doit déclarer uniquement les scénarios réellement implémentés. `maxConcurrency`, lorsqu'il existe, borne le plan. Le retour de run est :

```json
{"status":"ok","records":[{"id":"P-001","price":1201}],"metrics":{"taskMs":12.3}}
```

Ou `{"status":"unsupported","reason":"..."}`. Tous les nombres de métriques doivent être finis et positifs ou nuls. Un `ok` n'est pas un verdict métier : seul le coordinateur délivre `verified` après l'oracle. Les éléments attendus et la route de validation privée ne sont pas fournis au worker.

Pour le stockage, `prepare` reçoit path d'un sink neuf, format, busyTimeoutMs. `run` reçoit batchId, seed, rows et payloadBytes. Les IDs attendus sont `<batchId>:<index>`, value est seed+index, payload contient payloadBytes caractères ASCII `x`. Chaque batch est une transaction SQLite ou une unité append/snapshot. Le worker acquitte les lignes et ses timings ; l'oracle relit ensuite le contenu réel. Les lignes `attempts.jsonl` de stockage conservent `acknowledged/error/rejected` ; le verdict terminal indépendant se trouve dans `summary.json`.

Une erreur de protocole ou un timeout échoue tous les appels actifs du worker. Le coordinateur demande SIGTERM au groupe puis SIGKILL, et nettoie les flux. Cette isolation est propre aux processus du lab ; elle ne sert pas à attacher un navigateur utilisateur. Sur Windows, le fallback ne fournit pas la même terminaison complète de descendants que le groupe POSIX.

Le protocole permet d'ajouter Rust, Go ou Python sans changer l'oracle ni interpréter les durées internes comme seules preuves de performance. Le temps principal est mesuré côté coordinateur avec une horloge monotone ; les métriques internes complètent ce temps.

## Extension workflow du prototype

Le worker `workflow-node` réutilise cette enveloppe : `prepare` reçoit `path` et `leaseMs`, puis `run.op` reçoit `submit`, `claim`, `commit`, `ack`, `reject` ou `snapshot`. `commit` reçoit un batch `{jobId, token, records}` et retourne des receipts ; `ack` reçoit ces receipts. Le contrat et les limites de reprise sont détaillés dans [workflow.md](workflow.md). Les marqueurs de crash appartiennent exclusivement au lab local.
