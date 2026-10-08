# Politique d'accès : robots.txt et page de challenge, 8 octobre 2026

Contrats de l'ADR-002 testables sur les backends existants, en `local-dev` sur sources commitées. Run retenu : `access-policy-2026-10-08T17-15-34-616Z-cad714`. 4 backends, 3 variantes, 3 blocs. La campagne demande le respect de robots.txt : Crawlee reçoit `respectRobotsTxtFile`, Spider `respect_robots_txt` ; node-http n'a pas d'option équivalente.

| Backend | Normal | Page interdite par robots.txt | Page de challenge en 200 |
| --- | --- | --- | --- |
| node-http (fetch) | ok | **la demande quand même** | **faux succès** |
| crawlee-http | ok | respectée | **faux succès** |
| crawlee-http-pooled | ok | respectée, mais l'adapter reste bloqué jusqu'au timeout | **faux succès** |
| spider-http | ok | respectée | **faux succès** |

- Aucun backend ne reconnaît une page de challenge servie en 200 : tous rendent les données partielles comme un succès. La détection de challenge reste à construire dans le moteur, et cette variante en est le contrat.
- Crawlee mutualisé écarte bien la page interdite, mais son suivi de fin de tâche ne compte pas la requête écartée : la tâche n'aboutit jamais.

Content-Signal, TDMRep et Web Bot Auth supposent un moteur qui déclare une finalité ou signe ses requêtes ; ils seront testés avec le prototype du profil d'identité.
