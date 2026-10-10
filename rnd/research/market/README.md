# Piste « demande marché » : synthèse pour le registre de décisions

**Statut :** v1, recherche documentaire terminée ; entretiens non commencés. **Date :** 2026-10-08.
**Portée :** ce que des développeurs, des équipes et des acheteurs disent publiquement de leurs douleurs, de leurs besoins, de leurs budgets et des outils concurrents (scraping, automatisation navigateur, agents LLM), de janvier 2025 à octobre 2026.
**Ce que cette piste ne fournit pas :** une mesure de la volonté de payer, de la taille du marché adressable par Automan ou de la prévalence d'une douleur chez les clients. Ces points relèvent des entretiens et des tests de prix ([pricing-signals.md](pricing-signals.md), §5).

| Fichier | Contenu |
| --- | --- |
| [corpus.jsonl](corpus.jsonl) | 421 items canoniques codés et sourcés : une ligne JSON par item |
| [codebook.md](codebook.md) | 30 codes feuilles, protocole de codage, accord inter-codeurs, contrôle des sources, règles d'exclusion |
| [pains-ranking.md](pains-ranking.md) | Classement des douleurs par effectif brut et par score pondéré, variantes de sensibilité, items représentatifs |
| [segments-and-needs.md](segments-and-needs.md) | Besoins par segment, nouveaux usages LLM et agents, exigences acheteurs et conformité |
| [pricing-signals.md](pricing-signals.md) | Budgets et prix observés, unités acceptées ou rejetées, lecture pour `D-pricing` |
| [competitors-strengths-gaps.md](competitors-strengths-gaps.md) | Concurrent par concurrent : ce qui est salué, reproché, et les raisons de changement |
| [interview-guide.md](interview-guide.md) | Guide d'entretien « switch » (JTBD) : recrutement par segment, chronologie du changement, quatre forces, codage |
| [pricing-questionnaire.md](pricing-questionnaire.md) | Questionnaire Van Westendorp et Gabor-Granger sur le résultat vérifié, unité préférée et plafond |
| [recruitment.md](recruitment.md) | Recrutement des entretiens : canaux, annonces prêtes à publier, message direct, suivi anonymisé |

## 1. Constats clés

Chaque constat indique les décisions du registre (`lab/decisions/`) qu'il éclaire, ses preuves (identifiants de `corpus.jsonl`) et sa robustesse. Un constat **éclaire** une décision ; il ne la tranche pas. Les décisions techniques se tranchent par les campagnes du labo. Les décisions produit (`D-segment`, `D-pricing`, `D-feature-priority`) se tranchent après entretiens.

### C1. L'accès est la première douleur, et la continuité de session passe avant la furtivité
`D-identity` · `D-browser-driver` · `D-feature-priority`

- **Rang.** `access-antibot-ip` est premier dans les quatre variantes du classement : 76 items, W 92,4, soit au moins 1,8 fois le deuxième.
- **La continuité de session compte plus que l'empreinte.** Réutiliser une session connectée réelle supprime plus de blocages que le travail sur l'empreinte (`reddit-27`, `reddit-9`, `reddit-55`). Ce qui trahit l'automatisation, ce sont les drapeaux d'automatisation et le profil vierge, pas le mode headless (`reddit-9`).
- **Les barrières courantes sont simples.** Selon Zyte, le WAF seul (31 %) et WAF + JS (environ 20 %) dominent ; la pile complète ne touche que 3,3 % des sites (`reviews-reports-43` `[P]`).
- **Pour Automan.** La priorité est un profil d'identité porté par la session et persistant (option `session-identity-profile` de `D-identity`), plus que la furtivité lourde. Pour `D-browser-driver` : le pilote doit savoir s'attacher en CDP à un Chrome existant, sachant que Chrome 136 et plus bloque le profil par défaut (`github-19`).

### C2. Les sessions authentifiées doivent tenir dans le temps, et les identifiants rester hors du LLM
`D-identity` · `D-agent-interface` · `D-segment`

