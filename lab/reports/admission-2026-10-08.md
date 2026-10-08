# Admission ouverte et ordonnancement multi-domaines (D-durable-runtime, D-scheduling), 8 octobre 2026

Prototype dans le runtime Rust (mode `admission` de `automan-workflow-rust`), mesuré en `local-dev` sur sources commitées, Mac en usage. Runs retenus :
- `durable-admission-2026-10-08T19-43-14-860Z-b8ad5b` : sans surcharge ;
- `durable-scheduler-2026-10-08T19-43-56-509Z-dfabee` : un domaine lourd en surcharge ;
- `durable-admission-recovery-2026-10-08T19-50-26-865Z-f4c814` : crash en surcharge (domaine lourd à 150 travaux/s), délai de requête de 2 s.

Chaque run compte 5 blocs. Les deux politiques reçoivent, dans un bloc, le même calendrier d'arrivées de Poisson tiré au sort.

## Le prototype

- **Admission.** Une soumission réserve une place sous la borne de la file, puis un fil d'écriture la journalise (SQLite en WAL, `synchronous=FULL`, build partagée du lab). Ce fil commite en une transaction tout ce qui est arrivé pendant le commit précédent. L'accusé n'est envoyé qu'après ce commit. File pleine : refus `E_CAPACITY` avec une indication de nouvel essai, et le travail n'est jamais journalisé.
- **Départ.** Le départ de chaque travail vers le site est journalisé avant la requête, dans le même commit groupé.
- **Ordonnancement.** 8 workers, au plus 2 requêtes simultanées par domaine. Deux politiques :
  - `fifo-global` : une file globale de 64 ; le répartiteur prend le plus ancien travail dont le domaine a une place libre ;
  - `fair-domain` : une file de 16 par domaine sous le même plafond global de 64 ; le répartiteur sert les domaines à tour de rôle.
- **Charge.** Un domaine lourd et trois petits domaines à 15 travaux/s chacun, pendant 4 s ; le site répond en 20 ms. Le domaine lourd envoie 60 travaux/s sans surcharge et 300 travaux/s en surcharge. Avec sa limite de 2 requêtes, il ne peut écouler que 100 travaux/s.

## L'oracle

Le harnais et le site multi-domaines observent le runtime de l'extérieur :
- chaque soumission reçoit une réponse ;
- chaque travail accusé est dans le journal, terminé, avec les enregistrements attendus, et demandé une seule fois au site ;
- chaque refus porte `E_CAPACITY`, n'est jamais exécuté et n'est pas journalisé ;
- le journal ne contient aucun travail non accusé et passe `PRAGMA integrity_check` ;
- le site relève le pic de requêtes simultanées par domaine ;
- la file observée (accusés pas encore arrivés au site) reste sous la borne. Une tolérance égale au nombre de workers couvre le trajet entre la sortie de file et le site.

## Résultats

| Scénario | Politique | Cellules vérifiées | Soumissions | Refus | Acceptation des petits domaines (médiane) | Débit vérifié des petits domaines (travaux/s, médiane) | p95 soumission → fin, petits domaines (ms) | p95 de l'accusé (ms) | Pic de file observé | Pic de requêtes par domaine |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| steady | fair-domain | 5/5 | 2086 | 0 | 1.00 | 47.8 | 26.5 | 1.83 | 9 | 2 |
| steady | fifo-global | 5/5 | 2086 | 0 | 1.00 | 47.8 | 27.6 | 1.82 | 7 | 2 |
| flood | fair-domain | 5/5 | 6851 | 4056 | 1.00 | 46.8 | 24.5 | 1.78 | 19 | 2 |
| flood | fifo-global | 5/5 | 6851 | 4426 | 0.36 | 16.8 | 23.6 | 1.60 | 65 | 2 |
| overload-crash | fair-domain | 5/5 | 3947 | 1882 | 1.00 | 42.8 | 34.6 | 1.61 | 33 | 2 |
| overload-crash | fifo-global | 5/5 | 3947 | 2351 | 0.39 | 17.5 | 24.7 | 1.13 | 65 | 2 |

