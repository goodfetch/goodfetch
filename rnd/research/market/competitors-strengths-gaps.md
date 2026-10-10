# Concurrents : ce que les utilisateurs saluent, ce qu'ils reprochent, pourquoi ils changent

**Statut :** v1, recherche documentaire. **Date :** 2026-10-08. **Corpus :** 421 items ([corpus.jsonl](corpus.jsonl)).
**Limites :**

- **Asymétrie de source.** Les outils OSS (Browser Use, Firecrawl, Crawl4AI, Crawlee, Scrapy, Skyvern, Scrapling, Spider) sont jugés surtout par leurs **issues GitHub**, qui portent par nature des plaintes et non des éloges. Les services fermés (Bright Data, Zyte, Oxylabs, ScrapingBee, ScraperAPI) le sont par des **avis**, plus équilibrés mais agrégés et parfois rédigés par des concurrents (`[P]`).
- **Ce que le document ne fait pas.** Un nombre de plaintes ne mesure pas la qualité d'un produit, et ce document ne classe pas les concurrents. Pour les capacités vérifiées et les versions, voir [la cartographie commerciale](../2026-10-07-commercial-landscape.md), [les frameworks navigateur](../2026-10-07-browser-frameworks.md) et [les moteurs de crawl](../2026-10-07-crawl-engines.md).

## 1. Firecrawl (43 items)

- **Salué :**
  - Mise en route rapide, Markdown propre prêt pour les LLM, extraction par schéma, serveur MCP officiel pour Claude et Cursor (`reviews-reports-29`, `reviews-reports-32`).
  - Efficace pour constituer des listes d'entreprises (`hn-124`).
  - La version auto-hébergée fonctionne avec moins de 2 % d'échecs (`hn-63`).
  - Croissance déclarée : 1,5 M d'utilisateurs et plus, Série B de 75 M$ (`reviews-reports-56` `[P]`, `hn-107`).
- **Reproché :**
  - **Prix** : jugé excessif (`hn-60`) ; abonnement à environ 190 $/mois remplacé par du code maison (`hn-58`).
  - **Crédits** : difficiles à prévoir et non reportés (`reviews-reports-29`, `reviews-reports-30`) ; extraction jusqu'à 10 fois le crédit de base (`reviews-reports-32`) ; endpoint agent facturé même en échec (`reviews-reports-31`).
  - **Sites difficiles** : un repli sur Bright Data reste nécessaire (`reviews-reports-32`). Reddit et LinkedIn sont refusés (`hn-61`, `reviews-reports-31`).
  - **Auto-hébergement** : il manque le moteur anti-bot Fire-Engine et les endpoints agent et navigateur (`reviews-reports-31`), ainsi que les proxys (`github-52`) ; migrations de base absentes (`github-57`).
  - **Robustesse** : boucles infinies sur pages bloquées (`github-53`, `github-54`), erreur opaque « All scraping engines failed » (`github-55`), crawls bloqués en statut « scraping » (`github-63`), captures d'écran non déterministes (`github-64`).
  - **Suivi des changements** : il compare des caractères triés, si bien qu'un passage de 12 $ à 21 $ est vu comme inchangé (`demand-use-cases-40`).
  - **Choix du LLM** : extract limité à OpenAI, Ollama qui exige une clé Google (`github-61`, `github-62`).
  - **Crawl** : gestion de robots.txt jugée incohérente (`hn-62` `[P]`), exploitants de sites qui réclament un agent utilisateur identifiable (`github-60`), boucle de retry à 40 $ côté client (`reddit-57`).
- **Changements :**
  - Vers Kagi Extract avec repli sur l'API navigateur de Cloudflare, pour le coût (`hn-56`).
  - Vers du code maison, pour le prix et la finition (`hn-58`).
  - Vers Firecrawl auto-hébergé, pour sortir des crédits (`hn-63`).
  - Départ envisagé pour maîtriser l'étiquette de crawl (`hn-62` `[P]`).
