# Signaux de prix : budgets, unités acceptées ou rejetées, plaintes de facturation

**Statut :** v1, recherche documentaire. **Date :** 2026-10-08. **Corpus :** 421 items ([corpus.jsonl](./corpus.jsonl)). Codes mobilisés : `wtp-price-points` (46 items), `cost-unpredictable-runaway` (28), `cost-infra-proxy-spend` (19), `build-vs-buy-switching` (31).

> **Avertissement : la recherche documentaire ne mesure pas la volonté de payer.** Ce document recense ce que des personnes **disent** avoir payé, refusé ou trouvé injuste, ainsi que les prix que des **éditeurs affichent**. Ces signaux sont auto-sélectionnés, puisqu'on se plaint plus volontiers qu'on ne félicite. Ils sont souvent promotionnels (`[P]`) et ne disent rien d'un prix accepté pour Automan. Les budgets des offres de mission paient **du travail humain, pas un logiciel**, et l'échantillon Upwork penche vers le bas du marché (3-70 $/h, 35-700 $ au forfait). La décision `D-pricing` exige des entretiens et un test de prix (§5).

## 1. Points de prix observés, par segment

| Segment | Signal | Valeur | Nature | Source |
| --- | --- | --- | --- | --- |
| Amateurs | Budget total d'un projet perso bloqué par Turnstile | ~50 $/mois | Déclaration d'utilisateur | `reddit-2` |
| Amateurs | Pool résidentiel jugé hors budget | 100 $/mois | Déclaration d'utilisateur | `reddit-12` |
| Amateurs | Firecrawl remplacé par environ 2 700 lignes d'Elixir | ~190 $/mois payés avant | Déclaration d'utilisateur | `hn-58` |
| Amateurs | Deux scrapes simples et une mise à jour Sheets via un agent | ~15 $ | Déclaration d'utilisateur | `reddit-30` |
| Veille prix (PME) | Seuil souhaité face à Prisync (99 $) et Price2Spy (83 $) | < 30 $/mois | Déclaration d'utilisateur | `reddit-43` |
| Veille prix (PME) | Refus des outils à 100 $/mois et plus ; envisage de scripter lui-même | > 100 $/mois refusé | Déclaration d'utilisateur | `reddit-47` |
| Veille prix (PME) | Scraper quotidien avec anti-bot vers Sheets | 35 $ au forfait | Mission | `demand-use-cases-6` |
| Veille prix (grand compte) | Pile DIY remplacée par une API | 530 k$/an → 75 k$/an | Éditeur `[P]` | `demand-use-cases-57` |
| Développeurs d'agents | Coût d'une pile d'agent autonome sur VPS | 30-400 $/mois | Déclarations d'utilisateurs | `reddit-64` |
| Développeurs d'agents | Prix testé pour un scraper agentique structuré | 0,025-0,03 $/requête (question) | Déclaration d'utilisateur | `demand-use-cases-23` |
| Développeurs d'agents | Prix acceptable pour une API de base produits | ~0,01 $/1 k requêtes | Déclaration d'utilisateur | `demand-use-cases-24` |
| Développeurs d'agents | Tarif Notte après plainte sur l'opacité des crédits | 1 ¢/scrape, 2 ¢/étape, 1 ¢/minute ; ~21 ¢ pour 10 étapes et 1 min | Éditeur `[P]` | `hn-75` |
| Développeurs d'agents | Infrastructure navigateur du marché selon Firecrawl | 0,005-0,05 $/minute de navigateur ; ~0,005 $/page | Éditeur `[P]` | `demand-use-cases-49` |
| Développeurs d'agents | Coût par tâche selon le harnais, même modèle | 1,50 $ contre ~10 $/tâche | Éditeur `[P]` | `hn-89` |
| Développeurs d'agents | Boucle d'agent remplacée par un plan JSON rejoué | 0,50-3,00 $ → 0,01-0,05 $/tâche | Éditeur `[P]` | `reddit-62` |
| Développeurs d'agents | Coût agent pour trois soumissions de formulaire, contre ~1 000 captchas humains pour 2 $ | 2,27 $ | Déclaration d'utilisateur | `hn-69` |
| Développeurs d'agents | Solveurs de captcha | ~1-3 $/1 000 | Déclarations d'utilisateurs | `reddit-66` |
| Portails B2B | Plan à la vente directe, signé selon l'éditeur par des entreprises de 5 personnes | 2 500 $/mois | Éditeur `[P]` | `hn-33` |
| Portails B2B | Objection : saut direct de l'essai gratuit à 2 500 $/mois, sans paiement à l'usage | — | Déclaration d'utilisateur | `hn-32` |
| Portails B2B | Pack de démarrage demandé | ~10 $ pour 5 k crédits | Avis | `reviews-reports-16` |
| Portails B2B | Robots Playwright de portails vers GoHighLevel | 3 $/h | Mission | `demand-use-cases-4` |
| Équipes data | Run quotidien de veille sur 500 sites pour des agents | ~310 $/mois, dont 180 $ de calcul | Déclaration d'utilisateur | `reddit-59` |
| Équipes data | Plan Oxylabs à 49 $ | ~42,6 k résultats web génériques au lieu des 98 k affichés | Billet | `reviews-reports-28` |
| Équipes data | Scraping de données publiques | 10 $/h ; 15-35 $/h ; MVP à 100 $ | Missions | `demand-use-cases-2`, `demand-use-cases-9`, `demand-use-cases-0` |
| Entreprise | Équipe interne contre service managé | ~260 k$/an contre ~10 k$/mois | Éditeur `[P]` | `demand-use-cases-58` |
| Entreprise | Consolidation de trois fournisseurs (proxy, captcha, navigateurs) | 22,1 k$ → 5,9 k$/mois | Éditeur `[P]` | `demand-use-cases-53` |
| Entreprise | Revente de jeux de données publics | Brut 0,5-5 $ ; enrichi 5-50 $ par 1 k enregistrements | Déclarations d'utilisateurs | `demand-use-cases-59` |
| Agences d'agents | Cadrage, mise en œuvre, abonnement de maintenance | 1,5-3 k$ ; 5-25 k$ ; 1-5 k$/mois, ou 300-1 500 $/mois par client | Éditeur `[P]` | `demand-use-cases-70` |
| SEO et GEO | Outils de suivi de visibilité dans les IA | 95-828 $/mois | Revue | `demand-use-cases-66` |
| SEO et leads | Leads Google Maps : prix au lead accepté, échantillon de 50 | au lead accepté | Mission | `demand-use-cases-16` |
| SEO et leads | Données Google Maps enrichies, devis pour 2 000 et 5 000 enregistrements | à l'enregistrement | Mission | `demand-use-cases-18` |
| SEO et leads | Google Maps à grande échelle et e-mails | 700 $ au forfait (relevé au contrôle de source) | Mission | `demand-use-cases-15` |
| SEO et leads | SaaS d'automatisation LinkedIn | ~2 k$ le premier mois, en offres à vie | Éditeur `[P]` | `reddit-78` |
| Transverse | Intérêt pour un prix d'environ 2 $/1 k pages pour les scrapers IA | ~2 $/1 k pages | Agrégateur de fils Reddit | `reddit-81` |

