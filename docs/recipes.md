---
title: Recipes
description: Write a Recipe's manifest, goodfetch-recipe.toml, choose its entry and runtime, and see which files its package holds.
products: [collie, foxhound]
---

# Recipes

This guide is for a person who packages an automation as a Recipe, so that it runs the same on their machine and on a hosted engine. A Recipe is a folder whose root holds its manifest, `goodfetch-recipe.toml`.

## The manifest

```toml
version = "0.1"
name = "price-watch"
product = "collie"

[entry]
program = "src/main.ts"

[runtime]
language = "node"
version = "24"

[settings.timeout]
navigation = 20000

[expose."timeout.navigation"]
minimum = 5000
maximum = 60000
```

- `version`: the version of the manifest format, `0.1`. An engine refuses a manifest newer than it reads with `format_version_unsupported`, which names the version to upgrade to.
- `name`: lowercase ASCII letters, digits, `-` and `_`, opening with a letter, at most 64 characters.
- `product`: the Product the Recipe needs, `collie` or `foxhound`. Collie's engine refuses a Recipe that needs Foxhound.
- `entry`: exactly one of the three kinds below, each the path of a file of the package from its root, with `/` between its parts.
- `runtime`: the language and version a program runs on. A step list and a crawl declaration name none.
- `settings`: the settings the Recipe sets, nested by key, each locked for its users.
- `expose`: the settings its users may still set, by key, with a `minimum` and `maximum` for a count or a duration, or the values they may give in `one_of`.

Any other field is refused. Every refusal is `recipe_invalid`, whose details name the `field`, the `reason` and the `fix`.

The bundle's JSON Schema of the manifest, the format `recipe-manifest`, lets an editor check it as you type.

## The entry

| Entry | What it is | Product | Runtime |
|---|---|---|---|
| `program` | the main source file of a program | Collie or Foxhound | required |
| `steps` | a list of steps, replayed with no program and no model | Collie or Foxhound | none |
| `crawl` | a crawl declared as data | Foxhound only | none |

A Recipe of steps or of a crawl declaration holds no source.

## The runtime

| `language` | `version` |
|---|---|
| `node` | `24`, `26` |
| `python` | `3.11`, `3.12`, `3.13`, `3.14` |
| `rust` | `1.97`, `1.98`, `1.99` |

These are the versions the SDKs are tested on and, for Rust, the workspace's oldest supported toolchain to its newest. A version outside the list is refused.

## What the package holds

The package is every file of the folder, the manifest, the source and the lockfiles included, except:

- what a `.goodfetchignore` file leaves out, in `.gitignore`'s syntax; a `.goodfetchignore` in a subfolder applies beneath it, as a `.gitignore` does;
- what a package always leaves out, which no `.goodfetchignore` brings back: `.git/`, `.env`, `.env.*`, `node_modules/`, `.venv/`, `venv/`, `__pycache__/`, `target/`, `dist/` and `build/`, at any depth.

A package holds no symbolic link: one is refused, naming its path. The entry must be one of the package's files, and the manifest may not be ignored.
