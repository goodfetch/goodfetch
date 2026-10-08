# Incidents HTTP du 8 octobre 2026

Matrice de contrat de D-http-transport, exécutée en `local-dev` sur sources commitées. Run retenu : `http-incidents-2026-10-08T17-07-35-305Z-73011e`. 4 backends, 7 variantes, 3 blocs. Les adapters sont ceux du lab, tels que configurés pour les benchmarks : aucun ne relance une requête échouée.

| Backend | Normal | 429 + Retry-After | 503 unique | Tronqué | Coupé | Boucle de redirections | Serveur lent |
| --- | --- | --- | --- | --- | --- | --- | --- |
| node-http (fetch) | ok | erreur, sans relance | erreur, sans relance | erreur explicite | erreur explicite | erreur explicite | erreur explicite |
| crawlee-http | ok | erreur, sans relance | erreur, sans relance | erreur explicite | erreur explicite | erreur explicite | données complètes, échéance ignorée |
| crawlee-http-pooled | ok | erreur, sans relance | erreur, sans relance | erreur explicite | erreur explicite | erreur explicite | données complètes, échéance ignorée |
| spider-http | ok | **faux succès** | **faux succès** | **faux succès** | **faux succès** | **faux succès** | **faux succès** |

- Spider HTTP abandonne la page en incident et rend le reste comme un succès : il manque des enregistrements, sans aucune erreur. C'est le seul backend qui produit des faux succès.
- Aucun backend ne respecte `Retry-After` ni ne relance après un 503 : la politique de relance reste à construire dans le moteur, et ces deux variantes en sont le contrat.
- Crawlee n'applique pas l'échéance de tâche à ses requêtes HTTP : il attend la réponse lente et la rend complète.

Un premier run (`http-incidents-2026-10-08T17-03-09-936Z-726cb0`) donnait les mêmes résultats, avec des raisons moins précises, sur des sources non commitées.
