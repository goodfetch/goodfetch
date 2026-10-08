# Admission ouverte et ordonnancement multi-domaines (D-durable-runtime, D-scheduling), 8 octobre 2026

Prototype dans le runtime Rust (mode `admission` de `automan-workflow-rust`), mesuré en `local-dev` sur sources commitées, Mac en usage. Runs retenus :
- `durable-admission-2026-10-08T19-22-31-770Z-5ab034` : sans surcharge ;
- `durable-scheduler-2026-10-08T19-23-13-402Z-a1f48f` : un domaine lourd en surcharge.

Chaque run compte 5 blocs. Les deux politiques reçoivent, dans un bloc, le même calendrier d'arrivées de Poisson tiré au sort.

## Le prototype

- **Admission.** Une soumission réserve une place sous la borne de la file, puis un fil d'écriture la journalise (SQLite en WAL, `synchronous=FULL`, build partagée du lab). Ce fil commite en une transaction tout ce qui est arrivé pendant le commit précédent. L'accusé n'est envoyé qu'après ce commit. File pleine : refus `E_CAPACITY` avec une indication de nouvel essai, et le travail n'est jamais journalisé.
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
| steady | fair-domain | 5/5 | 2086 | 0 | 1.00 | 47.8 | 25.3 | 0.90 | 7 | 2 |
| steady | fifo-global | 5/5 | 2086 | 0 | 1.00 | 47.8 | 24.3 | 1.07 | 7 | 2 |
| flood | fair-domain | 5/5 | 6851 | 3966 | 1.00 | 46.8 | 22.3 | 0.81 | 19 | 2 |
| flood | fifo-global | 5/5 | 6851 | 4302 | 0.40 | 18.8 | 22.1 | 0.88 | 64 | 2 |

- **Aucune perte, sous les deux politiques.** Toutes les cellules sont vérifiées. Aucun domaine ne dépasse 2 requêtes simultanées, aucun refus n'est exécuté, et la file reste sous sa borne.
- **Sous surcharge, la file globale se remplit de travaux du domaine lourd.** Les petits domaines voient alors refuser environ trois soumissions sur cinq, alors qu'ils restent très loin de leur capacité. La file par domaine les admet toutes et ne refuse que l'excédent du domaine lourd.
- **Les travaux admis attendent peu dans les deux cas.** Le répartiteur saute les travaux d'un domaine déjà à sa limite, donc l'écart porte sur l'admission, pas sur l'attente.
- **L'accusé journalisé reste sous la milliseconde au p95** dans ces conditions. Le commit groupé absorbe les arrivées simultanées.
- **Verdict.** `pnpm lab verdict D-scheduling` compare le débit vérifié des petits domaines, apparié par bloc, dans le scénario de surcharge. Le résultat est rendu dans [decisions.md](decisions.md).

## Limites

- La durabilité couvre un crash de processus, comme le reste du runtime ([D-durable-runtime](../decisions/D-durable-runtime.json)). Les reprises après crash de ce mode ne sont pas encore injectées : les pannes de commit et d'accusé sont couvertes par `runtime-faults-extended` pour le mode coordinateur.
- Le domaine d'un travail est déclaré par le harnais ; le produit le dérivera de l'hôte de l'URL.
- Les durées viennent d'un hôte `local-dev`. Le rerun en `local-controlled` est inscrit au ledger.
