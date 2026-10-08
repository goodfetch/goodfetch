# Vérification de la comparaison collée : Crawlee, Stagehand et agent-browser

Vérifié le **7 octobre 2026**. Cette note corrige les affirmations du texte fourni par l’utilisateur à partir des documentations officielles, des manifests publiés et du code versionné. Les exemples ci-dessous illustrent des contrats documentés ; aucun crawl, appel LLM ni benchmark comparatif n’a été exécuté.

## La différence de versions est déterminante

Le package npm actuel **`@crawlee/stagehand` 3.18.2** déclare une peer dependency **`@browserbasehq/stagehand: ^3.0.0`** et utilise **3.0.7** pour son développement. Il ne déclare donc pas la compatibilité Stagehand v4. Son intégration enrichit des pages Playwright et se connecte par CDP au navigateur lancé par Stagehand. [Manifest versionné](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/package.json), [registre npm actuel](https://registry.npmjs.org/@crawlee/stagehand/latest), [plugin versionné](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/src/internals/stagehand-plugin.ts).

Les documentations **Stagehand v4** décrivent une autre API : pilotage Chromium par CDP sans dépendance Playwright, pas d’interopérabilité directe avec une `Page` Playwright, et suppression de l’orchestrateur `agent()`. Celui qui utilise le SDK contrôle la boucle, éventuellement avec un agent externe. Confondre les exemples Crawlee v3 et la documentation Stagehand v4 conduit à du code incorrect. [Migration Playwright](https://docs.stagehand.dev/v4/migrations/playwright), [migration Stagehand v3 vers v4](https://docs.stagehand.dev/v4/migrations/v3).

## L’intégration existe ; les projets n’ont pas fusionné

L’ajout de **StagehandCrawler en JavaScript** a été annoncé avec **Crawlee 3.16.0 le 6 février 2026**. C’est une composition officielle de bibliothèques. Elle ajoute les fonctions IA aux capacités du crawler ; cela ne remplace pas les autres crawlers et ne constitue pas une fusion des projets. [Annonce Crawlee](https://crawlee.dev/blog/crawlee-v3-16), [changelog du package](https://crawlee.dev/js/api/stagehand-crawler/changelog.md).

L’équivalent **Python** existe aussi, ajouté dans **Crawlee Python 1.7.0 le 12 mai 2026**. Son contrat est distinct : installation `crawlee[stagehand]`, configuration `stagehand_options=StagehandOptions(...)`, méthodes `act`, `extract`, `observe`, `execute`. Dans **Crawlee Python 1.10.4**, l’extra dépend du client Python `stagehand>=3.19.5,<4.0.0`. Ces numéros de SDK Python ne doivent pas être assimilés aux versions npm. [Changelog Python versionné](https://github.com/apify/crawlee-python/blob/v1.10.4/CHANGELOG.md), [manifest Python](https://github.com/apify/crawlee-python/blob/v1.10.4/pyproject.toml), [guide Python](https://crawlee.dev/python/docs/guides/stagehand-crawler).

## Le bloc de configuration fourni mélange des options

Pour `@crawlee/stagehand` 3.18.2 :

| Proposition collée | Verdict au périmètre du package publié |
| --- | --- |
| `stagehandOptions` | Oui : option réelle de `StagehandCrawlerOptions`. |
| `modelName: "gpt-4o-mini"` | Pas une propriété de son interface `StagehandOptions`. La propriété attendue est `model`, avec notamment une chaîne préfixée par le fournisseur. |
| `domToParse: "accessibility"` | Pas une option de l’interface publiée, et aucun middleware de ce nom n’est fourni ou montré dans l’extrait. Rien ne justifie d’affirmer que cette ligne force un arbre d’accessibilité. |
| « Désactiver la vision » avec cet extrait | Le code ne contient aucun interrupteur documenté de vision. Le choix d’un modèle et celui de la représentation de page sont deux décisions différentes. |

Source qui gouverne le contrat : [interfaces `StagehandOptions` et `StagehandCrawlerOptions`](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/src/internals/stagehand-crawler.ts). Le launcher fixe notamment un `model` par défaut ; le plugin construit son `V3Options` en utilisant `stagehandOptions.model`. En JavaScript non typé, transmettre une propriété inconnue n’établit pas qu’elle est interprétée. [Launcher](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/src/internals/stagehand-launcher.ts), [construction du modèle dans le plugin](https://github.com/apify/crawlee/blob/v3.18.2/packages/stagehand-crawler/src/internals/stagehand-plugin.ts).

Forme minimale cohérente avec l’intégration JS publiée :

```ts
import { StagehandCrawler } from '@crawlee/stagehand';

const crawler = new StagehandCrawler({
    stagehandOptions: {
        env: 'LOCAL',
        model: 'openai/gpt-4.1-mini',
        apiKey: process.env.OPENAI_API_KEY,
    },
    async requestHandler({ page }) {
        await page.act('Click the Documentation link');
    },
});
```

Le modèle est celui utilisé comme exemple et défaut dans les sources inspectées, sans prétention de meilleur choix économique actuel. L’extrait est une illustration d’API, pas un exemple exécuté ou une configuration validée de désactivation de vision. [Guide JS](https://crawlee.dev/js/docs/guides/stagehand-crawler-guide).

En Python, les paramètres réels sont notamment `model` et `model_api_key`, avec des champs distincts pour Browserbase. Le SDK REST et le wrapper Playwright ne se configurent pas en recopiant l’objet JavaScript. [Dataclass `StagehandOptions`](https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/browsers/_stagehand_types.py).

## L’arbre d’accessibilité n’est pas exclusif à agent-browser

Stagehand v4 expose `page.snapshot()` avec un arbre d’accessibilité formaté (`formattedTree`) et une correspondance vers les XPath (`xpathMap`). Sa documentation présente snapshots et screenshots comme des outils distincts à exposer à un agent. Elle recommande aussi les opérations déterministes pour les sélecteurs stables et les primitives IA seulement lorsque la tâche demande du jugement. Le portrait « Stagehand envoie toujours un DOM entier ou une image » est donc inexact. [Migration v3 vers v4](https://docs.stagehand.dev/v4/migrations/v3).

**Playwright MCP utilise également des snapshots structurés d’accessibilité**, sans exiger un modèle de vision. Son README explique désormais l’intérêt de CLI + skills pour limiter les schémas et arbres verbeux dans le contexte des agents de codage. La comparaison « agent-browser arbre léger contre Playwright MCP screenshot lourd » n’est donc pas une description correcte des modes standard. [README officiel Playwright MCP](https://github.com/microsoft/playwright-mcp).

Le README actuel d’agent-browser décrit un **CLI et un daemon Rust**, avec CDP direct. Il documente les snapshots filtrés et des screenshots optionnels ; le texte fourni conserve l’ancienne architecture de daemon Node.js. [README agent-browser](https://github.com/vercel-labs/agent-browser#architecture).

## Les avantages quantitatifs et les garanties sont exagérés

Les ratios « 15× à 32× moins de tokens » n’ont pas été rattachés, dans cette vérification, à un benchmark primaire reproductible comparant les mêmes tâches, modèles, résultats et modes actuels Stagehand/Playwright MCP/agent-browser. Les liens génériques Reddit, YouTube ou LinkedIn du texte ne permettent pas d’auditer ces affirmations. Ce constat ne prouve pas qu’aucun benchmark externe existe ; il interdit de présenter ce ratio comme un résultat général établi ici.

Les benchmarks officiels présents dans le dépôt agent-browser comparent **son ancien daemon Node.js et son daemon Rust**, pas les tokens de Stagehand contre Playwright MCP. Le README du benchmark précise que les délais par commande dépendent surtout de Chrome et des échanges CDP ; les bénéfices principaux du daemon Rust portent sur le démarrage, la mémoire du daemon et la distribution. [Benchmarks agent-browser](https://github.com/vercel-labs/agent-browser/tree/main/benchmarks).

Le guide Crawlee mentionne explicitement les limites de son adaptateur Stagehand : Chromium uniquement et contrôle de fingerprinting réduit. On ne peut pas lui attribuer automatiquement toutes les possibilités de lancement/fingerprinting de `PlaywrightCrawler`, encore moins une garantie générale de contournement des protections ou de réparation correcte. [Limites JS](https://crawlee.dev/js/docs/guides/stagehand-crawler-guide), [limites Python](https://crawlee.dev/python/docs/guides/stagehand-crawler).

## Conséquence proposée pour Automan

Retenir une représentation de page compacte par défaut, un périmètre ciblé et la vision optionnelle est cohérent. Cela peut être indépendant du driver CDP, de la queue et du choix CLI/MCP/SDK. La différenciation doit être mesurée sur des parcours réels : observations évitées, actions déterministes réutilisées, réparation validée et résultats corrects, plutôt que sur la présence d’un arbre d’accessibilité déjà partagé par plusieurs outils.

Pour les contrats détaillés et limites déjà inspectées, voir [l’étude Crawlee/Spider](./2026-10-07-crawlee-spider-contracts-followup.md) et [l’étude des alternatives Stagehand](./2026-10-07-stagehand-alternatives-followup.md). Aucun nouveau résultat expérimental n’est ajouté par cette note.