- **Volume.** 32 items, 4e en W.
- **Persistance.** Elle casse d'une version à l'autre (`github-19`, `github-43`, `github-99`, `github-28`).
- **Attentes.**
  - Des sessions de plusieurs mois et une passation humaine → agent pour la 2FA (`hn-34`).
  - Des identifiants résolus côté outil, jamais vus par le modèle (`demand-use-cases-38`, `hn-35`, `demand-use-cases-68` `[P]`).
- **Signal d'usage.** 14 % des requêtes d'agents mesurées par HUMAN visent des pages de compte ou d'authentification (`demand-use-cases-41`).
- **Pour Automan.** Ajouter à `D-identity` une porte de contrat « identifiant jamais exposé au modèle ». C'est aussi l'argument principal pour l'option `b2b-portals` de `D-segment`.

### C3. Le coût en tokens et la lenteur des agents sont réels, mais leur poids est gonflé par des sources promotionnelles
`D-agent-interface` · `D-ai-layer`

- **Rang.** `agent-token-latency-cost` est 2e en W (44 items), mais tombe au 5e rang sans les items promo ou affiliés.
- **Preuves indépendantes.**
  - Tokens multipliés par 6 entre deux versions de Playwright MCP (`github-106`), par 2 entre deux versions de Browser Use (`github-22`).
  - 11,7 k tokens de définitions d'outils par requête (`hn-98`).
  - Un formulaire de 6 champs prend 4 à 5 minutes (`github-98`).
- **Le coût vient du harnais plus que du modèle.** Avec le même modèle, 1,50 $ contre environ 10 $ par tâche (`hn-89` `[P]`). Le pixel peut coûter moins cher que le DOM dans 4 tâches sur 5 (`reddit-61`).
- **Pour Automan.** Ce constat confirme les métriques `tokens-per-verified-task` et `interface-turns-per-task` déjà prévues. Il demande aussi un contrôle de **stabilité entre versions** de la taille des sorties.

### C4. La demande « compiler une fois, rejouer sans LLM, réparer sur casse » est constante depuis 2025
`D-ai-layer` · `D-feature-priority`

- **Volume.** 30 items `agent-deterministic-replay` (6e en n). Les demandes vont de janvier 2025 (`github-39`, `hn-0`) à septembre 2026 (`hn-87`), et apparaissent dans des offres de mission (`demand-use-cases-19`).
- **Lock-in.** Le cache de Stagehand est lié aux sessions Browserbase et ne sert à rien en local (`hn-87`, `hn-88`, `github-42`).
- **Point faible.** Le gain de coût annoncé (de 0,50-3 $ à 0,01-0,05 $ par tâche, `reddit-62`) est une affirmation d'éditeur `[P]`. Sans les items promo, le code passe du 9e au 13e rang en W.
- **Pour Automan.** C'est le meilleur appui marché de l'option `compiled-replay` de `D-ai-layer` et de la revendication `C-replay-without-llm`. Le cache doit être **local et versionnable**. Le gain de coût reste à démontrer par le labo.

### C5. L'interface agent n'a pas de gagnant : MCP et CLI coexistent
`D-agent-interface`

- **Volume.** 37 items `integ-mcp-agent-tooling` (3e en W, sans promo et avec plafond ; 4e après correction des dates Reddit).
- **Arguments CLI.** Des utilisateurs avancés passent à Playwright CLI avec une compétence pour économiser le contexte (`hn-49`, `hn-50`).
- **Arguments MCP.** MCP garde l'avantage pour les non-développeurs, l'OAuth multi-tenant, l'isolement des identifiants et l'audit (`hn-51`). Il présente aussi des risques d'intégration : sortie standard qui corrompt le protocole (`github-112`), capture d'écran qui casse le contexte (`github-24`).
- **Hôtes.** Claude Code (65,5 %) et Copilot (58,7 %) dominent (`reviews-reports-54`).
- **Pour Automan.** Le marché ne tranche pas `D-agent-interface`. Les mesures du labo (tours et tokens par tâche vérifiée) sont donc le bon arbitre. Il faut tester au moins la combinaison Claude Code avec MCP réduit, avec CLI, et avec code mode.

### C6. Les faux succès silencieux sont une douleur récente et transverse
`D-feature-priority` · `D-extraction` · `D-ai-layer`

