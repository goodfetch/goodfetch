# Audit des benchmarks publics : Stagehand, Crawlee et Spider-rs

Relevé du 7 octobre 2026. Ce document examine des sources et des scripts ; il ne présente pas un benchmark comparatif exécuté par nous. La seule vérification locale est une petite expression Node qui confirme le traitement des arguments et la concaténation des URL du stub Spider. Les commentaires sur les défauts concernent le code épinglé ci-dessous, sans présumer quel code a servi aux anciennes mesures publiées.

## Conclusion

**Aucun des benchmarks examinés ne permet de classer Stagehand, Crawlee et Spider-rs pour toutes les tâches d'Automan.** Le benchmark Spider compare une bibliothèque Rust à Colly, au paquet Node `crawler` et à wget ; Crawlee et Stagehand n'y figurent pas. Un benchmark d'agents compare Stagehand 3.2.1 à Browser Use et Sentinel sur quelques tâches. Un benchmark de crawlers mesure surtout des sorties Markdown et leur utilité pour un pipeline de recherche documentaire. Ces expériences portent sur des contrats différents.

Les résultats publiés restent utiles pour formuler des hypothèses. Ils ne démontrent ni « Rust est toujours plus rapide », ni « un script bat toujours un agent », ni une fiabilité actuelle de Stagehand 4.1.0. Un classement honnête exige des pages, sorties et états finaux équivalents, ainsi que les coûts des tentatives ratées.

## 1. Spider : lire le script avant de citer les multiplicateurs

