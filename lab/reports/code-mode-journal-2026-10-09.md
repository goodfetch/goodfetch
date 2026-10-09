# Mode code avec journal d'effets (D-agent-interface), 9 octobre 2026

La première arène ([rapport](interface-arena-2026-10-09.md)) a bloqué le mode code sur la porte d'exactitude. Quand GPT-5.6 Luna oubliait `page.done()` et qu'on le lui signalait, il réécrivait tout le programme. La version corrigée rejouait saisies et clics sur une page qui les avait déjà reçus : case de consentement décochée, réservation envoyée deux fois.

## Le garde-fou

`code-mode-journal` exécute le même programme, mais chaque effet (`fill`, `select`, `click`) est inscrit dans un journal. Une correction est rejouée contre ce journal depuis le début :
- une action identique à celle du journal au même rang est sautée et rendue comme « déjà appliquée » ;
- une action différente, alors qu'un clic du journal reste devant elle, arrête le programme avec `E_EFFECT_DIVERGENCE`. Un clic déjà envoyé ne se réconcilie pas. L'arrêt tient même si le programme attrape l'erreur.
- `fill` et `select` fixent une valeur et sont idempotents : sans clic à venir dans le journal, une divergence sur eux reprend en direct.

Le message de correction liste les actions déjà reçues par la page. Un test d'intégration avec un modèle scripté reproduit le cas de l'arène : sans journal la réservation est réfutée, avec journal les cinq actions sont sautées et la tâche est vérifiée.

## Run

`interface-arena-journal-2026-10-09T15-11-59-464Z-f3de1a`, `local-dev`, sources commitées. Il compte 5 interfaces, 3 modèles réels via la gateway Vercel AI, 3 variantes de page et 5 blocs de 3 tâches, soit 675 tâches pour 0,42 $ au total.

| Interface | Modèle | Tâches vérifiées | Faux succès | Non terminées | Tours par tâche vérifiée | Tokens par tâche vérifiée | $ pour 1 000 tâches vérifiées |
| --- | --- | --- | --- | --- | --- | --- | --- |
| mode code | `anthropic/claude-haiku-5.5` | 45/45 | 0 | 0 | 1,0 | 716 | 0,16 |
| mode code | `google/gemini-3.1-flash-lite` | 45/45 | 0 | 0 | 1,0 | 363 | 0,15 |
| mode code | `openai/gpt-5.6-luna` | 28/45 | **17** | 0 | 2,2 | 1057 | 0,52 |
| mode code + journal | `anthropic/claude-haiku-5.5` | 45/45 | 0 | 0 | 1,0 | 718 | 0,16 |
| mode code + journal | `google/gemini-3.1-flash-lite` | 45/45 | 0 | 0 | 1,0 | 363 | 0,15 |
| mode code + journal | `openai/gpt-5.6-luna` | **45/45** | **0** | 0 | 1,4 | 772 | 0,42 |
| MCP | `anthropic/claude-haiku-5.5` | 45/45 | 0 | 0 | 5,7 | 11913 | 1,66 |
| MCP | `google/gemini-3.1-flash-lite` | 45/45 | 0 | 0 | 5,7 | 2379 | 0,70 |
| MCP | `openai/gpt-5.6-luna` | 45/45 | 0 | 0 | 3,9 | 2113 | 0,58 |
| outils de fonction (sdk) | `anthropic/claude-haiku-5.5` | 44/45 | 0 | 1 | 5,2 | 10475 | 1,45 |
| outils de fonction (sdk) | `google/gemini-3.1-flash-lite` | 45/45 | 0 | 0 | 5,7 | 2359 | 0,71 |
| outils de fonction (sdk) | `openai/gpt-5.6-luna` | 45/45 | 0 | 0 | 3,9 | 2125 | 0,58 |
| CLI | `anthropic/claude-haiku-5.5` | 27/45 | 0 | 18 | 9,6 | 13447 | 2,01 |
| CLI | `google/gemini-3.1-flash-lite` | 45/45 | 0 | 0 | 4,3 | 1644 | 0,58 |
| CLI | `openai/gpt-5.6-luna` | 45/45 | 0 | 0 | 4,3 | 2014 | 0,59 |

## Lecture

- **Le journal supprime la faille sans rien coûter aux modèles qui ne la déclenchent pas.** Les 17 faux succès de GPT-5.6 Luna en mode code tombent à 0. Ses corrections sont sautées au lieu d'être rejouées, ce qui lui coûte 1,4 tour par tâche au lieu de 2,2. Claude Haiku 5.5 et Gemini Flash Lite écrivent un programme complet du premier coup : leurs chiffres ne bougent pas.
- **Le mode code avec journal est l'interface la moins chère et la seule à un tour.** Il demande 3 à 15 fois moins de tokens que les outils de fonction et 4 à 10 fois moins de dollars, pour la même exactitude.
- Les constats de la première arène tiennent avec 5 blocs. MCP et les outils de fonction coûtent pareil. La CLI dépend du modèle : Claude Haiku laisse 18 tâches sur 45 sans les déclarer finies.
- Aucun modèle n'a cliqué le contrôle destructif, quelle que soit l'interface.

## Verdict mécanique

`pnpm lab verdict D-agent-interface`, en tours de modèle par tâche vérifiée, contre MCP. Les tours et les tokens étant comptés par la gateway, la décision se prend en `local-dev`.

| Option | Page de réservation | Leurre destructif | Lecture de prix |
| --- | --- | --- | --- |
| outils de fonction (sdk) | équivalent (0,95) | équivalent (1,01) | équivalent (0,98) |
| CLI | `insufficient-n` (1,09) | `insufficient-n` (1,05) | **moins bon** (1,43) |
| mode code | bloqué par le contrat | bloqué | bloqué |
| mode code + journal | `insufficient-n` (0,20) | `insufficient-n` (0,20) | **meilleur** (0,34) |

Le mode code journalisé demande 3 à 5 fois moins de tours que MCP. Sur les tâches de réservation, la règle réclame 93 à 99 blocs, parce que GPT-5.6 Luna y ajoute une correction dans certains blocs et pas dans d'autres, ce qui disperse les rapports. L'écart n'est pas en cause : l'intervalle reste entre 0,16 et 0,26.

## Ce que cela dit pour l'interface d'Automan

1. **Le mode code est offert, et ses effets sont journalisés par défaut.** C'est la même règle que le rejeu compilé de la couche IA : une correction s'exécute contre un état connu et ne renvoie jamais un effet déjà reçu par le site.
2. Le journal est positionnel et refuse de réconcilier un clic. C'est volontairement strict : une action non idempotente déjà envoyée appelle une revue (`effect_unknown`), pas une supposition.
3. MCP et outils de fonction restent deux emballages d'un même catalogue d'opérations. La CLI sert l'humain et les scripts, pas d'abord les agents.

## Limites

- Tâches courtes sur une seule page. Un parcours long multiplie les occasions de divergence ; le journal reste à mesurer sur plusieurs pages et formulaires bruités.
- Durées hors verdict : elles contiennent la latence des modèles, sur un hôte `local-dev`.
