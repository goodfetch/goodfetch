# Reproduire les qualifications du 8 octobre

Depuis `/Users/jb/Dev/automan/lab`, avec le Node épinglé dans `.node-version` et les dépendances/bootstrap déjà construits :

```sh
node --test --test-concurrency=1 tests/*.test.ts tests/integration/*.test.ts
./node_modules/.bin/tsc --noEmit
cargo fmt --manifest-path adapters/workflow-rust/Cargo.toml --check
node tools/qualification-suite.ts
node tools/qualification-current.ts
```

Les deux suites créent de nouveaux dossiers uniques, sans remplacer les précédents. `qualification-suite.ts` couvre les cinq domaines et les arrivées ouvertes ; `qualification-current.ts` couvre les corrections du lab et les transports Spider restants. Ne pas lancer les suites ou builds en parallèle : les ressources mesurées se partageraient la machine.

La première suite retenue est `results/qualification-integrated-2026-10-07T23-21-58-974Z-6b64b0`. La requalification est `results/qualification-current-2026-10-08T00-11-54-553Z-21d1f2`. Chacune conserve sa copie de sources et les chemins de ses campagnes dans son `summary.json`.

Pour régénérer l'analyse, fournir la première suite, les cinq dossiers de campagne référencés par la seconde, puis la seconde suite elle-même. Celle-ci fournit les sources figées des campagnes HTTP qui ne copient pas leur propre source :

```sh
node reports/analyze-integrated.mjs results/qualification-integrated-2026-10-07T23-21-58-974Z-6b64b0 <native> <recovery> <browser> <sqlite-readers> <spider-transport> results/qualification-current-2026-10-08T00-11-54-553Z-21d1f2
python3 reports/audit-integrated.py results/qualification-current-2026-10-08T00-11-54-553Z-21d1f2
python3 reports/render-integrated.py results/qualification-current-2026-10-08T00-11-54-553Z-21d1f2
python3 reports/update-integrated-index.py results/qualification-integrated-2026-10-07T23-21-58-974Z-6b64b0 results/qualification-current-2026-10-08T00-11-54-553Z-21d1f2
```

Les placeholders doivent être remplacés par les dossiers exacts du résumé de requalification. L'analyse s'exécute après les benchmarks. L'audit distingue conformité des comparateurs, correction des erreurs du lab et invariants des données brutes. Une suite terminée n'implique pas que chaque backend ait réussi : les états et effectifs sont conservés dans le rapport transversal.
