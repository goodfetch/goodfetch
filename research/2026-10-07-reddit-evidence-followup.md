# Reddit : preuves et limites pour Stagehand, Crawlee et Spider-rs

Relevé effectué le 7 octobre 2026. Ce document vérifie les témoignages Reddit citables ; il ne classe pas les frameworks et ne réduit pas l'ambition Rust d'Automan.

## Résultat

Aucune comparaison Reddit directe et suffisamment documentée entre **Stagehand, Crawlee et Spider-rs**, ni aucun protocole commun permettant de les classer, n'a été trouvé dans ce balayage. Cela décrit les résultats de cette recherche limitée, pas une preuve d'absence de toute discussion sur Reddit. Les pages d'accueil de Reddit ou d'un fournisseur ne peuvent pas étayer un « consensus Reddit ».

Il existe des retours individuels sur Stagehand contre Browser Use, mais ils sont contradictoires, les versions sont généralement absentes et le cas d'usage est souvent peu décrit. Le fil aux chiffres les plus détaillés compare des stacks complètes, dont **Browserbase + Stagehand**, et non Stagehand isolé. Crawlee apparaît dans un lancement officiel, une discussion ancienne sur ses erreurs et un benchmark annoncé par l'auteur d'un autre crawler Rust. Ce dernier est **Crawlyx, pas Spider-rs**. Les commentaires sur Skyvern trouvés ici comprennent des interventions de personnes présentant leur propre produit.

Conclusion permise : ces fils aident à construire des scénarios de validation — sessions authentifiées, récupération après dérive de page, mémoire des navigateurs, coût des appels au modèle et preuve de réussite. Ils ne démontrent ni une supériorité générale de Stagehand ni une infériorité de Crawlee ou de Spider-rs.

## Méthode et provenance

Le skill Exa Search, ses références `searching.md` et `source-quality.md` ont servi à séparer expérience déclarée, promotion, métriques vérifiables et opinion. Cinq requêtes Exa ont couvert : comparaison directe du trio ; utilisation de Stagehand contre Browser Use/Skyvern ; expérience de Crawlee/Spider-rs à grande échelle ; comparaison Crawl4AI/Scrapling ; restriction explicite à Reddit.

| Requête Exa | `numResults` demandé | Résultat utile pour Reddit |
| --- | ---: | --- |
| Comparaison directe Stagehand/Crawlee/Spider-rs | 10 | Documentation et comparatifs hors Reddit |
| Stagehand contre Browser Use/Skyvern, expérience réelle | 10 | Comparatifs éditoriaux et pages de fournisseurs |
| Crawlee/Spider-rs, production et métriques | 10 | GitHub, documentation, blogs, un fil Hacker News |
| Crawl4AI/Scrapling et frameworks voisins | 10 | Benchmarks GitHub et articles hors Reddit |
| `site:reddit.com` avec les outils concernés | 15 | Aucun résultat |

**`sources_reviewed: 55`**, selon la convention Exa : somme des `numResults` demandés, y compris la requête vide. Ce compteur ne signifie pas que 55 sources ont été lues. Exa a fourni 40 entrées URL avant déduplication, aucune URL Reddit. La récupération Exa de six fils connus a échoué avec `SOURCE_NOT_AVAILABLE`.

La recherche web de complément a privilégié le domaine `reddit.com`, puis l'ouverture des fils exacts. Dix fils retenus ont été ouverts, avec six permaliens de commentaires ouverts séparément. Les auteurs ci-dessous viennent des pages ouvertes, pas d'une supposition à partir du titre. Les essais de récupération JSON publique de deux fils ont renvoyé HTTP 403 ; aucun timestamp ou champ caché n'en a été obtenu.

