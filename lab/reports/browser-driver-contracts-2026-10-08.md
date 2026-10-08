# Pilotes navigateur : contrats du 8 octobre 2026

Matrices de contrat de D-browser-driver, exécutées en `local-dev` sur des sources commitées. Ce sont des résultats d'exactitude (classe A) : ils ne dépendent pas du bruit de la machine. Aucune vitesse n'est comparée ici.

## Matrice d'actionnabilité étendue

Run retenu : `browser-actionability-extended-2026-10-08T12-05-27-529Z-49432b`. 8 pilotes, 9 variantes, 2 politiques, 3 blocs.

- Tous les pilotes **gardés** passent les 27 cellules : Rust CDP, Node CDP, Puppeteer, Playwright, Stagehand, Stagehand batch, Crawlee Playwright et Crawlee Stagehand.
- En **natif**, Playwright et les deux Crawlee passent 27 sur 27.
- Puppeteer natif échoue sur calque, cible couverte, cible ambiguë et re-rendu.
- Les pilotes CDP bruts et les deux Stagehand natifs n'ont aucune garantie d'actionnabilité, et échouent sur toutes les variantes sauf `normal`.

## Primitives

Run retenu : `browser-driver-primitives-2026-10-08T12-14-55-485Z-4b6074`. 4 pilotes, 10 gestes, 2 politiques, 3 blocs.

- Les pilotes gardés et Playwright natif réussissent les 30 cellules.
- Les pilotes natifs sans interception (Rust CDP, Node CDP et Puppeteer) échouent sur `moved` et `retarget`.

## Deux défauts de la garde Automan, trouvés et corrigés

1. **Pas de contrôle pendant le geste.** Dans `browser-driver-primitives-2026-10-08T11-42-40-378Z-c8b654`, la garde v1 vérifiait la cible avant le geste seulement. Quand la page déplaçait la cible ou la remplaçait par un bouton « Supprimer » à l'arrivée du pointeur, tous les pilotes gardés cliquaient le leurre ou le contrôle destructeur. La garde v2 arme un écouteur en phase de capture sur l'élément vérifié : un événement qui atteint autre chose est annulé, puis le geste est rejoué jusqu'à l'échéance.
2. **Pause des frames après navigation.** Chromium headless peut suspendre `requestAnimationFrame` pendant environ une seconde après une navigation, alors que les timers continuent. Une garde rythmée par les frames dépassait alors son échéance sur la cible en mouvement. Elle échantillonne désormais sur une frame ou 20 ms, au premier des deux.

## Ce que ces résultats permettent

Les gates `actionability-conformance` et `false-success-count` de D-browser-driver ont désormais des preuves pour les quatre options comparées. Le verdict de vitesse attend les campagnes RTT sur une machine au calme.
