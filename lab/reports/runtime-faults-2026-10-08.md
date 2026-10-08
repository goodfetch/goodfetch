# Pannes étendues du runtime durable, 8 octobre 2026

Contrat de D-durable-runtime, exécuté en `local-dev` sur sources commitées. Run retenu : `runtime-faults-extended-2026-10-08T17-49-02-487Z-54a5e6`. Runtimes Node et Rust, 24 jobs par cellule, 3 blocs. Le contrat reste celui de la reprise : chaque job vérifié une fois, avec ses enregistrements exacts, sans perte ni doublon.

| Panne | Ce qui se passe | Node | Rust |
| --- | --- | --- | --- |
| `write-error` | la transaction de commit échoue sans crash (erreur d'E/S simulée) | 1 redémarrage, 24/24 vérifiés | 1 redémarrage, 24/24 vérifiés |
| `lock-held` | une autre connexion tient le verrou d'écriture 0,5 s, sous le délai d'attente de 5 s | aucun redémarrage, 24/24 | aucun redémarrage, 24/24 |
| `lock-starved` | le verrou est tenu 6 s, au-delà du délai d'attente | 1 redémarrage, 24/24 vérifiés | 1 redémarrage, 24/24 vérifiés |

Les deux runtimes tiennent le contrat sur les trois pannes, sur les trois blocs. Ces pannes complètent les crashs de processus déjà couverts (avant et après commit, pendant et après l'acquittement, coordinateur arrêté). La durabilité au-delà d'un crash de processus (coupure d'une VM) reste hors de portée de cette matrice.
