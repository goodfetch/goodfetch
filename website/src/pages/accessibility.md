---
title: Accessibility statement
description: The accessibility these docs aim for, their known exceptions and how to report a barrier.
---

# Accessibility statement

Goodfetch wants every reader to be able to use these docs.

## The target

These docs aim to conform to the [Web Content Accessibility Guidelines (WCAG) 2.2](https://www.w3.org/TR/WCAG22/) at level AA. So does the Live view: its own controls, its accessible name and its announcements. The remote page the Live view shows is a third-party site's content, outside Goodfetch's reach, so for it the conformance is partial.

## How it is checked

- Every page of the docs is checked with axe against WCAG 2.2 A and AA, in the light and the dark theme, before each change is merged.
- Each new page is reviewed for what no tool judges: names and instructions, consistent help and cues that rely on one sense only.
- The command-line tools never carry information by colour or motion alone, and honour `NO_COLOR`.

## Known exceptions

None is known. Each one found is listed here, with what it affects and when it will be fixed.

## Report a barrier

If something in these docs is hard or impossible for you to use, [open an issue](https://github.com/goodfetch/goodfetch/issues/new) that says which page, what you tried and what assistive technology you use, if any.
