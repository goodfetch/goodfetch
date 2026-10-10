# ADR-003 : modèle de session authentifiée longue

**Statut : proposé, sur verdict mécanique du lab ; à adopter explicitement par le porteur du projet.**
**Date : 9 octobre 2026.**
**Décision finale : utilisateur / porteur du projet.**
**Décision du registre : `D-session-model` ; verdict `lab/verdicts/D-session-model.json` ; rapport `lab/reports/portal-2026-10-09.md`.**

## Contexte

Le segment des portails B2B repose sur des sessions authentifiées longues : un login à jeton CSRF, des sessions qui expirent, des refresh tokens, des runs interrompus puis repris. L'architecture fixe déjà plusieurs règles. La session porte une identité, des cookies, un backend propriétaire et un lease ([architecture §2.1](../proposals/architecture.md:59)), et le pool de sessions gère expiration, refresh et quarantaine ([§4.1](../proposals/architecture.md:143)). Les sessions expirées ne sont pas réutilisées aveuglément ([§2.2](../proposals/architecture.md:83)). Une politique distingue l'authentification expirée des autres causes, sous un budget de tentatives unique ([§3.4](../proposals/architecture.md:125)). L'ADR-002 impose un seul profil d'identité par session, entre HTTP et navigateur.

Restait à décider comment le moteur détecte la perte de session et ce qu'il garde d'un run à l'autre. La lane `portal` du lab a comparé trois politiques sur cinq situations, en HTTP et dans le navigateur, contre un site dont l'oracle compte enregistrements et logins (`portal-sessions-2026-10-09T15-26-01-462Z-3f0b51`, 5 blocs).

## Décision

1. **Chaque page d'une session est jugée sur le contrat de la recette, jamais sur le seul statut HTTP.**
   - La page attendue (le listing authentifié) est un succès.
   - Un formulaire de login reconnu est une session perdue.
   - Toute autre page arrête la tâche avec `E_CONTRACT`.
2. **L'état de session fait partie du journal du run.** Cookies, curseur et enregistrements déjà validés y sont écrits, et un nouveau processus ou un nouveau contexte navigateur les reprend. L'état repris est revalidé sur la première page. S'il est refusé, par révocation ou par expiration, le moteur se reconnecte une fois.
3. **La recette déclare la récupération de session** : endpoint et cookie de refresh, puis login avec une référence d'identifiants. La récupération consomme le budget de tentatives commun ; deux récupérations au plus par page.
4. L'import et l'export d'une session restent des opérations distinctes de l'export du plan. Les cookies du journal ont le périmètre tenant et identité de la session ([§8.3](../proposals/architecture.md:318)).

## Preuves

| Politique | Faux succès | Logins par run repris | Verdict (logins, contre `relogin-each-run`) |
| --- | --- | --- | --- |
| `contract` (journal + contrat de page) | 0 | 1 | **meilleur**, ratio 0,50, 5/5 blocs, en HTTP et dans le navigateur |
| `relogin-each-run` (contrat, sans état repris) | 0 | 2 | référence |
| `status-only` (état repris, statut HTTP) | 20/20 sur l'expiration servie en `200` | 1 | **bloqué par le contrat** (`oracle-exact`) |

- Quand le portail répond `200` avec son formulaire de login à l'URL d'un rapport, `status-only` lit une page vide et déclare le crawl fini : 20 ou 40 enregistrements manquent selon le tier.
- Reprendre l'état journalisé évite un login à chaque redémarrage. Une session révoquée pendant l'arrêt est détectée à la première page et coûte un seul login, sans perte ni doublon.
- Les logins sont comptés par le site et ne dépendent pas de l'hôte : la décision peut se prendre en `local-dev`. Les durées, indicatives, ne servent pas au verdict.

## Conséquences

- Le schéma de recette porte un **contrat de page de session**, un marqueur d'authentification et la forme attendue du listing, ainsi qu'une **politique de récupération**.
- Le journal d'un run stocke l'état de session chiffré au repos, avec la même rétention et les mêmes droits que les traces ([§8.3](../proposals/architecture.md:318)).
- La documentation montre ce cas en exemple : un portail qui renvoie `200` à l'expiration est le piège le plus courant.

## Limites

- Fixtures synthétiques. MFA, SSO et jetons liés à l'appareil ne sont pas couverts ; ils relèvent d'une reprise avec intervention humaine (`needs_review`).
- Un seul compte par session. La rotation de comptes n'est pas traitée ici.
