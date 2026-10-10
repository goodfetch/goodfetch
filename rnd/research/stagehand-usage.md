# Comment Stagehand est utilisé

Recherche consultée le 7 octobre 2026. Cette section décrit Stagehand **v4** à partir de documentation officielle. Les exemples de documentation démontrent des usages possibles ; ils ne mesurent pas la fréquence de ces usages chez les clients.

## Ce que le développeur installe et exécute

Stagehand fournit des SDK TypeScript/JavaScript, Python et Go. Le quickstart installe le package du langage choisi, crée un fichier `index.ts`, `main.py` ou `main.go`, lance Chrome localement, initialise Stagehand puis appelle `act`, `extract` et `observe`. Le script reste un programme ordinaire exécuté avec Node/tsx, Python ou Go. [Quickstart v4](https://docs.stagehand.dev/v4/first-steps/quickstart)

Le développeur peut mélanger des commandes précises, comme `page.goto()` et `page.locator().fill()`, avec des instructions en langage naturel et de l'extraction validée par un schéma. Il choisit quels passages utilisent un modèle. [Introduction v4](https://docs.stagehand.dev/v4/first-steps/introduction)

Le navigateur peut tourner sur la machine du développeur, dans une session Browserbase, ou dans un Chromium déjà lancé auquel le SDK se connecte par CDP. Utiliser le SDK n'impose donc pas d'utiliser le service cloud Browserbase. Le navigateur local requiert une installation Chrome et un modèle fourni par le développeur pour les primitives IA ; Browserbase ajoute notamment le Model Gateway et le cache côté serveur. [Quickstart v4](https://docs.stagehand.dev/v4/first-steps/quickstart), [Configuration du navigateur](https://docs.stagehand.dev/v4/configuration/browser)

## Les principaux modes d'utilisation

| Mode | Où Stagehand intervient | Qui décide la suite ? |
| --- | --- | --- |
| Script autonome | Un programme lance un navigateur et réalise une séquence d'opérations. | Le code du script. [Quickstart](https://docs.stagehand.dev/v4/first-steps/quickstart) |
| Application ou workflow | L'application utilise le SDK pour automatiser le workflow web d'un service, puis récupère les résultats. | Le code de l'application. [Intégrations : Applications](https://docs.stagehand.dev/v4/integrations/overview) |
| Backend, webhook ou job | Une fonction d'automatisation est invoquée par API depuis un backend, un worker, un webhook ou une tâche planifiée. | Le handler écrit par le développeur. Browserbase Functions est une option de déploiement. [Functions](https://docs.browserbase.com/platform/functions/overview) |
| Agent LLM développé dans une application | Le framework de l'agent reçoit des outils navigateur et conserve une session entre les appels. | Le modèle et la boucle d'outils gérés par le framework de l'agent. [Agent Frameworks](https://docs.stagehand.dev/v4/integrations/agent-frameworks/overview) |
| Agent de développement existant | Claude Code, Codex, fx ou Pi reçoit des outils pour inspecter une application en cours de développement ou naviguer sur le web. | L'agent de développement. [CLI Agents](https://docs.stagehand.dev/v4/integrations/cli-agents/overview) |

**Interprétation pour le produit :** une application peut proposer un bouton ou un chat qui déclenche Stagehand dans son backend/worker. L'utilisateur final voit la fonctionnalité et le résultat ; c'est le développeur de l'application qui intègre le SDK. Ce flux bouton → backend → Stagehand → navigateur → résultat est un exemple illustratif d'intégration, cohérent avec les modes Applications et Functions documentés. [Intégrations](https://docs.stagehand.dev/v4/integrations/overview), [Functions](https://docs.browserbase.com/platform/functions/overview)

## Cinq exemples concrets officiels

1. **Petit script local :** ouvrir `stagehand.dev`, extraire la proposition de valeur dans un objet typé, cliquer sur Evals puis découvrir les actions disponibles. Le programme écrit explicitement ces trois étapes. C'est l'exemple de démarrage, pas un témoignage client. [Quickstart v4](https://docs.stagehand.dev/v4/first-steps/quickstart)
2. **Automatisation appelée par API :** la Function `page-title` reçoit une URL, connecte Stagehand à la session Browserbase créée pour cette invocation, navigue, extrait un titre puis retourne du JSON. Le même mécanisme permet un déclenchement depuis un backend, un webhook, une queue ou un job. C'est un exemple de déploiement officiel. [Functions](https://docs.browserbase.com/platform/functions/overview), [Invocation](https://docs.browserbase.com/platform/functions/invoke)
3. **Agent dans une application :** l'exemple Vercel AI SDK ouvre un client MCP une fois, charge `run`, `snapshot` et `screenshot`, puis laisse sa boucle d'outils répondre à une demande telle qu'ouvrir un site et rapporter son titre. Le navigateur conserve l'authentification et l'état entre les appels. L'intégration est expérimentale et se construit depuis le dépôt ; elle n'est pas publiée comme adapter indépendant. [Vercel AI SDK](https://docs.stagehand.dev/v4/integrations/vercel-ai-sdk)
4. **Agent de développement :** configurer le serveur MCP Stagehand dans Codex ou Claude Code, puis demander à l'agent d'ouvrir une page et d'en rapporter le titre. La doc mentionne également l'inspection d'une application en cours d'exécution. L'agent utilise ses propres modèle et authentification ; Stagehand apporte les outils navigateur. L'adapter partagé est expérimental et se construit depuis le dépôt. [CLI Agents](https://docs.stagehand.dev/v4/integrations/cli-agents/overview)
5. **Workflow métier intégré :** l'exemple Stagehand 4.1.0 + Stripe Link prépare une livraison de fleurs sur 1-800-Flowers, examine le panier, demande l'approbation du paiement via Link et peut remplir le checkout. Stagehand manipule le site ; Link gère la demande d'approbation et le moyen de paiement ; l'application orchestre le workflow. C'est un exemple complet officiel, pas une preuve d'adoption mesurée. [Stagehand + Link SDK](https://docs.browserbase.com/integrations/stripe/stagehand), [Vue d'ensemble Stripe](https://docs.browserbase.com/integrations/stripe/introduction)

## Le point essentiel de v4 : le moteur n'est pas la boucle d'agent

`stagehand.agent()` et `agent.execute()` ont disparu en v4. Il n'existe pas de remplacement un pour un. Deux usages sont recommandés : un assistant écrit un script ensuite exécuté, ou un modèle reste dans une boucle d'outils que l'application possède. `act`, `extract` et `observe` demeurent disponibles pour des étapes qui bénéficient d'une instruction en langage naturel. [Migration v3 → v4](https://docs.stagehand.dev/v4/migrations/v3)

Dans une intégration agent, le framework externe gère le modèle, les instructions et la boucle. Stagehand conserve le navigateur et exécute les opérations. Les intégrations du dépôt partagent trois outils : `run` pour exécuter du JavaScript ou des actions d'éléments, `snapshot` pour lire une représentation de l'accessibilité, et `screenshot` pour une capture. Elles existent en MCP/stdio ou comme outils natifs selon le framework : Anthropic SDK, Eve, Deep Agents, CrewAI, Mastra et Vercel AI SDK. [Agent Frameworks](https://docs.stagehand.dev/v4/integrations/agent-frameworks/overview), [README des intégrations](https://github.com/browserbase/stagehand/blob/main/packages/integrations/README.md)

Les opérations déterministes telles que navigation, snapshot et screenshot ne nécessitent pas un modèle Stagehand supplémentaire. Si l'agent appelle aussi des méthodes IA Stagehand, leur modèle peut avoir une configuration et des credentials distincts de ceux du modèle qui dirige l'agent. [Agent Frameworks : Configure models](https://docs.stagehand.dev/v4/integrations/agent-frameworks/overview)

## Où tourne chaque partie

Le code du SDK et du script/agent tourne dans le processus hôte du développeur ou de l'application. Le cœur de Stagehand v4 — gestion des cibles, frames, contextes d'exécution et commandes CDP — tourne dans une extension du navigateur. Les SDK sont des clients qui parlent à ce cœur par RPC sur la connexion CDP. Quand un développeur fournit une fonction de modèle personnalisée, cette fonction reste dans son processus et l'extension lui demande les générations via le même canal. [Architecture officielle v4](https://www.browserbase.com/blog/stagehand-v4)

Dans les intégrations MCP, le serveur garde une session navigateur persistante. Le JavaScript envoyé à `run` s'exécute dans le service worker de l'extension Stagehand ; il peut contrôler le navigateur. L'agent qui choisit cet appel tourne dans son propre environnement. [Agent Frameworks](https://docs.stagehand.dev/v4/integrations/agent-frameworks/overview)

Browserbase Functions est un produit d'hébergement complémentaire : le code TypeScript du handler tourne à proximité du navigateur cloud, et l'application invoque la fonction par API. Il faut distinguer le SDK Stagehand, le navigateur hébergé Browserbase et l'hébergement de code Functions. [Functions](https://docs.browserbase.com/platform/functions/overview)

## Conséquence pour un framework similaire

**Inférence de conception :** les interfaces de consommation à prévoir sont d'abord un SDK utilisable dans un script/backend, puis des outils pour agents/MCP. Une app graphique propre au framework peut être ajoutée, mais les usages documentés ne l'exigent pas. Le choix Rust concerne l'implémentation du cœur et sa distribution ; le développeur consommateur doit surtout disposer d'une installation simple, d'une API naturelle dans son langage et d'un déploiement adapté. Cette conclusion est déduite des modes d'utilisation ci-dessus ; ce n'est pas une recommandation officielle Stagehand. [Intégrations](https://docs.stagehand.dev/v4/integrations/overview)

## Prudence sur les versions

Certaines pages marketing ou des résultats indexés parlent encore de quatre primitives et de `agent()`. Pour répondre sur v4, la documentation versionnée et la migration v3 → v4 font foi : trois primitives IA, des API navigateur et une orchestration externe. Ne pas prendre les anciens exemples `new Stagehand()`, `init()` ou `agent.execute()` pour le parcours v4. [Migration v3 → v4](https://docs.stagehand.dev/v4/migrations/v3)

## Retours d'utilisateurs sur Reddit

Les discussions suivantes sont des déclarations publiques de leurs auteurs. L'identité des entreprises, les chiffres de coût et les taux de réussite n'ont pas été vérifiés indépendamment. Elles illustrent des usages et des questions d'intégration ; elles ne mesurent ni la popularité de chaque usage ni les performances de v4. Les dates ci-dessous viennent des résultats de recherche du fil, et les contenus ont été ouverts et relus.

| Date du post | Usage décrit | Portée du témoignage |
| --- | --- | --- |
| 22 mai 2026 | L'auteur dit maintenir un pipeline de veille des prix pour un SaaS. Il décrit Stagehand comme le SDK de pilotage et Browserbase comme le service navigateur, avec des résultats alimentant des dashboards. | Exemple déclaré d'intégration dans un produit et de collecte récurrente. Post antérieur à v4 ; ses snippets et ses chiffres ne servent pas de description ou de benchmark de la version actuelle. [Fil WebScrapingInsider](https://www.reddit.com/r/WebScrapingInsider/comments/1tkqi5e/we_rebuilt_our_scraping_stack_4_times_in_18/) |
| 7 avril 2025 | Un auteur explique chercher une brique d'automatisation navigateur pour un agent qu'il construit et apprécie la possibilité de contrôler des étapes individuelles. | Témoignage ancien mêlé à la promotion de sa newsletter. Utile pour identifier le rôle d'outil dans un agent, pas pour évaluer v4 ni reprendre les chiffres de tokens avancés. [Fil r/node](https://www.reddit.com/r/node/comments/1jtdwpz/stagehand_node_package_to_control_browser_with/) |
| 13 août 2026 | Un utilisateur cherche à donner un navigateur à son agent local Hermes et hésite entre Stagehand SDK, Browse CLI et les recettes Browse.sh. | Exemple de besoin d'intégration à un agent existant et de confusion entre les couches du produit. Les choix d'installation et les fonctionnalités sont vérifiés dans la doc officielle, pas déduits des commentaires. [Fil LLMDevs](https://www.reddit.com/r/LLMDevs/comments/1vn530v/confused_about_browserbase_product_lines/) |
| 15 avril 2026 | Un utilisateur compare des sessions d'agents navigateur Stagehand et Browser Use. D'autres commentent les difficultés d'authentification et de localisation des éléments. | Retours individuels, cas d'usage peu détaillé et antérieur à v4. Aucun classement de fiabilité ne peut en être déduit. [Fil AI_Agents](https://www.reddit.com/r/AI_Agents/comments/1slvuta/stagehand_vs_browser_use_which_one_actually_works/) |

## Réponse pratique à « une app ou un agent ? »

Les deux : Stagehand est une bibliothèque que le développeur consomme, et le programme qui la consomme peut être un script, un backend/worker ou un agent. Dans un script, le code choisit la séquence ; dans un agent, la boucle de l'agent choisit les outils et Stagehand exécute les opérations navigateur. Cette distinction est directement documentée dans le [quickstart v4](https://docs.stagehand.dev/v4/first-steps/quickstart) et les [intégrations d'agents](https://docs.stagehand.dev/v4/integrations/agent-frameworks).

Flux illustratif d'une application : l'utilisateur demande un export dans l'interface ; le backend déclenche un worker ; le worker utilise Stagehand pour ouvrir le site et récupérer les données ; l'application conserve et affiche le résultat. Ce flux est une illustration de conception, pas un client identifié ni une fonctionnalité d'export universel fournie par Stagehand. L'hébergement du code peut être celui du développeur ou une offre complémentaire comme [Browserbase Functions](https://docs.browserbase.com/platform/functions/overview).

Pour le framework envisagé, l'interface consommateur à définir en premier est donc une API de bibliothèque avec gestion des sessions, opérations navigateur et résultats structurés. Un adapter pour agents/MCP constitue une seconde surface sur le même moteur. Cette proposition ne suppose pas encore de builder visuel, de marketplace ou de SaaS.