**Ordres de grandeur.** Les budgets **individuels**, déclarés par des utilisateurs, se situent entre 30 et 400 $/mois. Pour l'accès simple, les prix **à l'unité** acceptés ou demandés vont de 0,00001 $ (0,01 $ pour 1 k requêtes, `demand-use-cases-24`) à 0,03 $ par requête ou par page. La plupart se situent entre 0,002 et 0,01 $ (`reddit-81`, `demand-use-cases-49` `[P]`, `hn-75` `[P]`). Une tâche d'agent coûte de ~0,20 $ à ~10 $ selon le harnais (`hn-75` `[P]`, `reddit-62` `[P]`, `hn-89` `[P]`). Les prix supérieurs à 1 000 $/mois viennent tous d'éditeurs `[P]`. Dans ce haut de gamme, le seul indice indépendant de paiement est une inscription au palier payant de Browser Use, sans montant connu (`hn-13`).

## 2. Unités rejetées ou acceptées

### 2.1 Rejetées ou sources de plaintes

| Unité ou pratique | Plainte | Sources |
| --- | --- | --- |
| **Multiplicateurs de crédits** selon l'option (JS, proxy premium, capture d'écran, extraction) | Les paliers d'entrée s'épuisent vite ; ventilation confuse ; multiplicateur 5x non annoncé sur des contrats importants ; extraction jusqu'à 10 fois le crédit de base | `reviews-reports-13`, `reviews-reports-18`, `reviews-reports-19` `[P]`, `reviews-reports-32`, `reddit-35` |
| **Facturation des échecs** | Requêtes échouées facturées ; endpoint agent facturé même en échec ; runs très courts facturés | `reviews-reports-10` `[P]`, `reviews-reports-31`, `reviews-reports-7`, `hn-76` |
| **Absence de plafond de dépense** | Facture environ 40 fois l'attendu ; boucle de retry à 40 $ en une nuit ; limites d'items non respectées | `reviews-reports-23`, `reddit-57`, `reviews-reports-7` |
| **Facturation à la bande passante** | Coût mensuel impossible à prévoir ; 10 Go de proxy consommés en deux jours | `reviews-reports-12`, `hn-55` |
| **Crédits qui expirent** sans report | Pénalise les usages par pics ; paiement à l'usage demandé | `reviews-reports-30`, `reviews-reports-7` |
| **Unités de calcul** sur gros volumes | Deviennent chères ; frais de proxy en plus des frais d'Actor | `reviews-reports-3`, `reviews-reports-2` |
| **Accès freiné** (appel commercial, KYC, carte exigée malgré « sans carte ») | Freine l'achat en libre-service ; comptes bloqués à l'inscription | `reviews-reports-12`, `reviews-reports-22`, `reviews-reports-25` |
| **Saut de prix** (gratuit → 2 500 $/mois) | Rien entre les deux pour tester | `hn-32` |
| **Facture d'agent opaque** (tout clic et tout retry facturé) | Les retries et branches échouées pèsent souvent 30 à 40 % du coût d'un run | `reddit-30`, `reddit-65`, `reddit-64` |