Sources épinglées au commit **`fb0f270d77e19442de59350d9dbbacf52f1804b4`**, correspondant à `main` lors du relevé : [protocole publié](https://github.com/spider-rs/spider/blob/fb0f270d77e19442de59350d9dbbacf52f1804b4/benches/BENCHMARKS.md), [harness crawl.rs](https://github.com/spider-rs/spider/blob/fb0f270d77e19442de59350d9dbbacf52f1804b4/benches/crawl.rs), [stub Go](https://github.com/spider-rs/spider/blob/fb0f270d77e19442de59350d9dbbacf52f1804b4/benches/go_colly.rs), [stub Node](https://github.com/spider-rs/spider/blob/fb0f270d77e19442de59350d9dbbacf52f1804b4/benches/node_crawler.rs).

Le tableau publié annonce notamment un crawl de 185 pages en 73 ms pour Rust, 32 s pour Go et 15 s pour Node sur un Mac M1 Max. Il décrit des machines et un site distant, mais le harness actuel ne garantit pas que chaque concurrent termine avec les mêmes URL et le même contenu. Ce sont des chiffres du fournisseur, pas des mesures que nous avons reproduites.

### Asymétries visibles

- **Bibliothèques comparées :** `spider`, Colly, le paquet Node `crawler`, wget. La ligne Node ne désigne pas Crawlee, Playwright ou Stagehand.
- **Chronométrage :** Spider est appelé dans le processus avec un runtime Tokio créé avant la mesure ; Go, Node et wget sont lancés dans un processus à chaque itération. En mode décentralisé, le worker est démarré avant le chronométrage. Cette expérience peut mesurer le rendement d'une exécution chaude, mais ne constitue pas une comparaison équivalente des démarrages ou du coût total des workers.
- **Travail accompli :** le harness n'assertionne ni le nombre de pages valides, ni l'égalité des URL, ni la qualité du contenu. Les sorties des commandes sont recueillies sans assertion sur leur code de succès. Un run qui fait moins de travail peut donc sembler rapide.
- **Concurrence Go :** le stub appelle `colly.NewCollector()` sans activer `colly.Async()`. La [source de Colly](https://github.com/gocolly/colly/blob/master/colly.go) expose explicitement cette option ; l'exécution générée n'active pas la concurrence asynchrone dans le programme. Des variables d'environnement de Colly pourraient modifier la configuration, mais elles ne sont pas fixées par ce harness. Cette ligne ne représente donc pas une optimisation comparable de Colly.
- **Réseau :** des sites distants, le cache, les limitations serveur et les blocages affectent les résultats. Les politiques de liens et de ressources des stubs ne constituent pas une spécification commune vérifiée.

### Deux problèmes dans le stub Node actuel

Le programme est lancé avec `node ./node-crawler.js <url>`, mais prend `argv.slice(1)[0]` comme URL de base. Dans cette invocation habituelle, cela donne le chemin du script. Il construit aussi les liens par concaténation `base + href`, même lorsque `href` est déjà une URL absolue.

Vérification locale limitée, sans lancer de crawl :

```js
const argv = ['/usr/bin/node', './node-crawler.js', 'https://fixture.invalid'];
argv.slice(1)[0]; // './node-crawler.js'
'https://fixture.invalid' + 'https://fixture.invalid/page';
// 'https://fixture.invalidhttps://fixture.invalid/page'
```

**Conclusion permise :** corriger et instrumenter ces stubs avant toute reproduction comparative. Ces observations ne prouvent pas que les anciennes mesures utilisaient exactement ce code, et ne permettent pas de conclure à une intention de tromper. Elles empêchent de reprendre aujourd'hui le tableau comme preuve d'une supériorité de Spider sur Crawlee ou Stagehand.

## 2. Agents : un résultat Stagehand 3.2.1 n'est pas un résultat Stagehand 4

Le dépôt [browser-agent-benchmark](https://github.com/ArasHuseyin/browser-agent-benchmark) compare le produit de son auteur, Sentinel, à Browser Use et Stagehand. L'auteur déclare ce conflit d'intérêt. Audit épinglé au commit **`8672aaa405ad371d947a2e84f850f20464cb6350`** : [méthodologie](https://github.com/ArasHuseyin/browser-agent-benchmark/blob/8672aaa405ad371d947a2e84f850f20464cb6350/METHODOLOGY.md), [résultats agrégés](https://github.com/ArasHuseyin/browser-agent-benchmark/blob/8672aaa405ad371d947a2e84f850f20464cb6350/results/aggregated/summary.json), [limites](https://github.com/ArasHuseyin/browser-agent-benchmark/blob/8672aaa405ad371d947a2e84f850f20464cb6350/docs/limitations.md).

Les résultats JSON sont datés du **23 avril 2026**, utilisent `gemini-3-flash-preview` et donnent, après agrégation directe des champs :

| Configuration déclarée | Tâches | Tentatives | Succès | Échantillons de durée et de tokens présents |
| --- | ---: | ---: | ---: | ---: |
| Browser Use 0.12.6 | 9 | 45 | 45 | 45 |
| Sentinel 4.1.2 | 9 | 45 | 45 | 45 |
| Stagehand 3.2.1 | 9 | 45 | 39 | 39 |
| Stagehand hybrid 3.2.1 | 4 | 20 | 18 | 18 |

Les totaux sont calculés à partir du JSON, pas estimés depuis un graphique. Ils n'ont pas été reproduits par l'exécution des tâches. L'ancienne prose de `METHODOLOGY.md` présente encore une série différente, notamment 32/45 pour Stagehand. Il faut donc identifier précisément le fichier et la série cités.

Les durées et tokens agrégés ne couvrent ici que les succès. La méthodologie reconnaît l'exclusion des runs échoués des statistiques de coût, notamment des tentatives très consommatrices. Le coût moyen affiché ne suffit donc pas à calculer **le coût total par succès**, qui doit inclure toutes les tentatives. Les p95 calculés avec cinq runs par tâche sont aussi très peu informatifs.

La documentation dit que les données brutes sont commitées. Pourtant, à ce SHA, l'arbre `results/raw/` ne contient que `.gitkeep` ; seuls les résumés sont visibles dans `results/aggregated/`. Cela empêche de recalculer depuis ce dépôt les distributions de toutes les tentatives. Cela ne prouve pas que des logs n'existent nulle part ailleurs.

Enfin, les modes diffèrent : Stagehand est utilisé avec des primitives et une variante hybrid, Browser Use avec une boucle autonome. La comparaison a un intérêt de cas d'usage, mais ne sépare pas proprement toutes les variables. Les tâches et versions d'avril ne déterminent pas le résultat d'octobre pour Stagehand 4, un autre modèle, des workflows récurrents ou un crawl massif.

## 3. Crawlers Markdown/RAG : autre objectif, autre configuration

Le dépôt [llm-crawler-benchmarks](https://github.com/AIMLPM/llm-crawler-benchmarks) est maintenu par les développeurs de markcrawl, l'un des outils comparés ; l'intérêt produit est déclaré. Audit au commit **`02d513cf491732450c30b26b71a7e06a6ca4d49e`** : [méthodologie effective](https://github.com/AIMLPM/llm-crawler-benchmarks/blob/02d513cf491732450c30b26b71a7e06a6ca4d49e/reports/METHODOLOGY.md), [runner Crawlee](https://github.com/AIMLPM/llm-crawler-benchmarks/blob/02d513cf491732450c30b26b71a7e06a6ca4d49e/runners/crawlee_runner.py), [worker](https://github.com/AIMLPM/llm-crawler-benchmarks/blob/02d513cf491732450c30b26b71a7e06a6ca4d49e/crawlee_worker.py).

Le pipeline examine le crawl, la conversion Markdown, le contenu, la récupération documentaire et les réponses. Cela mesure une finalité utile, mais pas une interaction authentifiée ou la sécurité d'un clic. Les différentes versions de la documentation décrivent des corpus et étapes successifs ; éviter de mélanger leurs tableaux comme une seule expérience.

La configuration Crawlee étudiée utilise **Python et Playwright**, avec un sous-processus par appel pour contourner une difficulté de boucle événementielle décrite par le runner. Ce démarrage est compris dans la durée. Elle ne représente ni Crawlee TypeScript, ni `HttpCrawler`, ni `CheerioCrawler`, ni toutes les configurations de pool de navigateurs. Des baselines HTTP et des configurations avec navigateur rendent également des services différents.

Le texte méthodologique distingue les répétitions du crawl de vitesse et les mesures de qualité documentaire, dont certaines restent sur une seule série. Une vérification de pertinence par modèle n'est pas un oracle indépendant de vérité. **Spider et Stagehand ne sont pas comparés dans cette expérience** ; aucun classement du trio n'en découle.

## 4. Évaluations Stagehand et anecdotes Reddit

La page [Stagehand evals](https://www.stagehand.dev/evals) décrit une évaluation de modèles, affichée comme actualisée le **10 juillet 2026**. Elle ne constitue pas une comparaison Stagehand/Crawlee/Spider. Les modèles, versions et objectifs doivent accompagner toute reprise de métriques.

Les [témoignages Reddit audités](./2026-10-07-reddit-evidence-followup.md) signalent des problèmes de sessions, de coût et de succès apparent. Ils ne fournissent pas un protocole commun du trio. Un retour d'avril 2025 annonçant plus de 500 000 tokens pour un agent ne décrit pas automatiquement Stagehand 4 actuel. Un gain d'une stack Browserbase + Stagehand ne peut pas être attribué au SDK seul.

## 5. Ce qu'Automan doit publier pour prouver son avantage

Le protocole général reste détaillé dans [benchmark et runtime](./2026-10-07-benchmark-and-runtime.md). Cette recherche précise les comparateurs à isoler :

| Contrat | Comparateurs pertinents | Résultat vérifié |
| --- | --- | --- |
| Crawl HTTP | Spider HTTP, Crawlee HTTP/Cheerio, Scrapy, Colly async, Automan HTTP | Même ensemble d'URL, contenu attendu, couverture, erreurs |
| Crawl avec rendu | Spider Chrome, Crawlee Playwright, Crawl4AI, Automan CDP | Même contenu après JavaScript, ressources et isolation déclarées |
| Actions connues | Playwright, Stagehand page/locator, agent-browser CLI, plans Automan | État final observé, effets uniques, reprises sûres |
| Découverte sémantique | Stagehand act/observe/extract, Browser Use, Skyvern, spider_agent, Automan | Même modèle si compatible, même budget et oracle de tâche |
| Exécution répétée avec dérive | Cache/replay Stagehand, heuristiques Scrapling, workflows Automan | Succès après mutations contrôlées, faux succès, données périmées |
| Incident et reprise | Framework + stockage réellement configurés | Pas de travail perdu non déclaré, pas de doublons d'effets, diagnostic exploitable |

Publier séparément démarrage froid et chaud, décision initiale et replay, temps navigateur et LLM, volume réellement validé, mémoire totale processus enfants comprise, p50/p95 avec assez d'échantillons, coût total de toutes les tentatives par succès et traces anonymisées. Fixer les versions, machines, sites autorisés, caches, proxies, concurrence, politiques robots et délais. Un classement sans ces contrats confond souvent moins de travail avec plus de performance.

**Décision d'architecture :** conserver Rust comme hypothèse de runtime commun, tout en exigeant des gains mesurés par sous-système. Les benchmarks existants justifient une expérience comparative ; ils ne constituent pas sa conclusion.
