---
title: Response cache
description: How the Response cache serves a later Run's requests on every tier, what it keeps apart, and why a request a site's service worker answers never reaches it.
products: [collie, foxhound]
---

# Response cache

This guide is for a person who wants a later Run to reuse the responses an earlier one fetched, rather than ask the site for each page again, and who needs to know why a request was not served from the cache.

The Response cache is kept per user on the machine, outside any Run, and every Run of that user may read it (ADR 0046). It is off by default.

## Modes

`cache.mode` takes one of three values:

- `off`, the default: the cache stores and serves nothing, so a Run never serves a stale page without saying so.
- `revalidate`: the cache follows HTTP's caching rules (RFC 9111). It stores `GET` and `HEAD` responses, serves a fresh one with no request, and asks the site again about a stale one with `If-None-Match` and `If-Modified-Since`, serving the stored body when the site answers `304`.
- `replay`, for development: the cache stores every response, whatever its method, status or freshness, and serves it again with no request. A `POST` is matched on its body, with JSON keys and form fields in any order. A Schedule's target never sets `replay`.

An entry matches a request by its method, its URL without the fragment, the request headers the response names in `Vary` and, under `replay`, its body. A response sent with or setting a cookie or a credential, or marked `private`, is kept apart, in the partition of the Session that fetched it, which no other Session reads, until that Session ends. One fetched outside a Session is not stored.

The cache keeps at most `cache.max_size` bytes, 10 GB by default, evicting the entries used least recently, and deletes an entry older than `cache.max_age_ceiling`, 30 days by default. Both are set in user configuration or an Operator policy only.

## One cache for every tier

The `http` tier and the browser read and write the same entries. A page the `http` tier stored is served to a browser page that loads it, and a script, stylesheet or frame a browser page stored is served to a later `http` request for it, wherever the key and the partition match.

In a browser, the engine's interception serves the cache: a request the cache answers is fulfilled from the stored response with nothing sent to the site, and a stale entry is revalidated with the stored entry's conditions in place of the page's own. A page's scripts, stylesheets, images, calls and frames are served this way. In Collie a page's own navigations are too. In a Foxhound crawl, the navigation to the crawl request's own page always reaches the site, since the navigation gate and the page's checks judge it, and so does every request a site's service worker sends itself.

## Service workers

A request that a site's service worker answers never reaches the Response cache, which neither serves nor stores it (ADR 0121). The worker's fetch handler answers it from its own Cache Storage, or passes it on with a fetch of its own, and the page's request never pauses where the cache acts. Such a request is a miss, however often the Run sends it.

When the cache must serve the requests a site's service worker would answer, set `browser.service_workers` to `block` for the Session or the Run. No worker then registers, the page's own requests reach the cache, and a later Run is served what an earlier one stored. On Chromium, service workers run by default (`allow`); on Firefox and WebKit no service worker ever registers, so their pages' requests always reach the cache.
