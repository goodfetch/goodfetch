# Besoins par segment, nouveaux usages agents et exigences acheteurs

**Statut :** v1, recherche documentaire. **Date :** 2026-10-08. **Corpus :** 421 items ([corpus.jsonl](corpus.jsonl)).
**Limites :** le segment est **estimé** par le codeur à partir du contexte déclaré ; il ne s'agit pas d'un firmographique vérifié. La taille de chaque segment dans le corpus reflète surtout **qui parle en public** (dépôts OSS d'agents, HN) et non la taille du marché. 32 items sont `unknown`.

## 1. Répartition

| Segment | Items | Part | Trois premiers codes (n) | Option de `D-segment` |
| --- | ---: | ---: | --- | --- |
| `agent-developer` | 186 | 44 % | `agent-token-latency-cost` (36), `integ-mcp-agent-tooling` (34), `access-antibot-ip` (28) | `agent-developers` |
| `data-team` | 86 | 20 % | `access-antibot-ip` (22), `perf-memory-concurrency` (13), `ops-scale-ratelimit-scheduling` (10) | `data-teams` |
| `unknown` | 32 | 8 % | `compliance-site-owner-politeness` (11), `market-adoption-stats` (7) | — |
| `b2b-portal-integration` | 29 | 7 % | `usecase-portal-workflow` (14), `access-auth-session` (7), `wtp-price-points` (6) | `b2b-portals` |
| `enterprise` | 29 | 7 % | `compliance-security-privacy` (15), `access-antibot-ip` (5), `build-vs-buy-switching` (4) | transverse |
| `ecommerce-pricing` | 22 | 5 % | `usecase-monitoring-leadgen` (11), `access-antibot-ip` (8), `wtp-price-points` (4) | `ecommerce-pricing` |
| `seo-marketing` | 16 | 4 % | `usecase-monitoring-leadgen` (9), `wtp-price-points` (6), `compliance-security-privacy` (3) | — |
| `hobbyist` | 16 | 4 % | `wtp-price-points` (4), `access-antibot-ip` (3), `agent-human-in-loop` (3) | — |
| `research-academic` | 5 | 1 % | `usecase-qa-research-ingest` (3), `data-extraction-quality` (2) | — |

Les items `unknown` sont surtout des exploitants de sites, qui ne sont pas des acheteurs mais **une contrainte** (§4).

## 2. Besoins par segment

### 2.1 Développeurs d'agents (`agent-developer`, 186)

**Ce qu'ils demandent :**

1. **Une représentation de page économe en tokens.** Snapshots paginés (`github-107`), sorties d'outils moins verbeuses (`github-106`), HTML nettoyé fourni par la plateforme (`github-105`), poignées d'éléments stables qui survivent aux re-rendus (`reddit-14` `[P]`, `reddit-29`).
2. **Compiler ou rejouer au lieu de raisonner à chaque run.** Export des runs en script (`github-15`, `hn-12`), réutilisation des XPaths (`github-40`), cache versionnable hors du cloud de l'éditeur (`hn-87`).
3. **Utiliser les sessions réelles.** S'attacher au vrai Chrome ou à un profil persistant (`reddit-7`, `reddit-9`, `github-113`), avec un profil par service et des mots de passe invisibles pour l'agent (`demand-use-cases-68` `[P]`).
4. **Vérifier l'état atteint.** Un outil d'assertion natif (`github-25`), chaque action vérifiée par relecture de la page (`reddit-1`), un résultat « bloqué » typé (`github-80`).
5. **Garder le contrôle du coût.** Tokens visibles dans les logs (`github-21`), plafonds par exécution et par outil, budgets de retry par classe d'erreur, coupe-circuits (`reddit-58`), coût par résultat accepté suivi au p95 (`reddit-65`).
6. **Brancher le modèle et le runtime de leur choix.** Modèles locaux (`github-31`), compatibilité avec l'AI SDK (`github-49`), SDK Python (`github-48`), runtime edge (`github-45`).
7. **Une interface adaptée à l'hôte.** Claude Code (65,5 %), Copilot (58,7 %), Codex (29,5 %) et Cursor (25,5 %) dominent l'usage des agents de code (`reviews-reports-54`, Stack Overflow 2026, 12 255 répondants). Le choix entre MCP et CLI reste ouvert (`hn-50`, `hn-51`, `hn-42`).

**Volonté de payer :** faible et polarisée. Plusieurs prix sont jugés excessifs (`hn-60`, `hn-58`), la tentation du « Codex + bibliothèque OSS » est forte (`hn-71`, `demand-use-cases-27`), et seuls 6,4 % des développeurs ayant changé d'outil IA l'ont fait pour le coût, contre 25,1 % pour la qualité (`reviews-reports-53`). C'est le segment qui **parle le plus** et qui **justifie le moins** une offre payante.

### 2.2 Équipes data (`data-team`, 86)

**Ce qu'elles demandent :**

1. **De l'accès sans proxy « temps plein ».** Bascule vers des API hébergées quand chaque site difficile demande rendu, proxy et captcha (`reddit-6`, `reddit-3`). Dépenses de proxy en hausse pour 58,3 % des répondants, et achat chez 2 à 3 fournisseurs pour 43,1 % (`reviews-reports-36`, `reviews-reports-37`, `[P]` Apify).
2. **Un moteur qui ne s'emballe pas.** Backoff réel sur les 429 (`github-85`, `github-96`), autoscaling qui lit correctement la mémoire (`github-84`), plus de processus Chrome orphelins (`github-70`), moins de fuites mémoire (`github-69`).
3. **Une hygiène de correction.** Pas de doublons après retry ou redirection (`github-7`), pas de perte d'items à la fermeture (`github-8`), pas de crawl partiel muet (`github-74`), une reprise après crash et pas seulement après un arrêt propre (`github-5`).
4. **Le même produit en auto-hébergé.** Proxy (`github-52`), images prêtes (`github-56`), Helm (`github-58`), parité entre serveur et bibliothèque (`github-76`).
5. **Le HTTP d'abord, le navigateur en repli.** Développeurs qui réécrivent ce schéma eux-mêmes (`github-94`), pages JS qui tirent souvent d'un endpoint JSON (`hn-22`).

**Volonté de payer :** les budgets réels passent par le **proxy et l'infrastructure**, pas par le framework. 62,5 % déclarent une hausse du coût d'infrastructure hors proxy (`reviews-reports-39` `[P]`). Le rate limiting distribué serait « un problème pour lequel on paie » (`hn-103`). Mais selon l'enquête Apify, 46,7 % n'utilisent que du code maison et 5 % seulement s'en remettent entièrement à l'externe (`reviews-reports-38` `[P]`).

### 2.3 Intégration de portails B2B (`b2b-portal-integration`, 29)

**Ce qu'ils demandent :**

1. **Des sessions de longue durée.** Sessions qui tiennent des mois, alertes de reconnexion, passage de main humain → agent pour la 2FA (`hn-34`). Cookies qui tiennent entre workflows (`github-99`, `github-28`).
2. **Des identifiants hors du LLM** et des identités séparées par fournisseur (`demand-use-cases-38`, `demand-use-cases-52` `[P]`).
3. **Des sorties exploitables.** Téléchargement et renommage de fichiers, sortie conforme à un schéma vers l'ERP (`demand-use-cases-10`, `demand-use-cases-52` `[P]`), avec reprise.
4. **Une approbation avant les effets de bord.** Validation humaine avant soumission ou paiement (`demand-use-cases-11`, `demand-use-cases-69`).
5. **Des sites qui changent sans prévenir.** Coupa injecte des éléments DOM de façon non déterministe (`hn-38`) ; un script écrit à la main demande 10 à 20 h par site (`hn-81` `[P]`).

**Volonté de payer :** les prix déclarés les plus élevés du corpus viennent de ce segment, mais de **sources d'éditeurs**. Simplex affirme que des entreprises de cinq personnes signent à 2 500 $/mois (`hn-33` `[P]`), un prix contesté faute de paiement à l'usage (`hn-32`). Un utilisateur s'est inscrit au palier payant de Browser Use dès le lancement (`hn-13`). Intuned cite comme clients la govtech, l'insurtech et des agences, dont certaines font tourner plus de 1 000 scrapers (`demand-use-cases-26` `[P]`). Côté missions, les tarifs vont de 3 $/h (`demand-use-cases-4`) à 20-30 $/h (`demand-use-cases-11`). **La preuve indépendante est mince.** Seuls 8 des 29 items sont promo ou affiliés, mais ce sont eux qui portent les prix élevés et les volumes de clients. Les 21 autres sont surtout des offres de mission à bas tarif et des issues techniques.

### 2.4 Veille prix e-commerce (`ecommerce-pricing`, 22)

**Ce qu'ils demandent :**

1. **Le rapprochement automatique des produits.** Coller des URL pour des milliers de SKU revient à vérifier à la main (`reddit-46`).
2. **Des diffs et des seuils d'alerte.** Le scraping est « la moitié facile » ; il faut aussi suivre stock et promotions (`reddit-44`). Un bug montre l'enjeu : un passage de 12 $ à 21 $ est vu comme « inchangé » (`demand-use-cases-40`).
3. **Du volume malgré l'anti-bot.** 20 000 articles OEM par jour avec alerte à 2 % d'écart (`reddit-41`). Une équipe a mis 6 mois à rendre fiable la pile proxy, solveurs, pipeline et surveillance (`reddit-42`). Une autre vise plus de 500 M pages par jour (`hn-105`).

**Volonté de payer :** très bimodale.

- **Petits vendeurs :** moins de 30 $/mois souhaités face à Prisync (99 $) et Price2Spy (83 $) (`reddit-43`), refus de 100 $/mois et plus (`reddit-47`), mission à 35 $ (`demand-use-cases-6`).
- **Haut de marché :** cas d'éditeur passé d'une pile DIY à 530 k$/an à une API à 75 k$/an (`demand-use-cases-57` `[P]`). La veille prix et concurrentielle est le cas d'usage à la croissance la plus rapide (18,34 % de TCAC) selon Mordor (`demand-use-cases-45`).

### 2.5 SEO, marketing et génération de leads (`seo-marketing`, 16)

**Ce qu'ils demandent :**

- **Google Maps à grande échelle**, enrichi en e-mails et téléphones valides, payé au lead accepté ou à l'enregistrement (`demand-use-cases-15`, `demand-use-cases-16`, `demand-use-cases-18`).
- **Le suivi de visibilité dans les IA (GEO).** Outils à 95-828 $/mois, dont plusieurs scrapent l'interface de vraies sessions (`demand-use-cases-66`). Google AI Overviews est protégé par l'anti-bot, et le coût par requête est volatil (`demand-use-cases-67` `[P]`).

**Risque :** l'automatisation LinkedIn expose à des bannissements et à des poursuites (`reddit-77` `[P]`, `reddit-78` `[P]`). Pour ces acheteurs, la sécurité du compte est le critère d'achat.

### 2.6 Recherche, académique et amateurs (21)

- **Recherche :**
  - Une sécurité juridique avant tout (`reddit-76`).
  - Mesurer le rappel d'un crawler (`github-83`) et la qualité des tableaux extraits pour un RAG financier (`github-66`).
  - Des configurations Scrapy par site écrites par un agent pour 1-3 $ de tokens par site, sans LLM par page (`demand-use-cases-33` `[P]`).
- **Amateurs :** budget autour de 50 $/mois, sans vouloir « devenir expert en scraping » (`reddit-2`). Réticence face aux factures d'agent (2 tâches pour 15 $, `reddit-30`). Résolution manuelle des captchas (`reddit-66`).

## 3. Nouveaux usages LLM et agents

| Usage | Ce qui est demandé | Sources |
| --- | --- | --- |
| **Agents QA et tests de régression** | Cas en langage naturel, run réel, verdict avec traces, puis **test Playwright déterministe exporté** pour la CI ; filtrer les faux positifs | `demand-use-cases-19`, `demand-use-cases-20`, `demand-use-cases-21`, `demand-use-cases-22`, `github-25`, `github-110`, `hn-0`, `hn-94` |
| **Agents de recherche (recherche puis scraping)** | Recherche sémantique d'abord, rerank qui écarte 60 à 80 % des URL, Playwright en dernier recours, arrêt anticipé ; rapport sourcé | `reddit-31`, `reddit-32`, `demand-use-cases-5`, `hn-18` |
| **Ingestion de sites pour le RAG** | Bases de connaissances multi-tenant tenues à jour, qualité structurelle mesurée | `demand-use-cases-13`, `demand-use-cases-14`, `github-66`, `reddit-59` |
| **Agents dans des SaaS déjà connectés** | Stripe, Search Console, outils internes ; profils isolés ; 14 % des requêtes d'agents visent des pages de compte ou d'authentification (HUMAN) | `demand-use-cases-68` `[P]`, `demand-use-cases-41`, `reddit-25`, `demand-use-cases-34` `[P]` |
| **Portails fournisseurs et administrations** | Factures de centaines de portails de fournisseurs d'énergie, achats, éligibilité d'assurance, NFS-e, formulaires administratifs | `demand-use-cases-50` `[P]`, `demand-use-cases-51` `[P]`, `demand-use-cases-12`, `reddit-68`, `demand-use-cases-32` |
| **Opérations à approbation humaine** | Clinique : factures vers l'ERP avec validation avant soumission ou paiement, modèles locaux (HIPAA) ; réservation où le paiement marque une frontière d'autorisation | `demand-use-cases-69`, `reddit-67` |
| **Suivi GEO et visibilité dans les IA** | Citations de marque dans ChatGPT, Perplexity et AI Overviews | `demand-use-cases-66`, `demand-use-cases-67` `[P]` |
| **Agents comme compétence (skill) ou serveur MCP** | Scraping packagé en compétence d'agent ; 2 265 Actors MCP dans l'Apify Store | `github-114`, `reviews-reports-59`, `hn-108` |

Constante dans ces usages : **le résultat doit être vérifiable**. Seuls 48 % des développeurs font confiance à l'IA, et seulement quand ils peuvent vérifier facilement. 78,7 % jugent l'attribution des sources importante ou très importante (`reviews-reports-51`).

## 4. Exigences des acheteurs et contraintes de conformité

| Exigence | Qui la formule | Sources |
| --- | --- | --- |
| SOC 2 Type II, ISO 27001, DPA, liste des sous-traitants, droit d'audit | Acheteurs entreprise de services de données | `demand-use-cases-62` `[P]`, `demand-use-cases-64` `[P]`, `hn-25` |
| Conservation nulle des données, politique de rétention et de suppression, documentation sécurité | Évaluateurs de navigateurs cloud | `hn-91`, `hn-31`, `demand-use-cases-62` `[P]` |
| **Provenance par enregistrement** (URL, horodatage, statut robots) et entité qui effectue la collecte | Due diligence de fournisseurs de données, EU AI Act (2026-08-02) | `demand-use-cases-63` `[P]`, `reviews-reports-47` `[P]` |
| Supervision humaine, journalisation, audit par action | Déploiements d'agents dans l'UE, équipes sous SOC 2 | `demand-use-cases-39`, `demand-use-cases-50` `[P]`, `reddit-29` |
| Identifiants jamais vus par le LLM, ni stockés chez le prestataire ; 2FA non automatisée | Intégrateurs de portails, clients sensibles à la sécurité | `demand-use-cases-38`, `hn-35`, `reddit-50` `[P]` |
| Exécution locale ou auto-hébergée, licence pour son propre environnement | Santé, cabinets, équipes sensibles à la confidentialité | `demand-use-cases-10`, `hn-40`, `hn-92`, `hn-44` |
| Définition écrite de la précision, détection d'anomalies, SLA sur la livraison et pas seulement sur la disponibilité, PoC sur 20 à 50 domaines de l'acheteur | RFP de veille prix | `demand-use-cases-61` `[P]`, `demand-use-cases-64` `[P]` |
| Origine des IP résidentielles consentie, KYC, signalement des abus | Achat de proxies | `demand-use-cases-65` `[P]` |
| Chaîne d'approvisionnement maîtrisée | Utilisateurs de CLI d'agents | `github-33` |
| Collecte licite de données publiques ; accès non autorisé exclu | Acheteurs de leads ; affaire pénale en Australie | `demand-use-cases-16`, `reddit-75` |

**Contrainte côté sites :**

- **Défauts polis.** Respect de robots.txt et throttling (`github-0`, `github-1`, `hn-16`).
- **Identité de crawler.** Agent utilisateur identifiable (`github-60`).
- **Réputation des IP.** Les IP partagées des navigateurs cloud sont bloquées en bloc (`demand-use-cases-31`, `hn-110`).
- **Réglages par défaut de Cloudflare.** Agents et robots d'entraînement bloqués par défaut, pay-per-crawl (`reddit-70`, `reddit-72`, `hn-112`).
- **Robots.txt des grandes plateformes.** LinkedIn, Instagram et TikTok excluent GPTBot et ClaudeBot (`reddit-74` `[P]`).
- **Directives IA dans les robots.txt.** 13 % des sites ont des règles explicites pour les robots IA, 69 % des sites de presse (`reviews-reports-44` `[P]`).

Pour le segment `enterprise`, la conformité est le premier code (15 items sur 29). Un acheteur qui a du budget parle de conformité **avant** la qualité des données (`demand-use-cases-59`).

## 5. Lecture pour `D-segment` (provisoire, à trancher en entretiens)

| Option | Volume de voix | Douleur alignée sur les mécanismes du labo | Preuve de volonté de payer | Risque principal |
| --- | --- | --- | --- | --- |
| `agent-developers` | Fort (186) | Fort : tokens, rejeu compilé, sessions, faux succès | Faible, pression du DIY | Faire de l'OSS sans revenus |
| `b2b-portals` | Faible (29) | Fort : sessions longues, identifiants hors LLM, approbation, audit, reprise | La plus élevée affichée, mais portée par des éditeurs | Preuve indépendante mince ; vente longue |
| `data-teams` | Moyen (86) | Moyen : robustesse du moteur, rate limiting, reprise | Budgets proxy et infra réels, préférence pour le maison | Concurrence frontale des API d'accès |
| `ecommerce-pricing` | Faible (22) | Moyen : diffs, rapprochement produit, volume anti-bot | Bimodale : PME sous 30 $, grands comptes en services | Rapprochement produit hors périmètre du labo |

Ce que la recherche documentaire ne tranche pas, et que les entretiens doivent trancher :

1. Les portails B2B paient-ils vraiment à hauteur des prix affichés par les éditeurs ?
2. Un développeur d'agent paie-t-il pour un **résultat vérifié** alors qu'il peut générer un script lui-même ?
3. La conformité entreprise (§4) est-elle un prérequis dès le premier segment, ou peut-elle attendre ?