- **Rang.** 17 items `silent-false-success`, 18e en n mais 10e en W : récents et engagés.
- **Exemples.**
  - Blocage renvoyé en 500 (`github-80`), handler exécuté sur une page de challenge (`github-93`), file marquée traitée sans exécution (`github-91`).
  - Clics « ok » ignorés (`reddit-1`), fin déclarée trop tôt (`github-30`).
  - Sélecteurs qui renvoient des chaînes vides après une refonte (`hn-101` `[P]`), diff de prix inversé vu comme inchangé (`demand-use-cases-40`).
- **Confiance.** 48 % des développeurs ne font confiance à l'IA que s'ils peuvent vérifier ; 78,7 % jugent l'attribution des sources importante (`reviews-reports-51`).
- **Pour Automan.** La porte `false-success-count` et la revendication `C-zero-false-success` répondent directement à une plainte du marché. C'est un argument de positionnement candidat. Pour `D-extraction`, la qualité doit être **mesurable** : rappel, tableaux, provenance (`github-83`, `github-66`, `demand-use-cases-61` `[P]`).

### C7. Les acheteurs détestent les factures imprévisibles et demandent le paiement au résultat réussi
`D-pricing`

- **Rang.** `cost-unpredictable-runaway` est 5e en W et 2e sans promo (28 items). Le signal ne dépend pas des agrégats d'avis.
- **Rejeté.**
  - Multiplicateurs de crédits (`reviews-reports-13`, `reviews-reports-19` `[P]`).
  - Échecs facturés (`reviews-reports-10` `[P]`, `reviews-reports-31`).
  - Absence de plafond : facture environ 40 fois l'attendu (`reviews-reports-23`), boucle de retry à 40 $ (`reddit-57`).
  - Bande passante (`reviews-reports-12`), crédits qui expirent (`reviews-reports-30`).
- **Demandé.**
  - Paiement au résultat réussi (`reviews-reports-28`, `demand-use-cases-64` `[P]`), au lead ou à l'enregistrement (`demand-use-cases-16`, `demand-use-cases-18`).
  - Plafonds par tâche (`reddit-58`), paiement à l'usage (`hn-32`).
- **Pour Automan.** C'est l'appui le plus net du corpus à l'option `verified-result` de `D-pricing` et à `C-cost-per-1k-verified`. **Le montant reste inconnu** (cf. C10).

### C8. Les exploitants de sites forment une contrainte de premier rang, et les défauts de politesse comptent
`D-identity` · `D-feature-priority`

- **Rang.** 25 items `compliance-site-owner-politeness`, entre le 4e et le 7e rang selon la variante. Les engagements sont les plus forts du corpus : 462 et 357 points HN (`hn-125`, `hn-126`), 44 et 49 sur GitHub (`github-0`, `github-1`).
- **Les défauts sont mis en cause.** Les défauts de Scrapy ont mis des sites à terre (`github-1`). Les IP partagées de navigateurs cloud sont bloquées en bloc (`demand-use-cases-31`, `hn-110`). Des acheteurs veulent maîtriser l'étiquette de crawl (`hn-62` `[P]`).
- **Pour Automan.** Ce constat confirme les profils `identified` et `standard` de l'ADR-002, avec robots.txt et throttling **actifs par défaut**. C'est aussi un argument pour les acheteurs entreprise (C9).

### C9. Pour l'entreprise, la conformité est le premier critère d'achat, et une partie ne coûte presque rien si elle est conçue tôt
`D-segment` · `D-feature-priority`

- **Rang.** La conformité est le premier code du segment `enterprise` (15 items sur 29).
- **Exigences.** SOC 2, DPA, rétention, sous-traitants (`demand-use-cases-62` `[P]`, `demand-use-cases-64` `[P]`). Provenance par enregistrement et EU AI Act (`demand-use-cases-63` `[P]`, `demand-use-cases-39`). BAA et conservation nulle (`hn-25`, `hn-31`, `hn-91`). Exécution locale (`demand-use-cases-10`, `hn-92`).
- **Ordre des priorités.** L'acheteur qui a du budget parle de conformité avant la qualité (`demand-use-cases-59`).
- **Pour Automan.** Le journal d'audit par action et la provenance par enregistrement relèvent de l'architecture, pas de la certification. Ils sont peu coûteux s'ils sont prévus dès le départ. SOC 2 est hors périmètre tant que `D-segment` n'a pas retenu l'entreprise.