- **À noter :** la plupart des « avis Firecrawl » bien classés seraient écrits par des concurrents (`reviews-reports-30`). Une page cache un texte destiné aux agents IA pour orienter leur inscription (`hn-108`).

## 2. Browser Use (41 items)

- **Salué :** aucun item d'éloge explicite dans le corpus, qui compte 15 items `pain` et 14 demandes. Un utilisateur s'est inscrit au palier cloud payant dès le lancement (`hn-13`).
- **Reproché :**
  - **Premier lancement** qui boucle sur `about:blank` (`github-13`, 55 ; `github-14`, 51).
  - **Tokens** multipliés par deux entre 0.8.1 et 0.11.1 (`github-22`) ; jusqu'à 40 k caractères renvoyés à chaque étape (`reddit-22` `[P]`).
  - **Furtivité** en régression après Patchright et le passage au tout-CDP (`github-26`, `github-27`).
  - **Profil Chrome réel** cassé par Chrome 136 (`github-19`) ; `storage_state` peu fiable (`github-28`).
  - **Cloud** qui échoue alors que la page de statut est au vert (`github-29`).
  - **Comportement de l'agent** : fin déclarée trop tôt (`github-30`), aucune récupération après le crash d'une page (`github-23`), contexte Claude Code corrompu par une capture d'écran (`github-24`).
  - **Dépendances et sécurité** : versions épinglées en `==` (`github-32`), litellm compromis via la CLI (`github-33`), Chrome principal piloté en CDP sans sandbox (`hn-7`, critique d'un concurrent `[P]`).
  - **Imprévisibilité, lenteur et coût** (`reddit-25`, `hn-10`, `hn-11`).
- **Demandé :**
  - Contrôle humain : intervention humaine (`github-11`, 42), MFA (`github-12`).
  - Rejeu : export des runs en script (`github-15`), cache (`github-16`), assertions (`github-25`).
  - Intégration : image Docker pour l'auto-hébergement (`github-17`), MCP (`github-18`, `hn-8`), captcha (`github-20`), tokens dans les logs (`github-21`).
  - Conformité : EU AI Act (`demand-use-cases-39`).
- **Changements :**
  - Abandon pour imprévisibilité (`reddit-25`).
  - Sur un pilote de 5 tâches, Computer Use a été moins cher dans 4 cas, et le DOM a gagné sur une grille produit dense (`reddit-61`).
  - Sur une tâche d'administration interne : 47 étapes et ~495 k tokens pour l'agent vision, contre 8 appels et ~12 k tokens pour un agent API (`hn-99` `[P]`).

## 3. Stagehand et Browserbase (34 items)

- **Salué :**
  - Proxys et captchas intégrés à l'abstraction (`hn-4`).
  - Sessions navigateur pour agents faciles à lancer et à faire grandir (`reviews-reports-33`).
  - Bien moins cher que Simplex (`hn-36`).
  - v4 regroupe les commandes (batching) dans une extension (`hn-86` `[P]`).
  - Volumes : environ 37 M sessions/mois et 10 000 entreprises et plus (`demand-use-cases-48`) ; 4 M de téléchargements npm par mois (`hn-86` `[P]`).
- **Reproché :**
  - **OSS vu comme rampe vers le cloud** : persistance réservée au compte payant (`github-42`), cache d'actions lié aux sessions Browserbase (`hn-87`), accélération réservée au cloud (`hn-88`).
  - **Régressions de version** : persistance de session cassée en v3 (`github-43`), incompatibilité avec l'AI SDK (`github-49`).
  - **Extraction et précision** : URL hallucinées par `extract()` (`github-46`), `act()` qui déborde sur la navigation (`github-47`), XPaths en cache cassants (`github-41`), modèles locaux imprécis (`hn-5`).
  - **Couverture** : SDK Python demandé (`github-48`), rattachement à Playwright existant (`github-38`), navigateur fourni par l'utilisateur (`github-44`), runtime edge (`github-45`).
  - **Coût du LLM** à l'exécution dans une suite de tests (`hn-0`, `hn-1`).
  - **Risques Browserbase** : bannissement pour violation des CGU sans explication (`reviews-reports-35`), IP partagées bloquées en bloc à cause d'autres clients (`demand-use-cases-31`, `hn-110`), risque de bannissement de compte inchangé (`hn-109` `[P]`), tutoriels minces (`reviews-reports-34`).
- **Changements :**
  - Libretto a quitté Stagehand pour une génération de code au moment du développement, déterministe à l'exécution (`hn-27` `[P]`).
  - « Pourquoi Stagehand si l'agent écrit du CDP ? » : l'éditeur répond tokens, performance, observabilité et politiques (`hn-90`).
  - Stagehand vécu comme du « babysitting » (`reddit-25`).

## 4. Playwright, Playwright MCP et Chrome DevTools MCP (40 items)

- **Salué :**
  - Les modèles connaissent déjà l'API Playwright, si bien qu'un MCP ou une compétence dédiée devient superflu (`hn-42`).
  - DevTools MCP est jugé plus fiable et plus économe que d'autres MCP CDP (`hn-52`).
  - L'inspection de la console et du réseau aide au débogage par agent (`hn-29`).
- **Reproché :**
  - **Tokens** : multipliés par 6 entre deux versions de MCP (`github-106`) ; snapshots trop longs (`github-107`) ; contexte saturé en quelques recherches (`github-108`) ; 11,7 à 13,7 k tokens de définitions d'outils (`hn-98`, `reddit-14` `[P]`) ; DevTools MCP très gourmand (`hn-45`).
  - **Détection** : connexion Google bloquée (`github-109`) ; Chromium lancé par Playwright repéré par ses drapeaux d'automatisation (`reddit-9`).
  - **Conteneurs** : verrou de profil en devcontainer (`github-111`).
  - **Ressources** : Chrome for Testing imposé, plus de 20 Go par instance (`github-104`) ; OOM vers 18-20 sessions concurrentes (`reddit-60`).
  - **Au-delà des tests simples** (`hn-43`).
- **Changements :**
  - Vers Playwright CLI avec une compétence, environ 1-2 k de contexte par interaction (`hn-49`, `hn-50`).
  - Wrappers qui écrivent sur disque ou résument les snapshots (`hn-47` `[P]`, `hn-48`).
  - Indécision entre MCP, CLI, navigateur Codex et Claude for Chrome (`hn-46`).

## 5. Crawl4AI (14 items)

- **Salué :** aucun item d'éloge. Le corpus ne contient que des issues.
- **Reproché :**
  - **Ressources** : fuite mémoire du serveur Docker (`github-69`), processus Chrome orphelins (`github-70`).
  - **Navigation** : « Target closed » (`github-71`, 37), `ERR_ABORTED` en pagination (`github-72`, 40).
  - **Crawl incomplet** : régression DFS (`github-73`), `arun_many` incomplet (`github-74`), défilement infini qui perd des éléments (`github-79`).
  - **Écarts du serveur Docker** : parité avec la bibliothèque (`github-76`), clé Groq ignorée (`github-77`), extraction de tableaux par LLM inaccessible via la file de jobs (`github-81`).
  - **Échecs mal signalés** : extraction LLM vide renvoyée comme un succès (`github-78`), blocage renvoyé en HTTP 500 (`github-80`).
  - **Serverless** : Lambda (`github-75`).

## 6. Crawlee et la plateforme Apify (24 items)

- **Crawlee, reproché :**
  - **Autoscaling** : poussé jusqu'à l'OOM sur Kubernetes (`github-84`) ; pas de backoff sur les 429, d'où une spirale reconnue par les mainteneurs (`github-85` `[P]`) ; domaine martelé en v4 (`github-96`) ; RAM qui monte avec les grands pools de proxys (`github-86` `[P]`).
  - **File de requêtes** : interblocage (`github-87` `[P]`), requêtes marquées traitées sans handler exécuté (`github-91`), verrous expirés qui produisent des doublons (`github-92`).
  - **Anti-bot** : handler Cloudflare qui n'attend pas la fin du challenge (`github-93`).
- **Crawlee, demandé :** crawler hybride HTTP puis navigateur (`github-94`), OpenTelemetry (`github-95`), API web en Python (`github-89`), Scrapling comme option (`github-90`).
- **Apify, salué :**
  - Actors prêts à l'emploi, planification et proxys intégrés qui remplacent des scripts fragiles (`reviews-reports-0`, Trustpilot 4,8 sur 439 avis).
  - Intégrations et marketplace (`reviews-reports-3`, G2 4,7 sur 611 avis).
  - Store de 64 812 Actors, dont 97,1 % payés à l'événement et 2 265 Actors MCP (`reviews-reports-59`).
  - 87 % des runs lancés par API (`reviews-reports-58` `[P]`).
- **Apify, reproché :**
  - **Fiabilité et support** : scrapers incohérents, paramètres ignorés, support muet (`reviews-reports-1`) ; Actors communautaires non maintenus (`reviews-reports-7`).
  - **Prix** : unités de calcul chères en volume et Actors sur mesure difficiles à écrire (`reviews-reports-3`) ; frais de proxy cumulés (`reviews-reports-2`) ; runs courts facturés, limites d'items non respectées, crédit qui expire (`reviews-reports-7`).
  - **Avis** : des récompenses offertes aux auteurs d'avis affaiblissent la note de 4,8 (`reviews-reports-7`).
  - **Débogage et MCP** : débogage dispersé sur plusieurs écrans, reconnexion MCP à refaire à chaque changement d'IDE (`reviews-reports-5`).
- **Changements :** une startup KYC a abandonné son scraper Facebook maison, bloqué sous 80 % de précision, et réduit ses coûts de 30 % par rapport à une API SERP (`demand-use-cases-56` `[P]`).

## 7. Scrapy (11 items)

- **Reproché :**
  - **Politesse** : défauts agressifs qui ont mis des sites à terre ; robots.txt et 403 ignorés (`github-0`, 44 ; `github-1`, 49 ; `github-2`).
  - **Performance** : régression 2.13 (`github-3`).
  - **Durabilité** : `JOBDIR` ne survit qu'à un arrêt propre (`github-5`).
  - **Correction** : doublons après retry (`github-7`), perte possible d'items (`github-8` `[P]`), rate limiting à repenser selon un mainteneur (`github-6` `[P]`).
- **Changements :**
  - Vers une API hébergée quand chaque site exige rendu, proxys et captchas (`reddit-6`).
  - Question sur l'intérêt de Scrapling (`github-9`).
  - Migration de Scrapy et BeautifulSoup vers les scrapers IA, pour la fragilité des sélecteurs, selon un agrégateur de fils Reddit (`reddit-81`).
  - Scrapy reste le runtime quand un agent écrit les configurations par site (`demand-use-cases-33` `[P]`).

## 8. Skyvern (12 items)

- **Salué :** les clients s'en servent surtout pour se connecter, remplir des formulaires et télécharger des fichiers (`hn-23` `[P]`) ; achats multi-sites (`hn-81` `[P]`).
- **Reproché :**
  - **Lenteur** : environ 5 minutes par workflow d'administration (`github-97`), 4-5 minutes pour un formulaire de 6 champs (`github-98`).
  - **Sessions** : cookies non persistés (`github-99`).
  - **Auto-hébergement** : pas de pool de proxys (`github-100`), vue en direct et prise de contrôle réservées au cloud (`github-101`), installation qui échoue sous Windows (`github-102`).
  - **Captchas** : l'agent devine un captcha qu'il ne voit pas (`github-103`).
  - **Conformité** : pages de conformité et BAA introuvables (`hn-25`).

## 9. Scrapling et Spider (10 items)

- **Scrapling :**
  - Sortie standard qui corrompt le protocole MCP (`github-112`), profil temporaire (`github-113`), blocage sur Cloudflare (`github-115`), locale silencieusement en en-US (`github-116`).
  - Demandes : compétence d'agent (`github-114`), capture XHR (`github-117`).
  - Sert de référence pour du code maison généré par Codex (`hn-71`).
- **Spider (Rust) :**
  - Liens extraits avant la fin du rendu JS (`github-118`), logs inondés en CDP distant (`github-119`).
  - Demandes : navigation par LLM (`github-120`), durée par page pour la politesse (`github-121`).
  - Engagement faible : peu de données.

## 10. Lightpanda (7 items)

- Intérêt : Chrome compte parmi les composants les plus gourmands d'une pile MCP (`hn-18`), et le coût navigateur pèse à 20 M pages/jour (`hn-15` `[P]`).
- Scepticisme : le coût LLM domine, et un benchmark RAM sur page simple est trompeur (`hn-14`).
- Manques : documentation des usages sûrs (`hn-17`) ; sans rendu, pas de capture d'écran pour déboguer (`hn-19`) ; l'anti-bot n'est pas une priorité (`hn-20`) ; politesse demandée (`hn-16`).

## 11. API d'accès et proxys

| Service | Salué | Reproché | Sources |
| --- | --- | --- | --- |
| **Bright Data** | Support, fin de la lutte contre captchas et blocages IP (Trustpilot 4,4 sur 969) ; étendue de l'offre | Structure de prix, réponses lentes ; échecs facturés avec en-têtes personnalisés ; configuration complexe, sites qui bloquent encore ; essai via appel commercial, KYC, bande passante imprévisible | `reviews-reports-8`, `reviews-reports-9`, `reviews-reports-10` `[P]`, `reviews-reports-11`, `reviews-reports-12` |
| **Zyte** | Débloque presque tout du premier coup ; remises sur sites difficiles ; écosystème Scrapy | Pas de plafond sans abonnement, facture multipliée par environ 40 ; carte exigée malgré la mention « sans carte » ; prise en main et interface | `reviews-reports-20`, `reviews-reports-21`, `reviews-reports-22`, `reviews-reports-23`, `reviews-reports-24` `[P]` |
| **Oxylabs** | Vitesse des proxys, bande passante illimitée, API cohérente (Gartner 4,7 sur 15) | Trustpilot 3,7 sur 724 : écarts de facturation, remboursements, comptes bloqués (KYC, géo) ; volumes affichés calculés au tarif le plus bas | `reviews-reports-25`, `reviews-reports-26`, `reviews-reports-27`, `reviews-reports-28` |
| **ScrapingBee** | Constant là où des services moins chers échouaient ; un seul appel d'API ; met fin à la gestion des proxys | Crédits multiples par option, plafonds de concurrence ; pas de planificateur ; essai à 1 k crédits ; documentation des cas limites | `reviews-reports-14`, `reviews-reports-15`, `reddit-3`, `reviews-reports-13`, `reviews-reports-16`, `reviews-reports-17` |
| **ScraperAPI** | Simple et fiable | Ventilation des crédits confuse ; multiplicateurs non annoncés ; faible sur cibles difficiles | `reviews-reports-18`, `reviews-reports-19` `[P]` |

## 12. Plateformes de génération d'automatisations et d'agents

Sont regroupés ici Intuned, Libretto, Simplex, Notte, BrowserBook, Smooth et Hyperbrowser.

- **Salué :**
  - Aucune IA à l'exécution, ce qui donne de la fiabilité (`hn-73` `[P]`, `demand-use-cases-26` `[P]`).
  - Des scripts fragiles mais déterministes, préférés aux agents (`hn-41`).
  - Approche validée par un développeur qui a construit un outil interne similaire, où l'agent corrige le script après un échec (`hn-28`).
- **Reproché :**
  - **Prix** : Simplex passe de l'essai gratuit à 2 500 $/mois (`hn-32`) ; crédits Notte opaques avant clarification (`hn-75` `[P]`).
  - **Fiabilité** : échec complet de Notte sur un site hôtelier (`hn-74`) ; Libretto attend la panne en production pour réparer (`hn-30`).
  - **Sécurité et conformité** : Smooth sans documentation sécurité ni conservation nulle par défaut (`hn-91`) ni auto-hébergement (`hn-92`) ; BrowserBook stocke les notebooks sur ses serveurs (`hn-40`) ; données de santé envoyées au LLM (`hn-31`).
  - **Installation** : installation MCP pénible (`hn-78` `[P]`).
- **Pression concurrente :** un agent de code général combiné à une bibliothèque OSS couvrirait l'essentiel (`hn-71`, `demand-use-cases-27`).

## 13. Cloudflare, un gardien plutôt qu'un concurrent (6 items)

- **Réglages par défaut** : blocage par défaut des agents et des robots d'entraînement ; signature Web Bot Auth jugée favorable aux grands laboratoires (`reddit-70`).
- **Paiement à l'exploration** : scepticisme sur le pay-per-crawl (`reddit-72`, `reddit-73`) ; demande d'un protocole ouvert de permissions et de paiement (`hn-112`).
- **Limite de son propre service** : son crawler ne passe pas non plus ses propres protections (`hn-111`).

## 14. Raisons de changement : synthèse

| Mouvement | Raison dominante | Sources |
| --- | --- | --- |
| Code maison ou OSS → API hébergée | Proxys, rendu et captchas deviennent un travail à temps plein ; constance | `reddit-3`, `reddit-6`, `reviews-reports-14`, `reviews-reports-0` |
| API hébergée → code maison ou auto-hébergement | Prix, crédits, finition ; les agents de code rendent le DIY bon marché | `hn-58`, `hn-56`, `hn-63`, `hn-71`, `demand-use-cases-27` |
| Agent à l'exécution → code généré ou rejoué | Non-déterminisme, coût et latence | `hn-27` `[P]`, `hn-0`, `hn-37`, `reddit-62` `[P]`, `hn-100` `[P]` |
| MCP → CLI et compétence | Contexte et tokens | `hn-49`, `hn-50`, `hn-47` `[P]` |
| Plusieurs fournisseurs → un seul point d'accès | Coût total et taux de succès | `demand-use-cases-53` `[P]`, `demand-use-cases-55` `[P]` |
| Panachage de fournisseurs selon la tâche | Aucun ne couvre tout | `hn-66`, `reviews-reports-37` `[P]` |

## 15. Manques récurrents qu'aucun concurrent du corpus ne comble

Ce sont des hypothèses à faire trancher par le labo, pas des avantages démontrés d'Automan :

1. **Cache et rejeu locaux, versionnables, indépendants d'un cloud.** Le cache de Stagehand est lié à Browserbase et la persistance est payante (`hn-87`, `hn-88`, `github-42`) ; c'est un enjeu de lock-in.
2. **Échecs typés, sans faux succès.** Un blocage ne doit pas revenir en 500, une liste vide ne doit pas passer pour un succès, une file ne doit pas se dire traitée sans l'être (`github-80`, `github-78`, `github-91`, `github-29`). Voir `C-zero-false-success`.
3. **Une facturation qui ne compte pas les échecs, avec plafonds durs** ([pricing-signals.md](pricing-signals.md)). Voir `C-cost-per-1k-verified`.
4. **Des sessions et profils qui survivent aux mises à jour** de Chrome et du framework (`github-19`, `github-43`, `github-99`).
5. **Le même produit en auto-hébergé**, anti-bot et vue en direct compris (`reviews-reports-31`, `github-52`, `github-101`).
6. **Une politesse et une identité par défaut** (`github-0`, `github-1`, `hn-62`, `demand-use-cases-31`). Voir `D-identity`.
7. **Une interface agent économe et stable d'une version à l'autre** (`github-106`, `github-22`, `hn-98`). Voir `D-agent-interface`.
