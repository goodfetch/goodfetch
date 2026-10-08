# Lightpanda (D-light-engine), 8 octobre 2026

Lightpanda 1.0.0 (AGPL-3.0, binaire épinglé, toujours hors processus), soumis aux mêmes pilotes, scénarios et oracles que Chromium, en `local-dev` sur sources commitées. Les quatre pilotes (Rust CDP, Node CDP, Puppeteer, Playwright) s'y connectent par CDP ; leurs résultats sont identiques, les échecs viennent donc du moteur.

Runs retenus : `light-engine-compat-native-2026-10-08T18-16-40-008Z-f659d1`, `light-engine-compat-guarded-2026-10-08T18-17-15-329Z-6d44f1`, `light-engine-actionability-2026-10-08T18-17-53-360Z-1d5ca3`, `light-engine-primitives-2026-10-08T18-19-11-980Z-d7f0e3`, `light-engine-navigation-2026-10-08T18-21-06-891Z-318afe`.

| Domaine | Fonctionne | Échoue |
| --- | --- | --- |
| Lecture | catalogue HTTP, catalogue rendu en JavaScript | — |
| Formulaires et clics | formulaire, clics répétés, événements `isTrusted` | shadow DOM (aucun pilote ne termine) |
| Primitives | double clic, clic droit, molette, Entrée, saisie Unicode, cible déplacée | survol, glisser-déposer (aucun mouvement de pointeur), `contenteditable` |
| Navigation | route SPA, envoi de fichier | retour dans l'historique, popup, iframe de même origine et d'une autre origine, téléchargement |
| Actionnabilité | cible normale ; cible retardée, cachée ou désactivée dans la majorité des cellules | **calque, cible couverte, cible ambiguë, cible en mouvement** |

- **Faux succès sous garde.** Sans modèle d'empilement ni hit-test, `elementFromPoint` rend la cible même sous un calque, et une cible dupliquée n'est pas vue comme ambiguë : le clic part. La garde Automan ne peut pas garantir l'actionnabilité sur ce moteur.
- `retarget` échoue parce que Lightpanda n'émet pas `pointerover` : la page ne remplace jamais la cible, et le test ne prouve rien dans ce cas.
- Conclusion de contrat : Lightpanda convient à la lecture de pages simples, pas aux actions qui exigent des garanties, ni aux portails à iframes, popups ou téléchargements. Sa vitesse et ses ressources restent à mesurer sur un hôte au calme.