### C10. Segments : les développeurs d'agents parlent le plus, les portails B2B affichent les prix les plus élevés, et aucun n'est prouvé
`D-segment` · `D-pricing`

- **Développeurs d'agents.** 44 % du corpus (186 items), mais une volonté de payer faible et une forte pression du DIY via les agents de code (`hn-71`, `demand-use-cases-27`). 6,4 % seulement changent d'outil pour le coût (`reviews-reports-53`).
- **Portails B2B.** Les seuls prix élevés, 2 500 $/mois (`hn-33`) et les volumes de clients d'Intuned (`demand-use-cases-26`), viennent d'éditeurs `[P]`.
- **Veille prix e-commerce.** Bimodale : moins de 30 $/mois chez les PME (`reddit-43`) ; croissance la plus rapide selon Mordor (`demand-use-cases-45`).
- **Équipes data.** Budgets réels en proxy et infrastructure, mais 46,7 % font tout en interne (`reviews-reports-38` `[P]`).
- **Pour Automan.** Hypothèse à tester en entretiens, pas une conclusion : l'**adoption** passerait par les développeurs d'agents (OSS, MCP, CLI), la **monétisation** par des charges de travail à résultat vérifié (portails, données). Voir [segments-and-needs.md](segments-and-needs.md), §5.

### C11. Changer d'outil : on va vers l'hébergé pour la fiabilité et on en part pour le prix ; l'auto-hébergement doit être complet
`D-pricing` · `D-segment`

- **Vers l'hébergé.** Quand la gestion des proxys devient un travail à temps plein (`reddit-3`, `reddit-6`, `reviews-reports-14`).
- **Hors de l'hébergé.** Pour le prix (`hn-58`, `hn-56`, `hn-63`).
- **Auto-hébergement.** 20 items `selfhost-parity-lockin`. L'OSS est perçu comme une rampe vers le cloud (`github-42`). Il manque l'anti-bot en auto-hébergé (`reviews-reports-31`), la vue en direct (`github-101`), les proxys (`github-52`).
- **Pour Automan.** La frontière entre open source et payant doit être lisible. **Retirer l'anti-bot ou le cache de la version auto-hébergée est précisément ce qui est reproché.**

### C12. Performance mémoire et reprise : peu exprimées, donc nécessaires mais pas différenciantes
`D-browser-driver` · `D-feature-priority`

- **Rang.** `perf-memory-concurrency` (15 items, 22e en W) et `durability-resume` (5 items, dernier) sont en bas du classement.
- **La mémoire ne se plaint qu'à volume.** OOM vers 18-20 sessions (`reddit-60`), plus de 20 Go avec Chrome for Testing (`github-104`). Le coût du LLM domine celui du navigateur dans les workflows d'agents (`hn-14`).
- **La reprise apparaît comme exigence** de cahier des charges (`demand-use-cases-1`, `demand-use-cases-10`, `github-5`).
- **Pour Automan.** Pour `D-browser-driver`, un gain de vitesse ou de mémoire **ne suffit pas comme argument**. La conformité d'actionnabilité et la stabilité des sessions comptent davantage. La décision `D-durable-runtime` reste une affaire de correction, pas de positionnement.

## 2. Matrice décisions × constats

