---
title: Release notes template
description: Release notes sections for every minor and major release, including framework adapter peer ranges.
products: [collie]
---

# Release notes template

This template lists the sections every minor and major release page carries (ADRs 0030 and 0070).

## Highlights

- Summarize user-visible additions and changes by Product (Both Products, Foxhound only).

## Breaking changes and upgrades

- List any breaking changes and the `upgrade-files` command when a stored format changed.

## Deprecations

- Include the generated deprecated-settings list when non-empty.

## Framework adapter peer ranges

Each framework adapter declares the framework's current major at its first publish and the previous major when its tool interface is unchanged (ADR 0070). The Release suite tests the lowest and highest version of each declared peer range and one version per major inside it.

| Product | Adapter | Ecosystem | Peer package | Declared range |
| --- | --- | --- | --- | --- |
| Collie | `@goodfetch/collie-ai-sdk` | npm | `ai` | `^7.0.0` |
| Collie | `@goodfetch/collie-langchain` | npm | `@langchain/core` | `^1.0.0` |
| Collie | `goodfetch-collie-langchain` | PyPI | `langchain-core` | `>=1.0.0,<2` |
| Collie | `@goodfetch/collie-mastra` | npm | `@mastra/core` | `^1.0.0` |