Les dates absolues proviennent soit du résultat de recherche du fil, soit du footer Reddit lorsqu'il donne le jour du post. Pour les fils du 22 mai et du 13 août 2026, elles proviennent du relevé de recherche antérieur conservé dans [stagehand-usage.md](./stagehand-usage.md). L'ouverture actuelle ne fournit pour ces deux fils que des dates relatives. Les affichages relatifs varient selon les pages ou les versions mises en cache ; ils n'ont pas servi à calculer de fausses dates exactes. La date du fil n'est pas automatiquement celle de chacun de ses commentaires.

## Témoignages effectivement lus

### 1. Stagehand contre Browser Use : retour favorable, cas insuffisamment décrit

- **Source :** [Stagehand vs Browser Use.. which one actually works for production agents?](https://www.reddit.com/r/AI_Agents/comments/1slvuta/stagehand_vs_browser_use_which_one_actually_works/), r/AI_Agents, **15 avril 2026** dans la recherche ; **u/Mammoth_Disk_6803**.
- **Expérience déclarée :** deux semaines de clics Browser Use sur des éléments inexistants, puis davantage de sessions terminées avec Stagehand. Documentation critiquée ; l'auteur reconnaît que son cas peut ne pas se généraliser.
- **Versions / contexte :** aucun modèle, site, scénario, nombre de runs ou réglage des sessions.
- **Métriques :** aucune proportion de succès, trace ou corpus. Les deux semaines ne mesurent pas une fiabilité comparative.
- **Quality :** essai personnel déclaré, sans promotion visible dans le post ; identité professionnelle non vérifiée.

Dans [ce commentaire lu](https://www.reddit.com/r/AI_Agents/comments/1slvuta/comment/ogboy1n/), **u/opentabs-dev** dit avoir abandonné les deux outils pour des difficultés d'authentification et promeut son extension/MCP OpenTabs. Aucune configuration fournie ne démontre une impossibilité de réutiliser l'authentification. **Quality :** expérience déclarée, intérêt promotionnel visible ; comparaison non indépendante.

### 2. Browserbase + Stagehand : chiffres opérationnels déclarés, effet confondu

- **Source :** [We rebuilt our scraping stack 4 times in 18 months. Is this common?](https://www.reddit.com/r/WebScrapingInsider/comments/1tkqi5e/we_rebuilt_our_scraping_stack_4_times_in_18/), r/WebScrapingInsider ; **u/yuuliiy** ; **22 mai 2026** selon le relevé antérieur, âge relatif seulement à l'ouverture actuelle.
- **Expérience déclarée :** SaaS de veille de prix, **400 sites, 2 millions de pages/jour**, quatre stacks en 18 mois : requests/BeautifulSoup, Playwright auto-hébergé, ScrapingBee, Browserbase + Stagehand.
- **Chiffres déclarés :** quatre semaines sur la dernière stack : **1,2 % d'échecs**, **p99 ~6 s**, **coût mensuel -45 %** par rapport à la combinaison précédente.
- **Reproductibilité :** aucun corpus, relevé brut, définition du succès, facturation, version, code ou machine. SDK, hébergement et exploitation changent ensemble ; effet Stagehand impossible à isoler.
- **Quality :** récit de production revendiqué, entreprise non identifiée, chiffres non auditables ; aucune affiliation commerciale établie.

Les réponses lues comprennent **u/0xMassii**, promoteur de son dépôt Webclaw, et **u/CapMonster1**, proposant un solveur CAPTCHA. [u/DaKheera47](https://www.reddit.com/r/WebScrapingInsider/comments/1tkqi5e/comment/onapp63/) demande de classifier les échecs. Aucun de ces commentaires ne valide indépendamment les chiffres.

### 3. Coût des primitives Stagehand : expérience ancienne et newsletter

- **Source :** [Stagehand - Node package to control browser with natural language](https://www.reddit.com/r/node/comments/1jtdwpz/stagehand_node_package_to_control_browser_with/), r/node ; post du **7 avril 2025**, jour confirmé par le footer Reddit du [commentaire de l'auteur](https://www.reddit.com/r/node/comments/1jtdwpz/comment/mltjyi6/) ; auteur **u/opensourcecolumbus**.
- **Expérience déclarée :** recherche d'une brique pour un agent après des essais de Computer Use/Operator ; préférence pour des primitives atomiques ou une exécution autonome selon le besoin.
- **Métriques annoncées :** environ **7 000 tokens** pour une automatisation en trois étapes atomiques, contre **plus de 500 000** pour l'exécution du même objectif en une seule demande.
- **Versions / reproductibilité :** aucune version de Stagehand, tâche détaillée, relevé de tokens ou protocole fourni dans le commentaire. `gpt-4o-mini` apparaît comme exemple de modèle, pas comme preuve du modèle utilisé pour ces deux mesures. La déclaration ancienne sur l'absence de support des modèles open source n'est pas une description de la version actuelle.
- **Quality :** expérience personnelle déclarée avec un contraste mesuré annoncé, mais promotion explicite de la newsletter OpenSourceDiscovery et d'un article externe ; aucune validation indépendante des chiffres dans le fil.

### 4. Retour moins favorable sur Stagehand

- **Source :** [commentaire exact de u/joPrincess1](https://www.reddit.com/r/AI_Agents/comments/1uhvp85/comment/oubl84a/) dans [How are you using browser agents?](https://www.reddit.com/r/AI_Agents/comments/1uhvp85/how_are_you_using_browser_agents/), post de **u/HagiBoi** daté du **28 juin 2026** dans la recherche.
- **Expérience déclarée :** Browser Use jugé imprévisible, Stagehand nécessitant trop de surveillance, puis préférence pour Opera Browser CLI. L'auteur décrit des usages personnels et professionnels : messages WhatsApp, veille Reddit, options de vols et synthèse de documents d'équipe.
- **Versions / métriques :** non précisées ; aucun nombre de tâches ratées, délai ou coût. Les tâches des différentes solutions ne sont pas appariées.
- **Quality :** expériences et tâches concrètes déclarées, avis défavorable qui empêche de présenter l'échantillon comme unanime ; texte centré ensuite sur la promotion d'une autre solution, affiliation non déclarée et non vérifiée.

### 5. Modèles locaux, sessions et Skyvern : mélange d'essais et de vendeurs

- **Source :** [Stagehand / BrowserUse / Logged In experiences for agents](https://www.reddit.com/r/LocalLLaMA/comments/1i0toui/stagehand_browseruse_logged_in_experiences_for/), r/LocalLLaMA ; **14 janvier 2025** ; auteur **u/VigilOnTheVerge**.
- **Expérience :** le post demande une comparaison pour des sites authentifiés ; il ne publie pas les résultats du benchmark qu'il envisage.
- **Commentaire utilisateur lu :** [u/alphakue](https://www.reddit.com/r/LocalLLaMA/comments/1i0toui/comment/m72ypjd/) dit n'avoir pas réussi avec des modèles locaux inférieurs à 14B, hormis un résultat partiel avec `llama3-groq-tool-use:8b-q4_K_M`, et évoque une intégration Bedrock cassée à cette époque. Aucun commit, version ou run complet.
- **Interventions commerciales lues :** **u/Financial_Radio_5036** se présente comme créateur de Browser Use, évoque ses tests `qwen2.5:latest 7b` et annonce son cloud. **u/do_all_the_awesome** présente l'intégration Skyvern/Bitwarden et propose Skyvern Cloud. Ce sont des affirmations de personnes présentant leur solution, pas des témoignages indépendants de clients Skyvern.
- **Quality :** essai local spécifique pour alphakue, sans métrique reproductible ; intérêt fournisseur explicitement déclaré ou visible pour les deux autres. Informations anciennes et dépendantes des modèles : inutilisables comme jugement sur les versions d'octobre 2026.

### 6. Browser Use contre Crawl4AI : le titre dépasse la comparaison réelle

- **Source :** [Browser-use vs Crawl4ai](https://www.reddit.com/r/AI_Agents/comments/1iyw8l6/browseruse_vs_crawl4ai/), r/AI_Agents ; **26 février 2025** ; auteur **u/Effective-Street1634**.
- **Expérience déclarée :** Browser Use dévie de la tâche en mode planner, omet parfois l'extraction et échoue sur des tâches complexes. L'auteur veut lire des messages, répondre à des clients et envoyer une synthèse ; il demande ensuite si Crawl4AI pourrait convenir.
- **Portée :** ce post ne montre pas que l'auteur a essayé Crawl4AI sur ces mêmes tâches. Les réponses visibles comprennent une promotion de rtrvr.ai et des commentaires techniques ; plusieurs commentaires sont supprimés.
- **Quality :** problème utilisateur concret, mais une seule solution testée et aucune version ou métrique. Le titre « vs » ne transforme pas la question en comparaison directe.

### 7. Scrapling et Crawl4AI dans un agent de recherche auto-hébergé

- **Source :** [Self-hosted research agent: which tools do you run?](https://www.reddit.com/r/AI_Agents/comments/1wejc1o/selfhosted_research_agent_which_tools_do_you_run/), r/AI_Agents ; **12 septembre 2026** dans la recherche et âge de 25 jours sur le permalink ouvert ; auteur **u/F1nch74**.
- **Expérience déclarée :** l'auteur utilise SearXNG et Scrapling sur ses VM et rencontre des détections de bot/CAPTCHA ; il cherche une solution sans API payante ni service cloud.
- **[Commentaire effectivement lu](https://www.reddit.com/r/AI_Agents/comments/1wejc1o/comment/p9e8w89/) :** **u/NationalCry9555** dit utiliser Crawl4AI depuis quelques mois, apprécier son rendu JavaScript et trouver Firecrawl auto-hébergé lourd pour ses besoins.
- **Versions / métriques :** aucune. Aucun test commun démontrant que Crawl4AI résout le blocage rencontré avec Scrapling. Plusieurs posts du même titre retrouvés dans d'autres subreddits sont des republications du même message, pas des utilisateurs indépendants supplémentaires.
- **Quality :** utilisation personnelle déclarée des outils, avec cas d'usage et contrainte économique ; sites et réglages absents, expérience non auditée.

### 8. Crawler Rust contre Crawlee/Crawl4AI : Crawlyx, pas Spider-rs

- **Source :** [Built a Rust web crawler after getting fed up with Firecrawl's pricing and crawl4ai's RAM usage](https://www.reddit.com/r/SideProject/comments/1uc1gfo/built_a_rust_web_crawler_after_getting_fed_up/), r/SideProject ; **21 juin 2026** dans la recherche ; **u/SameerVers3**.
- **Expérience déclarée :** crawler construit pour un agent client ; format Crawlee inadéquat pour son pipeline.
- **Métriques annoncées :** même site/machine, 200 pages : Crawlyx **53 s**, Crawlee **110 s**, Firecrawl **210 s**, Crawl4AI **328 s** ; mémoire stable revendiquée pour Crawlyx, plus de 1 Go pour Crawl4AI. Crawlyx **v1**, sans rotation de proxies ni anti-bot.
- **Reproductibilité :** [dépôt Crawlyx](https://github.com/SameerVers3/crawlyx) donné, non audité ici ; aucun site, commande, versions concurrentes, relevé brut, répétitions ou sorties équivalentes.
- **Réponse lue :** **u/SnooMuffins9844**, déclarant travailler chez Firecrawl, demande si les pages sont statiques ou des SPA ; question non résolue dans la vue ouverte.
- **Quality :** benchmark de l'auteur de l'outil, intérêt produit direct, résultat non auditable. Ces nombres ne concernent pas Spider-rs.

### 9. Crawlee : lancement et critique précisément rattachée à une ancienne version

- **Source :** [Crawlee - the web scraping and browser automation library](https://www.reddit.com/r/webscraping/comments/wvk5za/crawlee_the_web_scraping_and_browser_automation/), r/webscraping ; lancement du **23 août 2022** ; auteur **u/B4nan**, texte parlant au nom de l'équipe Apify.
- **Nature du post :** annonce du framework et de ses fonctionnalités. Les félicitations ou intentions d'utilisation ne sont pas des retours de production.
- **Commentaire lu :** **u/mdaniel** critique le contexte d'erreur fourni par le code et donne plusieurs liens vers `http-crawler.ts` au tag **v3.0.4**. **u/B4nan** répond que les erreurs intermédiaires sont abrégées avant retry, que le dernier échec conserve la trace complète et que `CRAWLEE_VERBOSE_LOG` permet d'en obtenir davantage.
- **Portée :** une discussion technique ancienne associée à une version précise, pas un test de la version actuelle. Pas de comparaison avec Stagehand ou Spider-rs et pas de taux de réussite.
- **Quality :** annonce fournisseur ; critique fondée sur des liens au code versionné et réponse de mainteneur. Utile pour identifier un contrat d'observabilité à vérifier, sans extrapoler un défaut actuel.

### 10. SDK, CLI et cloud : confusion d'intégration, pas benchmark

- **Source :** [confused about browserbase product lines.. stagehand sdk or browse cli](https://www.reddit.com/r/LLMDevs/comments/1vn530v/confused_about_browserbase_product_lines/), r/LLMDevs ; auteur **u/CloudTheoryqa** ; **13 août 2026** selon le relevé antérieur, date absolue non exposée par l'ouverture actuelle.
- **Expérience déclarée :** intégration d'un navigateur dans un agent local Hermes ; confusion entre Stagehand SDK, Browse CLI, recettes Browse.sh et hébergement Browserbase. Environ 40 minutes consacrées au mauvais quickstart selon l'auteur.
- **Portée :** besoin d'ergonomie et de séparation claire des couches. Versions absentes ; pas de comparaison de réussite, d'anti-bot ou de consommation mémoire entre frameworks.
- **Quality :** problème d'intégration personnel détaillé, aucun protocole comparatif. Plusieurs réponses sont supprimées ; le fil restant ne prouve ni une popularité relative des produits ni un consensus sur leur fiabilité.

## Résultats de recherche non transformés en preuves

Un extrait de [Thoughts on best ai web crawlers or scraping apis?](https://www.reddit.com/r/aiagents/comments/1nl6skn/thoughts_on_best_ai_web_crawlers_or_scraping_apis/) indique un problème de bouton de consentement avec Crawl4AI, ainsi que des recommandations de services/proxies. Ce fil n'a pas été ouvert séparément dans ce relevé : extrait de recherche seulement, exclu des témoignages vérifiés.

Les résultats Exa comprennent des comparatifs de fournisseurs et les benchmarks du dépôt Spider-rs. Ils peuvent pointer vers du code ou une méthodologie à auditer, mais ce ne sont pas des commentaires Reddit indépendants. Les expressions « Rust très rapide », « prêt pour la production » ou « consensus Reddit » trouvées dans des synthèses ne constituent pas une preuve de performance comparative.

## Ce qui reste à démontrer pour Automan

Les besoins évoqués par les témoignages doivent être vérifiés sur les versions exactes, les modèles et les tâches d'Automan : état authentifié qui expire, navigation dynamique, extraction avec schéma, reprise après crash, changement du DOM, blocage explicite, erreur classifiée et preuve de l'état final. Les mesures doivent distinguer rendu/HTTP, fournisseur de navigateur, proxies, orchestration et décisions LLM.

Un classement demanderait un corpus commun, des sorties équivalentes, les versions et configurations figées, plusieurs répétitions, les échecs conservés et les coûts complets. Aucun des fils retenus ne fournit cet ensemble pour Stagehand/Crawlee/Spider-rs. **Le travail de validation reste ouvert ; ces témoignages ne déterminent ni l'architecture Rust finale ni le plafond de capacité d'Automan.**