| Décision | Constats | Ce que la piste marché apporte | Ce qui reste à mesurer |
| --- | --- | --- | --- |
| `D-segment` | C2, C9, C10, C11 | Volume de voix, douleurs par segment, signaux de prix (biaisés) | Entretiens : budget réel, urgence, cycle d'achat ; au moins 5 par segment candidat |
| `D-pricing` | C7, C10, C11 | Unités rejetées ou acceptées ; `verified-result` le mieux aligné | Montant : entretiens, test de prix sur page d'atterrissage, PoC payant |
| `D-feature-priority` | C1, C4, C6, C8, C9, C12 | Backlog classé par douleur (`pains-ranking.md` §2, variante sans promo) | Pondération par segment une fois `D-segment` tranché |
| `D-browser-driver` | C1, C12 | Rattachement CDP à un Chrome réel, stabilité des profils ; vitesse peu différenciante | Campagnes d'actionnabilité déjà prévues |
| `D-ai-layer` | C3, C4, C6 | Demande constante de rejeu compilé ; cache local exigé | `usd-per-1k-verified`, faux succès |
| `D-agent-interface` | C2, C3, C5 | Pas de gagnant marché ; hôtes dominants ; exigences sur les identifiants | `interface-turns-per-task`, tokens par tâche vérifiée |
| `D-identity` | C1, C2, C8 | Continuité de session avant furtivité ; défauts polis ; identifiants hors LLM | `identity-coherence`, `policy-compliance` |
| `D-extraction` | C6 | Qualité mesurable : rappel, tableaux, provenance, diffs justes | `extraction-f1`, précision et rappel par champ |

## 3. Méthode

1. **Collecte.** Cinq collecteurs indépendants, en lecture seule :
   - **GitHub** : 12 dépôts et des recherches par mot-clé ;
   - **Reddit** : via un site tiers d'instantanés ;
   - **HN** : API Algolia ;
   - **Avis et rapports** : Trustpilot, synthèses G2, miroirs Capterra, Gartner, Product Hunt, rapports d'éditeurs ;
   - **Demande et cas d'usage** : Upwork, Workana, études de cas, guides d'achat.

   Les collecteurs ont remonté 465 items selon leurs notes. Le workflow a transmis 429 items canoniques.
2. **Codage.** Un premier codeur LLM attribue 1 à 3 codes parmi 30 feuilles ([codebook.md](codebook.md)). Un second codeur indépendant recode un échantillon stratifié de 120 items.
3. **Contrôle de sources** sur 60 items : URL, date, fidélité du résumé. Les 8 échecs sont exclus, ce qui laisse **421 items**.
4. **Classement.**
   - Effectif brut n et score pondéré `W = Σ (1 + ln(1 + engagement)) × 0,5^(âge_mois / 6)` au 2026-10-08.
   - Variantes : sans promo ni affiliés, engagement plafonné à 50, correction du biais de date Reddit.

Répartition du corpus final :

- **Sources** : HN 127, issues GitHub 113, Reddit 68, rapports 29, billets 28, avis 26, offres de mission 23, discussions GitHub 6, autre 1.
- **Nature** : `pain` 138, `feature-request` 83, `use-case` 65, `pricing-signal` 51, `statistic` 50, `switching` 21, `praise` 13.
- **Période** : de 2024-11 (un seul item) à 2026-10 ; 262 items datent de 2026.
- **Promo** : 98 items `promo` ou `vendor_affiliated`, conservés et marqués.

## 4. Accord inter-codeurs et contrôle des sources

| Contrôle | Résultat |
| --- | --- |
| Accord sur le code primaire (n = 120) | **83,3 %** (100/120), sans correction du hasard |
| Jaccard moyen sur l'ensemble des codes (n = 120) | **0,838** |
| Sources vérifiées | 60 : **52 réussies, 8 échecs (13,3 %), exclus** |
| Motifs d'échec | 7 dates : 5 dates d'instantané Reddit postérieures au post, de 4 à environ 102 jours (`reddit-5`, `reddit-10`, `reddit-23`, `reddit-69`, `reddit-71`), et 2 mois d'avis invérifiables (`reviews-reports-4`, `reviews-reports-6`) ; 1 résumé infidèle (`github-4` : un contournement existait) |
| Réserves sur les items conservés | 8 notes `check_note`, par exemple généralisation à partir d'un seul auteur (`github-94`) ou qualificatif non établi (`demand-use-cases-4`, `demand-use-cases-15`) |
| Biais résiduel | Les dates Reddit sont des dates d'instantané : jusqu'à environ 3 mois de décalage, poids de récence jusqu'à environ 1,5 fois trop fort. Corrigé en test : mêmes dix codes de douleur en tête, permutation des rangs 3 à 5 |

**Lecture.** L'accord suffit pour classer des familles de douleurs. Il ne départage pas deux codes à moins d'environ 15 % d'écart de score. Le taux d'échec de 13,3 % concerne surtout les **dates** et non le contenu : 1 seul résumé infidèle sur 60.

