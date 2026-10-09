# Passage de contrat sous Linux, dans un conteneur, 9 octobre 2026

Répétition de la mise en service de l'hôte de référence ([docs/reference-host.md](../../docs/reference-host.md)) dans un conteneur Linux sur le Mac du porteur :
- Docker Desktop, Debian bookworm, arm64, cgroup v2 ;
- image `lab/docker/linux.Dockerfile`, construite à partir des seules sources commitées (`git archive`) ;
- toolchains épinglées : Node 25.5.0, pnpm 12.9.1, Rust 1.93.0, Go 1.27.1.

**Ce que vaut ce passage.** Le conteneur tourne dans une machine virtuelle, sur un hôte partagé. Ses runs sont de classe `local-dev`, et la classe `reference` exige une machine physique dédiée. Ils valident le chemin Linux (builds, sondes, comptes de contrat) ; leurs durées ne servent à aucune décision et ne seront jamais publiées.

## Étapes

1. `pnpm bootstrap` : dépendances, Chromium pour Linux arm64, adapters Rust (Spider, stockage, CDP, extraction, reqwest, wreq), SQLite partagé.
2. `pnpm workflow-build` : runtime Rust.
3. `pnpm lab ci` : vert.
4. `pnpm lab suite campaigns/v2/suites/qualification-current.json` : 6 étapes sur 6, aux comptes de la qualification macOS.

| Run | Tâches vérifiées |
| --- | --- |
| `runtime-native-qualified-2026-10-09T00-41-00-780Z-a4457d` | 30 |
| `runtime-recovery-qualified-2026-10-09T00-50-37-703Z-632789` | 42 |
| `browser-actionability-guarded-2026-10-09T00-51-42-342Z-19842f` | 162 |
| `sqlite-readers-2026-10-09T00-54-43-859Z-d4dfa6` | 108 |
| `spider-http-transport-supported-2026-10-09T00-55-30-963Z-582f6b` | 60 |
| `runtime-http-backends-2026-10-09T00-55-32-223Z-35f7fa` | 24 |

## Ce qu'il a fallu porter

- **SQLite partagé.** Il est construit en `libautoman_sqlite.so` (`-shared`), et l'addon Node et le moteur de stockage Go se lient à lui. Le toolchain Go est épinglé par plateforme, avec les sommes de contrôle officielles. Sur macOS, la reconstruction donne une bibliothèque identique octet pour octet.
- **Ressources du runtime.** Sous Linux, l'échantillonneur lit `/proc/<pid>/stat` et `smaps_rollup` (PSS) ; la sonde `proc_pid_rusage` reste propre à macOS.
- **Ledger autonome.** Les résumés des runs natifs n'existaient que dans `results/`, qui n'est pas versionné : sur un clone neuf, aucun verdict n'était recalculable et la CI échouait. Le ledger garde désormais une copie de chaque `run.v2.json`.
- **Bac à sable de Chromium.** Dans le conteneur, les adapters tournent en root et Chromium refuse d'y démarrer avec son bac à sable. Le premier passage donnait 108/162 sur la matrice d'actionnabilité : Puppeteer et Crawlee avec Stagehand ne démarraient pas. Ils lancent désormais Chromium avec `--no-sandbox` en root sous Linux, comme Playwright le fait déjà.

## Reste pour l'hôte de référence

- Une machine physique dédiée, avec le gouverneur `performance` et les gates de `hosts/reference.json`.
- Les scopes systemd par run, pour `cgroup-memory-peak-bytes` et le CPU des enfants terminés.
- Lightpanda n'est épinglé que pour macOS arm64 ; ses campagnes ne tournent pas encore sous Linux.
