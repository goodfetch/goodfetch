# Hôte de référence Linux

Ce document décrit la préparation de l'hôte de classe `reference`, la seule classe dont les chiffres peuvent être publiés. L'hôte n'est pas encore provisionné (phase P9). D'ici là, une décision vaut au mieux `local-controlled`.

Les gates appliquées automatiquement sont dans [`lab/hosts/reference.json`](../lab/hosts/reference.json). `pnpm lab preflight reference` les vérifie avant chaque run et le moteur les revérifie à chaque bloc ; un hôte qui échoue est refusé, jamais dégradé en silence.

## Matériel et système

- Machine physique dédiée (gate `virtualized = false`) : pas de VM, pas d'instance cloud partagée. Une instance *bare metal* louée convient.
- Linux récent avec cgroup v2 unifié (`/sys/fs/cgroup/cgroup.controllers` présent), `systemd` et `/proc/<pid>/smaps_rollup` (noyau ≥ 4.14).
- Architecture x86_64 ou arm64, consignée dans chaque run. Un changement de machine ou d'architecture ouvre une nouvelle série : les runs ne se comparent qu'entre eux sur le même hôte.
- Aucun autre service actif pendant les campagnes (gate `foreign-cpu-percent ≤ 10`, charge par CPU ≤ 0,1, mémoire libre ≥ 30 %).

## Réglages CPU

À appliquer à chaque démarrage, par exemple via une unité systemd :

```bash
sudo cpupower frequency-set -g performance
```

- Gouverneur `performance` sur tous les cœurs (gate `cpu-governor`).
- Turbo et SMT : choisir une politique, la garder fixe pour toute la série et la consigner dans la description de l'hôte. Désactiver le turbo réduit la variance ; garder le SMT rapproche des conditions de production. Changer l'un ou l'autre ouvre une nouvelle série.
- Pas d'alerte thermique pendant un bloc (gate `thermal-warning`).

## Mémoire et CPU par run

Les sondes `/proc/<pid>/stat` (CPU) et `smaps_rollup` (PSS anonyme + swap) sont actives dès que l'hôte est Linux. La métrique `cgroup-memory-peak-bytes` (`memory.peak`, cache de pages inclus) et le CPU cgroup des enfants terminés demandent un scope systemd par run :

```bash
systemd-run --user --scope -p MemoryAccounting=yes -p CPUAccounting=yes pnpm lab run <campagne>
```

Le parseur existe (`lab/src/core/metrics/linux.ts`). La création du scope par le moteur est à brancher au provisionnement de l'hôte ; d'ici là, ces deux sondes sont déclarées indisponibles avec leur raison.

## Réseau

- RTT seul : le proxy de latence en espace utilisateur du lab (`lab/src/net/latency-proxy.ts`) fonctionne sans privilège ; chaque run consigne sa calibration appariée.
- Perte, gigue, débit : `tc netem` sur une interface dédiée ou un namespace réseau, réglé par l'opérateur. Le lab détecte et consigne le profil, il ne le règle pas lui-même.

## Toolchains et sources

- Node, pnpm, Rust et Go aux versions épinglées du lab (`.node-version`, `package.json`, lockfiles), puis `pnpm install --frozen-lockfile` et `pnpm bootstrap`.
- Le build SQLite natif (`.so`) doit être qualifié sur Linux avant toute campagne `sqlite-readers` en classe `reference`.
- Sources commitées et propres (gate `git-clean`) : un run sur des sources modifiées est plafonné à `local-dev`.

## Mise en service

1. Provisionner la machine et appliquer les réglages ci-dessus.
2. `pnpm lab ci` doit être vert.
3. `pnpm lab preflight reference` doit accorder la classe `reference`.
4. Lancer la suite de contrat (`pnpm lab suite campaigns/v2/suites/qualification-current.json`), qui doit reproduire les comptes attendus.
5. Lancer les reruns de priorité 0 listés dans le ledger (`rerun.priority`), puis les priorités 1 et 2.

## Répétition dans un conteneur

`lab/docker/linux.Dockerfile` construit une image Linux à partir des seules sources commitées. Le 9 octobre 2026, cette image a passé le bootstrap, la CI et toute la suite de contrat ([rapport](../lab/reports/linux-container-2026-10-09.md)). Un conteneur dans une machine virtuelle reste de classe `local-dev` : il valide le chemin Linux, pas les durées.