## 5. Couverture et trous

D'après les notes des collecteurs :

- **GitHub.**
  - L'API de recherche plafonne à 30 résultats par requête, donc la longue traîne manque.
  - Les commentaires n'ont pas été lus en profondeur.
  - Il y a peu de discussions explicites de prix, et Scrapling et Spider ont peu d'engagement.
  - Les issues Playwright ont été filtrées sur le scraping, la mémoire et les agents.
- **Reddit.**
  - Aucun accès direct (403, refus du domaine, Anubis), et aucun contournement tenté.
  - Tout passe par des instantanés tiers. La couverture va de mars à septembre 2026 seulement, sans 2025.
  - L'engagement correspond aux commentaires capturés, sans les votes. Les sous-reddits ne sont pas tagués.
  - r/dataengineering est peu couvert. Il y a peu de plaintes directes sur les prix d'Apify, Browserbase ou Bright Data.
- **HN.**
  - Steel et Kernel sont quasi absents. Aucun item Apify : les fils trouvés étaient promotionnels.
  - Crawl4AI est peu discuté. Les fils de septembre-octobre 2026 sont encore petits.
- **Avis et rapports.**
  - Les pages G2 et capterra.com étaient inaccessibles : on a utilisé la synthèse IA de G2 et des miroirs régionaux de Capterra.
  - Les pages Trustpilot de Firecrawl, ScrapingBee et ScraperAPI étaient inaccessibles. Les avis indépendants sur Firecrawl sont minces.
  - Les synthèses écrites par des concurrents (Thunderbit) sont marquées `[P]`.
  - Le PDF Bright Data 2026 était illisible : seuls les titres sont cités. L'enquête Apify ne publie pas la taille de son échantillon.
  - Plusieurs items non datés ont été écartés.
- **Demande.**
  - Les offres Upwork penchent vers septembre-octobre 2026 et vers les bas budgets.
  - G2, Capterra, LinkedIn et X n'ont pas été couverts.
  - Les études de cas sont surtout des éditeurs. La volonté de payer exprimée par des utilisateurs finaux est mince.
  - Les segments relèvent du jugement du codeur.
- **Absent de toute la piste :**
  - les entretiens ;
  - les communautés fermées (Discord, Slack) ;
  - les voix des achats en entreprise, hors guides d'éditeurs ;
  - les sources non anglophones, hormis deux cas cités en anglais (Turquie, Brésil) ;
  - les marchés asiatiques.

## 6. Limites

1. **Voix publique ≠ marché.** Les développeurs d'agents sont surreprésentés (44 %) parce que leurs outils ont des trackers publics et que HN les discute. Les acheteurs entreprise et les équipes data parlent peu en public.
2. **Engagement hétérogène.** Réactions, points, commentaires et nombres d'avis ne sont pas comparables entre eux. W est ordinal, et les agrégats d'avis pèsent lourd (68 % du poids de `dx-setup-docs`).
3. **Sources promotionnelles.** Les statistiques de marché sont à 66 % promo ou affiliées. Les prix élevés et les gains annoncés viennent d'éditeurs.
4. **Codage par LLM.** Le premier codeur est un LLM et l'accord avec le second codeur est bon, mais aucune validation humaine de l'échantillon n'est documentée dans cette synthèse. Le détail des désaccords n'est pas disponible.
5. **Recherche documentaire ≠ volonté de payer.** Aucun chiffre de ce dossier ne doit servir de prix Automan sans entretiens ni test (`D-pricing`, note du registre).
6. **Instantané daté.** Corpus figé au 2026-10-08, avec une demi-vie de 6 mois. Une mise à jour suppose un nouveau codage et un nouveau contrôle (codebook §6).

## 7. Prochaines actions de la piste

1. **Entretiens** : guide dérivé de C1 à C12. Au moins 5 entretiens par segment candidat de `D-segment` ; questions de budget ventilé et de seuils de prix sur un « résultat vérifié » défini.
2. **Test de prix** : page d'atterrissage comparant `verified-result` et un abonnement plafonné.
3. **Rafraîchissement trimestriel** : même protocole, en ajoutant les sources manquantes (§5).
