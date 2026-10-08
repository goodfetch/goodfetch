# Qualification intégrée — clôturée le 8 octobre 2026

Objectif conservé : poursuivre ensemble acquisition HTTP, interactions navigateur,
écriture SQLite, exécution durable et comparaison du workflow Rust complet.
Les chiffres du 7/8 octobre restent des preuves exploratoires, pas des seuils à battre.

Interfaces déjà demandées et retenues : protocole public JSONL des adapters/writers,
workflow acquisition → extraction → validation → queue bornée → commit → ACK,
oracle indépendant de records et d'effets navigateur. Ces interfaces couvrent
l'exactitude et la reprise ; elles ne prouvent ni durabilité après coupure électrique,
ni compatibilité avec tous les sites, ni un déploiement cloud distribué.

## Gates de cette continuation

- [x] Runtime : coordinateur natif Rust/Tokio, acquisition HTTP native, validation,
  batching borné et ledger durable ; comparaison Node à topologie et SQLite communs.
  [30/30 cellules, 3 blocs, sources/binaires inchangés](../results/runtime-native-sustained-2026-10-07T23-21-59-298Z-bf6ecb/summary.json).
  La configuration Rust est maintenant partagée par Arc ; la requalification conserve 30/30 cellules. La série initiale reste disponible.
- [x] HTTP : campagnes répétées plus longues, concurrence et délai paramétrés,
  comparaison native/Spider/Crawlee avec exactitude, saturation et rejets visibles.
- [x] Browser : Stagehand v4, batch, Playwright, Puppeteer et @crawlee/stagehand ;
  contrôles d'actionnabilité natifs sans masquer les incidents par des waits communs,
  effets trusted exacts et variantes retard/overlay/mutation.
- [x] SQLite : taille de batch, vrais writers, lecteurs concurrents et checkpoints,
  même artefact, garanties de commit explicites, relecture indépendante.
- [x] Durabilité : ledger Node/Rust, rejeu, fencing, crash avant/après commit et ACK,
  reprise d'un nouveau coordinateur, faux succès rejeté et borne respectée.
  [42/42 cellules de reprise](../results/runtime-recovery-2026-10-07T23-37-26-683Z-fcaced/summary.json)
  et [57 tests sans skip ni échec](../reports/qualification-current-tests.log).
- [x] Métrologie : startup/travail/drain/oracle séparés, quantiles bruts, processus
  inclus explicitement, sources/binaires figés et échecs conservés.
- [x] Livraison : commandes reproductibles, rapport transversal, preuves et
  recommandations distinguant observations, hypothèses et limites.

Les gates sont closes pour le périmètre exécuté, avec les échecs natifs des comparateurs et les limites maintenus dans le rapport. Elles ne prouvent pas toute l'architecture future.

Preuves transversales : [rapport](../reports/integrated-2026-10-08.md), [première suite complète](../results/qualification-integrated-2026-10-07T23-21-58-974Z-6b64b0/summary.json), [requalification complète](../results/qualification-current-2026-10-08T00-11-54-553Z-21d1f2/summary.json), [audit des contrats](../reports/integrated-verification.json), [audit des sources](../reports/integrated-source-audit.json), [reproduction](../reports/qualification-reproduce.md).

## Corrections issues des campagnes, avant clôture

- [x] Partager la configuration immuable du coordinateur Rust, puis remesurer
  les mêmes cellules Node/Rust ; conserver la série initiale.
- [x] Corriger les deux observations de stabilité sur de vraies frames RAF,
  couvrir le clic mobile par le worker public et requalifier les guards.
- [x] Initialiser une seule fois le schéma/WAL avant l'ouverture des writers
  concurrents ; ajouter le contrat W4 et relancer les 108 cellules SQLite.
- [x] Tester les autres variantes HTTP Spider sans effacer les trois refus de
  redirection loopback liés à son contrôle SSRF natif.


Résultats corrigés : runtime 30/30, reprise 42/42, guards 162/162, readers SQLite 108/108 (3 456 000 lignes) et Spider hors redirection loopback 60/60. La redirection demeure un refus de la politique SSRF native ; aucun contournement. Les diagnostics rouge/vert du guard et du startup W4 sont liés depuis le rapport.