- **Aucune perte, sous les deux politiques.** Toutes les cellules sont vérifiées. Aucun domaine ne dépasse 2 requêtes simultanées, aucun refus n'est exécuté, et la file reste sous sa borne.
- **Sous surcharge, la file globale se remplit de travaux du domaine lourd.** Les petits domaines voient alors refuser près de deux soumissions sur trois, alors qu'ils restent très loin de leur capacité. La file par domaine les admet toutes et ne refuse que l'excédent du domaine lourd.
- **Les travaux admis attendent peu dans les deux cas.** Le répartiteur saute les travaux d'un domaine déjà à sa limite, donc l'écart porte sur l'admission, pas sur l'attente.
- **L'accusé journalisé reste sous la milliseconde au p95** dans ces conditions. Le commit groupé absorbe les arrivées simultanées.
- **Verdict.** `pnpm lab verdict D-scheduling` compare le débit vérifié des petits domaines, apparié par bloc, dans le scénario de surcharge. Le résultat est rendu dans [decisions.md](decisions.md).

## Reprise après crash

La faute est portée par la première soumission admise après 2 s. Le runtime la journalise, puis se tue avant de répondre : c'est le cas où un client perd l'accusé d'un travail pourtant durable.

- Le harnais redémarre le runtime sur le même journal et renvoie, avec le même identifiant, chaque soumission restée sans réponse.
- Au démarrage, le runtime reconnaît tous les travaux journalisés. Une soumission déjà connue reçoit un accusé `duplicate` sans seconde entrée, et les travaux non terminés retournent dans leur file.
- Un travail en vol au moment du crash est refait une fois après la reprise ; tout autre travail ne l'est jamais.

| Politique | Travaux repris (min–max) | Soumissions renvoyées | Accusés dupliqués | Travaux refaits (min–max) |
| --- | --- | --- | --- | --- |
| fair-domain | 17–21 | 7 | 5 | 2–4 |
| fifo-global | 64–67 | 5 | 5 | 2–4 |

Les 10 cellules sont vérifiées.

**Défaut trouvé par l'oracle.** La première version reprenait la file et répartissait aussitôt. Le site recevait alors une troisième requête simultanée sur un domaine limité à deux : la requête du processus mort était encore servie. Le runtime journalise désormais le départ de chaque travail. À la reprise, une place dont la requête était en vol reste occupée jusqu'à l'échéance de cette requête, soit le délai de 2 s ; la reprise coûte au plus ce délai aux domaines concernés.

**Défaut du harnais.**
- **Crash par minuterie.** La première version tuait le runtime à une heure fixe. Aucune soumission n'était alors en vol, si bien que le renvoi idempotent n'était jamais exercé.
- **Signal asynchrone.** Un signal qu'un processus multithreadé s'envoie à lui-même arrive de façon asynchrone sous macOS. La tâche fautive répondait donc parfois avant de mourir ; elle se bloque maintenant jusqu'à l'arrivée du signal.
- **Soumission refusée.** Une soumission refusée faute de place ne peut pas porter la faute ; la faute attend la suivante. Les runs concernés sont classés `diagnostic`.

## Limites

- La durabilité couvre un crash de processus, comme le reste du runtime ([D-durable-runtime](../decisions/D-durable-runtime.json)), pas une coupure de courant.
- Les fautes du puits du mode coordinateur (`ledger.rs`) se tuent elles aussi avec un signal envoyé à soi-même ; elles ne bloquent pas ensuite, ce qui suffit dans un serveur à un seul thread mais reste à durcir.
- Le domaine d'un travail est déclaré par le harnais ; le produit le dérivera de l'hôte de l'URL.
- Les durées viennent d'un hôte `local-dev`. Le rerun en `local-controlled` est inscrit au ledger.
