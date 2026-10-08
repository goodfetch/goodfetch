# Revue indépendante des spécifications Automan

**7 octobre 2026 — revue en lecture seule des spécifications ; aucun moteur testé.** Documents relus : `docs/proposition-automan.md`, `docs/architecture.md`, `docs/api.md`, `issues/prd.md`, `docs/product-design.md`, `docs/roadmap-validation.md` et `docs/documentation-plan.md`. Les quatre corrections initiales ont été relues après intégration. La maquette n’était pas disponible pendant cette passe ; son comportement et sa présentation ne sont pas qualifiés ici. Les références de lignes désignent la version relue.

## Résultat actuel

**Aucun finding matériel ouvert dans les sept documents relus.** Les quatre corrections de la première passe sont intégrées aux contrats et à la roadmap. Ce constat porte sur la cohérence des spécifications ; leurs tests restent à réaliser avec le moteur.

## Corrections initiales — résolues après relecture

| Finding initial | Statut et correction vérifiée | Référence actuelle / preuve prévue |
| --- | --- | --- |
| P1 — Race téléchargement après clic | **Résolu dans la spécification.** `expectDownload` acquitte l'installation avant de retourner le handle ; le clic vient ensuite, puis `download.result()`. Le résultat proposé n'attend donc pas le download avant de déclencher l'action. | [docs/api.md:71](/Users/jb/Dev/automan/docs/api.md:71), [contrat:91](/Users/jb/Dev/automan/docs/api.md:91) ; fixture download immédiat dans [roadmap:42](/Users/jb/Dev/automan/docs/roadmap-validation.md:42) |
| P2 — Ack `emit`/`discover` ambigu | **Résolu dans la spécification.** L'ack signifie staging sous requête/tentative/epoch. Le moteur publie résultat/découverte/acquittement après succès et validation. Échec et politique partielle explicites ; effets tiers du callback exclus de l'atomicité. | [docs/api.md:138](/Users/jb/Dev/automan/docs/api.md:138), [architecture:104](/Users/jb/Dev/automan/docs/architecture.md:104) ; crash tests aux frontières dans [roadmap:36](/Users/jb/Dev/automan/docs/roadmap-validation.md:36) |
| P2 — Reprise d'un terminal non définie | **Résolu dans la spécification.** `waiting` reprend ; review/unknown terminaux gardent leur historique, reçoivent une réconciliation prouvée puis une continuation dans un nouveau run lié. Checkpoints revalidés ; effet inconnu non résolu non répété. | [architecture:80](/Users/jb/Dev/automan/docs/architecture.md:80), [API:222](/Users/jb/Dev/automan/docs/api.md:222), [design:82](/Users/jb/Dev/automan/docs/product-design.md:82) ; [roadmap:38](/Users/jb/Dev/automan/docs/roadmap-validation.md:38) |
| P2 — Auth du HTTP local non spécifiée | **Résolu dans la spécification.** Loopback, credential par utilisateur, échange scoped/éphémère, contrôle Host/Origin et protection CSRF. CORS/localhost/token embarqué ne sont pas assimilés à une authentification. | [architecture:303](/Users/jb/Dev/automan/docs/architecture.md:303) ; page hostile, Host/Origin/CSRF et DNS rebinding dans [roadmap:58](/Users/jb/Dev/automan/docs/roadmap-validation.md:58) |

## Points validés dans les textes

- Moteur canonique et SDK minces ; callbacks Node/Python locaux et packages cloud explicitement distingués. Aucun callback opaque supposé sérialisable en Rust.
- Frontier et exécuteur distincts, admission commune ; intégration Crawlee avec un seul propriétaire de queue. Les trois formes d'autoscaling et les pools sont inventoriés ; aucune vitesse n'est déduite du seul langage.
- Lease avec epoch et fencing internes ; partition avec ancien worker vivant, quarantine et effet externe non annulable distingués. Résultat/découverte/ack transactionnels seulement dans un même store ; blobs et outbox ne bénéficient pas d'une fausse transaction distribuée.
- Statut technique séparé du succès métier ; validation d'identité, complétude, fraîcheur et effets séparée. `done` du modèle ne vaut pas preuve et une annulation incertaine conserve l'effet inconnu.
- Caches perception/plan/données séparés ; nouveau run frais et replay de journal distingués. Repair candidate/version immuables, validateurs préservés, tests/canary/rollback ; aucun rollback prétendu annuler un effet tiers.
- Vite SPA est choisi dans les quatre documents ; Next reste une alternative justifiée. App macOS réellement SwiftUI/AppKit, daemon partagé, lifecycle de fond explicite, FFI non imposé.
- REST/SSE publics, RPC privé et adapter MCP séparés. MCP 2026-07-28 ne reçoit pas le handshake ni les requêtes indépendantes serveur du RPC bidirectionnel privé ; compatibilité des révisions à qualifier.
- Licence Apache-2.0 proposée et licences de composants conservées ; fonctions cloud tierces et limites de versions non assimilées à des capacités OSS acquises.
- Le PRD maintient l'ambition complète et annonce des capacités prévues avec critères de livraison. La documentation exhaustive est un travail futur ; les exemples de cette API sont explicitement des propositions. Aucune obligation artificielle de produire dès maintenant la documentation d'un moteur existant.
- Le plan de documentation confirme cette frontière et couvre concepts, guides, références, erreurs, quickstarts à exécuter contre la version livrée, compatibilité, exploitation, SDK/CLI/MCP, macOS et service ; sa couverture est liée aux capacités livrées plutôt qu'au nombre de pages.
- Le design conserve les conventions natives macOS, la séparation entre données/preuves et jugement probabiliste, l'état d'annulation en attente et la continuation liée. Maquette, interfaces et moteur restent annoncés à implémenter.
- La roadmap comprend crashes/staging/epochs, ressources globales, pertes de connexion, download immédiat, sécurité locale et compatibilité par backend/version. Ses benchmarks incluent baselines manuelles, refus/erreurs/couverture, coûts des tentatives et amortissement à 1/10/100/1 000 runs ; les gates ne deviennent pas des performances déjà mesurées.

**Aucune objection matérielle résiduelle à la direction générale et aux contrats relus.** Cette revue ne qualifie pas les drivers, les performances ni l'isolation du futur produit. L'implémentation, les tests, la documentation utilisateur exhaustive et la validation visuelle de la maquette restent des travaux explicitement prévus.
