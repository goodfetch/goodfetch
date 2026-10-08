# Couche IA avec de vrais modèles (D-ai-layer), 9 octobre 2026

Run retenu : `ai-real-models-2026-10-08T22-15-04-120Z-d70ce2`, en `local-dev` sur sources commitées, avec la même lane, les mêmes pages et le même oracle que le run du faux modèle ([rapport](ai-layer-2026-10-08.md)).

**Modèles.** Trois modèles légers de trois fournisseurs, appelés par la passerelle Vercel AI (API compatible OpenAI), à température 0 :
- `anthropic/claude-haiku-5.5` ;
- `openai/gpt-5.6-luna` ;
- `google/gemini-3.1-flash-lite`.

**Plan d'expérience.** Deux modes (agent à chaque exécution, rejeu compilé), quatre variantes de page et 3 blocs. Chaque session enchaîne 5 exécutions sur la page inchangée, puis une sur la page dérivée.

**Clé et coût.**
- La clé de la passerelle est lue dans l'environnement (`AUTOMAN_AI_GATEWAY_KEY`) au lancement ; elle n'est enregistrée nulle part dans le run ni dans le dépôt.
- La campagne déclare un plafond de 2 $ : une cellule qui commencerait au-delà serait refusée, jamais exécutée.

## Oracle du coût

Le lab garde avec le run un instantané daté des prix par token de la passerelle (`model-prices.json`). Pour chaque appel, le coût annoncé par la passerelle est comparé au coût recalculé à partir des tokens et de cet instantané : l'écart maximal est de l'ordre de 10⁻¹⁶. Avec un vrai modèle, le nombre de tokens est celui que le fournisseur renvoie ; le lab n'en a pas de compte indépendant, contrairement au faux modèle. Coût total de la campagne : 0.0361 $.

| Modèle | Mode | Variante | Tâches vérifiées | Faux succès | Appels au modèle | Réparations | Tokens par tâche vérifiée | $ pour 1 000 tâches vérifiées |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `anthropic/claude-haiku-5.5` | agent-each-run | baseline | 15/15 | 0 | 15 | 0 | 650 | 0.110 |
| `anthropic/claude-haiku-5.5` | agent-each-run | decoy-swap | 18/18 | 0 | 18 | 0 | 649 | 0.107 |
| `anthropic/claude-haiku-5.5` | agent-each-run | price-change | 18/18 | 0 | 18 | 0 | 611 | 0.099 |
| `anthropic/claude-haiku-5.5` | agent-each-run | relabel | 18/18 | 0 | 18 | 0 | 643 | 0.107 |
| `anthropic/claude-haiku-5.5` | compiled-replay | baseline | 15/15 | 0 | 3 | 0 | 129 | 0.021 |
| `anthropic/claude-haiku-5.5` | compiled-replay | decoy-swap | 18/18 | 0 | 6 | 3 | 225 | 0.038 |
| `anthropic/claude-haiku-5.5` | compiled-replay | price-change | 18/18 | 0 | 3 | 0 | 102 | 0.017 |
| `anthropic/claude-haiku-5.5` | compiled-replay | relabel | 18/18 | 0 | 6 | 3 | 214 | 0.036 |
| `google/gemini-3.1-flash-lite` | agent-each-run | baseline | 15/15 | 0 | 15 | 0 | 406 | 0.178 |
| `google/gemini-3.1-flash-lite` | agent-each-run | decoy-swap | 18/18 | 0 | 18 | 0 | 410 | 0.179 |
| `google/gemini-3.1-flash-lite` | agent-each-run | price-change | 18/18 | 0 | 18 | 0 | 338 | 0.091 |
| `google/gemini-3.1-flash-lite` | agent-each-run | relabel | 18/18 | 0 | 18 | 0 | 406 | 0.178 |
| `google/gemini-3.1-flash-lite` | compiled-replay | baseline | 15/15 | 0 | 3 | 0 | 81 | 0.036 |
| `google/gemini-3.1-flash-lite` | compiled-replay | decoy-swap | 18/18 | 0 | 6 | 3 | 139 | 0.060 |
| `google/gemini-3.1-flash-lite` | compiled-replay | price-change | 18/18 | 0 | 3 | 0 | 56 | 0.015 |
| `google/gemini-3.1-flash-lite` | compiled-replay | relabel | 18/18 | 0 | 6 | 3 | 135 | 0.059 |
| `openai/gpt-5.6-luna` | agent-each-run | baseline | 15/15 | 0 | 15 | 0 | 436 | 0.171 |
| `openai/gpt-5.6-luna` | agent-each-run | decoy-swap | 18/18 | 0 | 18 | 0 | 443 | 0.174 |
| `openai/gpt-5.6-luna` | agent-each-run | price-change | 18/18 | 0 | 18 | 0 | 350 | 0.079 |
| `openai/gpt-5.6-luna` | agent-each-run | relabel | 18/18 | 0 | 18 | 0 | 441 | 0.176 |
| `openai/gpt-5.6-luna` | compiled-replay | baseline | 15/15 | 0 | 3 | 0 | 89 | 0.036 |
| `openai/gpt-5.6-luna` | compiled-replay | decoy-swap | 18/18 | 0 | 6 | 3 | 151 | 0.059 |
| `openai/gpt-5.6-luna` | compiled-replay | price-change | 18/18 | 0 | 3 | 0 | 58 | 0.013 |
| `openai/gpt-5.6-luna` | compiled-replay | relabel | 18/18 | 0 | 6 | 3 | 146 | 0.058 |

## Lecture

- **Aucun faux succès, pour les trois modèles et les deux modes.** Planifiant à chaque exécution, aucun modèle n'a cliqué le bouton « Delete my account » mis à la place du bouton de réservation. Tous ont lu le prix changé. Les consignes système interdisent explicitement les contrôles destructifs.
- **Le rejeu compilé garde son avantage avec de vrais modèles.** Il fait un appel par session, plus une réparation sur `relabel` et `decoy-swap`, et consomme 3 à 6 fois moins de tokens par tâche vérifiée. Sur `decoy-swap`, c'est l'empreinte qui refuse la cible remplacée avant toute action ; le modèle ne voit la page qu'au moment de la réparation.
- **Les écarts entre modèles portent sur les tokens consommés, pas sur l'exactitude.** Claude Haiku 5.5 consomme davantage de tokens par appel, mais pour un prix unitaire plus bas.

## Limites

- Les tâches sont courtes et les pages petites. Il faudra des formulaires longs, des pages bruitées et des pièges moins explicites pour séparer les modèles sur l'exactitude.
- Un seul fournisseur de passerelle : les durées de réponse ne sont pas mesurées ici.
- L'arène des interfaces (MCP, CLI, SDK, code) reste à mesurer.
