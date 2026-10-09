# Mesures sur Mac au repos du 8 octobre 2026

Campagnes de vitesse et de ressources exécutées en classe `local-controlled`, sur des sources commitées. Les estimations, intervalles et verdicts sont calculés par `pnpm lab verdict` et rendus dans [decisions.md](decisions.md). Ce rapport ne recopie aucun chiffre mesuré.

## Hôte

Ce jour-là, un Mac au repos ne passait pas les premiers seuils de `local-controlled`. Deux causes : la maintenance de macOS lancée dès que la machine est inactive (Spotlight, Siri, file provider), et l'app d'agent qui pilote la campagne. Les seuils ont donc été ajustés et commités avant les mesures (`hosts/local-controlled.json`) :

- l'app d'agent qui pilote le run est consignée à part (`controllerCpuPercent`) au lieu de compter comme charge étrangère ;
- le CPU étranger autorisé passe à 150 % et la charge par CPU à 0,35 ;
- le branchement secteur n'est plus exigé, le mode basse consommation restant interdit.

Les blocs sont appariés, avec les pilotes entrelacés dans un ordre tiré au sort à chaque bloc : un bruit de fond lent touche toutes les options d'un bloc et s'annule dans le ratio. Tout bloc où le contrôle d'hôte échoue est exclu de l'estimation. Les chiffres absolus restent non publiables jusqu'aux reruns sur l'hôte de référence.

## Runs

| Run | Décision | Contenu |
| --- | --- | --- |
| `browser-driver-rtt-0-2026-10-08T12-39-53-109Z-63890b` | D-browser-driver | lien driver-browser à 0 ms ; bloc 0 bruité, exclu |
| `browser-driver-rtt-20-2026-10-08T12-50-54-854Z-96969e` | D-browser-driver | lien driver-browser à 20 ms |
| `browser-driver-rtt-100-2026-10-08T13-14-24-301Z-331916` | D-browser-driver | lien driver-browser à 100 ms |
| `browser-driver-rtt-300-2026-10-08T14-00-36-292Z-657a67` | D-browser-driver | lien driver-browser à 300 ms |
| `runtime-native-capacity-2026-10-08T12-46-38-910Z-83d7e0` | D-core-language | strate de décision, Node contre Rust, huit blocs |

Toutes les tâches de ces runs ont été vérifiées par l'oracle. Le verdict principal de D-browser-driver porte sur la strate à 0 ms, déclarée dans la décision ; les autres RTT décrivent la sensibilité à la latence. Le run `runtime-native-capacity` remplace `runtime-native-qualified-2026-10-08T00-11-54-841Z-789d5e`, mesuré en `local-dev`, comme mesure de D-core-language.

## Tentatives du 9 octobre

Douze runs de six campagnes ont été relancés en demandant `local-controlled`. Tous sont classés `diagnostic` ; leurs comptes sont exacts, et leurs seuls échecs sont des délais dépassés et rendus explicites.

- **Nuit** (à partir de 3 h 35, Mac au repos). Les campagnes s'enchaînaient sans pause, si bien que la charge d'un run pesait encore sur la vérification d'hôte du suivant ; trois runs ont été rétrogradés en `local-dev`. Une campagne d'identité s'est bloquée quatre heures sur une cellule après la mise en veille du Mac. Correctifs :
  - commandes CDP bornées à 60 s et budget de temps par cellule (#42) ;
  - lancements sous `caffeinate -i` ;
  - attente du calme avant chaque campagne.
- **Matin** (Mac en usage léger). Tous les runs sont restés en `local-dev` : l'analyse des photos de macOS, la VM de Docker Desktop et l'affichage dépassaient le CPU étranger autorisé.

Les reruns `local-controlled` restent donc à faire.