### 2.2 Acceptées ou demandées

| Unité ou pratique | Signal | Sources |
| --- | --- | --- |
| **Paiement au résultat réussi** | Oxylabs ne facture que les succès, ce qui rend la dépense prévisible (en revanche, le volume affiché est trompeur) ; SerpApi compte 1 crédit par recherche réussie non mise en cache ; guide de checklist entreprise | `reviews-reports-28`, `reddit-34` `[P]`, `demand-use-cases-64` `[P]` |
| **Paiement au lead ou à l'enregistrement accepté** | Devis au lead valide, à l'enregistrement | `demand-use-cases-16`, `demand-use-cases-18`, `demand-use-cases-59` |
| **Paiement à l'événement** | 97,1 % des 64 812 Actors de l'Apify Store | `reviews-reports-59` |
| **1 crédit = 1 scrape, sans multiplicateur** | Argument de positionnement contre Firecrawl | `hn-64` `[P]` |
| **Paiement à l'usage et petits packs de démarrage** | Demandés face aux crédits qui expirent, aux sauts de prix et aux essais trop maigres | `reviews-reports-30`, `hn-32`, `reviews-reports-16` |
| **Plafonds par tâche, par run, par outil** et budgets de retry | Courtier de dépense extérieur à l'agent | `reddit-58`, `reddit-57` |
| **Prix à l'étendue, prévisible** (services managés) | Mis en avant par un éditeur de services | `demand-use-cases-58` `[P]` |
| **Coût par résultat accepté** comme indicateur interne | Suivi au p95 | `reddit-65` |

