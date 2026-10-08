# Pages réelles figées (T1) : rendu par Chrome et Lightpanda, 8 octobre 2026

Run retenu : `extraction-rendered-test-2026-10-08T20-14-39-618Z-9f5f67`, en `local-dev` sur sources commitées, Mac en usage. Les 511 pages réelles du jeu de test WCXB (CC-BY-4.0) sont chargées par Chrome et par Lightpanda, puis leur DOM rendu passe à rs-trafilatura. Le résultat est comparé à l'extraction sur le HTML brut et à la vérité terrain de WCXB.

## Comment les pages restent figées

- **Le proxy d'archive** (`fixtures/replay-proxy.ts`) sert chaque page à son hôte et à son chemin d'origine. Il refuse toute autre requête et la compte : sous-ressource absente de l'archive, tunnel HTTPS vers un hôte non archivé.
- **Les hôtes HSTS préchargés.** Chrome charge toujours en HTTPS les hôtes de sa liste HSTS préchargée. Le proxy termine donc TLS lui-même pour les hôtes archivés, avec le certificat local du lab, que Chrome reçoit l'ordre d'accepter.
- **Chrome** passe tout par ce proxy (`--proxy-server`, sans exception pour le loopback). Il ne peut résoudre aucun autre hôte (`--host-resolver-rules`) et ne passe pas de lui-même en HTTPS.
- **Lightpanda** passe par `--http-proxy` et bloque toute adresse sauf le loopback (`--block-cidrs`).

Les deux moteurs voient donc les mêmes octets, et aucune requête ne quitte la machine.

## Résultats

| Moteur | Pages rendues | F1 (moyenne par page) | Snippets requis trouvés | Boilerplate qui fuit | Pages servies par l'archive | Requêtes refusées | Tunnels refusés | Chargement médian (ms) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| aucun (HTML brut) | 511/511 | 0.884 | 0.858 | 0.099 | — | — | — | — |
| Chrome | 511/511 | 0.884 | 0.858 | 0.099 | 511 | 8181 | 5413 | 286 |
| Lightpanda | 511/511 | 0.884 | 0.858 | 0.099 | 511 | 319 | 130 | 211 |

- **Le rendu ne change rien à l'extraction sur ces pages.** Les 1 022 textes extraits après rendu sont identiques, page par page, à ceux du HTML brut. La sérialisation du DOM diffère pourtant : Chrome produit moins d'octets que l'archive. Les pages WCXB sont des instantanés pris après rendu, et sans réseau leurs scripts n'ajoutent pas de contenu principal.
- **Lightpanda rend les 511 pages**, sans échec ni dépassement de délai. Il ne demande presque aucune sous-ressource (319 requêtes refusées contre 8 181 pour Chrome), ce qui explique en partie un chargement plus court. Les durées viennent d'un Mac en usage et restent indicatives.
- **Ce que cela dit pour les décisions :**
  - `D-light-engine` : sur des pages statiques réelles, Lightpanda lit autant que Chrome. Ses limites relevées sur les fixtures restent valables : faux succès sous garde, pas de frames, de popups ni de téléchargements.
  - `D-extraction` : un rendu navigateur n'est pas nécessaire pour extraire le contenu principal d'une page déjà rendue. Il le reste pour une page qui construit son contenu en JavaScript, ce que ce corpus ne contient pas.

## Défauts du harnais trouvés en route

Les runs concernés sont classés `diagnostic`.

- **Page sans URL.** Une page du corpus n'a pas d'URL ; elle faisait échouer toute la cellule « article ». Elle est servie sur un hôte réservé (`<id>.wcxb.invalid`).
- **Montée en HTTPS de Chrome.** Chrome passait de lui-même en HTTPS sur les navigations HTTP (HTTPS-Upgrades), et chargeait en HTTPS les 47 pages d'hôtes HSTS préchargés. La première fonction est désactivée ; le proxy sert désormais HTTPS pour les hôtes archivés.
- **Prédictions écrasées.** Les fichiers de prédictions des cellules rendues portaient le même nom que ceux du HTML brut et s'écrasaient. Ils sont désormais nommés par moteur.

## Limites

- Les pages WCXB ne contiennent que le document : aucune sous-ressource n'est archivée. Une vraie archive Web Page Replay, avec scripts, styles et requêtes de données, est nécessaire pour mesurer les pages qui se construisent en JavaScript.
- Le rerun en `local-controlled` est inscrit au ledger pour les durées.