## 3. Où va réellement l'argent

1. **L'accès coûte plus cher que le code.**
   - 58,3 % des répondants à l'enquête Apify ont augmenté leurs dépenses de proxy, et 62,5 % leurs coûts d'infrastructure (`reviews-reports-36`, `reviews-reports-39` `[P]`).
   - La facture de proxy résidentiel croît plus vite que la puissance de calcul (`reddit-15`). La bande passante coûte des ordres de grandeur de plus que la preuve de travail d'Anubis (`hn-118`).
   - Un projet Instagram passe de 40 à 2 000 $/mois à 10 k profils (`demand-use-cases-71` `[P]`).
2. **Le LLM coûte plus cher que le navigateur dans les workflows d'agents** (`hn-14`). Les écarts de coût viennent du harnais plus que du modèle (`hn-89` `[P]`, `reddit-61`).
3. **Les retries sont un coût caché.** Les branches échouées pèsent 30 à 40 % du coût d'un run (`reddit-65`) ; l'exemple extrême est une boucle d'environ 200 appels (`reddit-57`).
4. **Le temps humain** reste le poste le plus cité : 6 mois pour fiabiliser une pile de veille prix (`reddit-42`), « babysitting » d'agents (`reddit-0`, `reddit-64`).

Signal contraire : **le coût n'est pas le premier motif de changement d'outil.** Parmi les développeurs ayant changé d'outil IA, 6,4 % l'ont fait pour le coût, contre 25,1 % pour la qualité (`reviews-reports-53`, Stack Overflow 2026). Côté scraping, les bascules vers l'hébergé s'expliquent par la fiabilité et la charge (`reviews-reports-14`, `reddit-3`, `reddit-6`). Les bascules vers le DIY s'expliquent par le prix (`hn-58`, `hn-56`).

## 4. Lecture pour `D-pricing`

| Option | Ce que disent les signaux | Condition |
| --- | --- | --- |
| `verified-result` | La plus alignée sur les plaintes : pas de facturation des échecs, dépense prévisible, unité proche de ce que les acheteurs de leads et de données demandent déjà (au lead, à l'enregistrement, au résultat réussi) | Définir « vérifié » de façon auditable ; contrats d'oracle et faux succès nul, cf. `C-zero-false-success` et `C-cost-per-1k-verified` dans le labo |
| `compute-units` | Le motif le plus critiqué quand il est opaque : multiplicateurs, bande passante, crédits qui expirent | Acceptable seulement avec ventilation par run, plafonds durs et absence de facturation des échecs de la plateforme |
| `subscription-tiers` | Accepté pour les petits besoins de veille, mais la volonté de payer y est basse (moins de 30 à 100 $/mois) ; les sauts de prix sont rejetés | Paliers fins, paiement à l'usage au-delà, plafonds |

L'unité qui revient le plus souvent comme idéal est un **résultat réussi et facturé au réel, avec un plafond dur**. Les signaux ne disent **pas** combien. Ils ne disent pas non plus si les acheteurs accepteraient de payer plus cher par unité vérifiée pour éviter les échecs facturés.

## 5. Ce qu'il faut mesurer ensuite (hors recherche documentaire)

1. **Entretiens structurés** par segment candidat, cinq au minimum par segment :
   - budget actuel ventilé (proxy, captcha, LLM, infrastructure, temps humain) ;
   - dernier incident de facturation ;
   - unité préférée ;
   - questions de seuil de prix trop bas, bon marché, cher, trop cher, sur un résultat vérifié défini.
2. **Test de prix** sur une page d'atterrissage. Comparer deux unités (résultat vérifié contre abonnement plafonné) et mesurer le taux d'inscription sur liste d'attente.
3. **PoC payant** sur 20 à 50 domaines du client, sur le modèle de la checklist entreprise (`demand-use-cases-64` `[P]`). C'est le seul signal de paiement réel.
