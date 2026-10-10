# Engines, workers and browsers: Stagehand, Crawlee and Spider-rs

Survey of October 7, 2026. The framework's engine orchestrates the work; the browser performs rendering and executes the pages' JavaScript; the driver library transmits commands to the browser. "Worker" can designate an async task, an extension component or a distributed process, depending on the project.

## Stagehand v4

The runtime is written in TypeScript and distributed as JavaScript in a **Chrome extension**, notably its service worker. The TS/Python/Go SDKs run in the application's process and talk to the runtime via JSON-RPC carried over CDP. The service worker is a browser component, not a distributed job queue. Stagehand v4 drives Chromium/Chrome over CDP and no longer depends on Playwright. [Repository architecture](https://github.com/browserbase/stagehand/blob/main/CONTRIBUTING.md), [official migration](https://docs.stagehand.dev/v4/migrations/playwright), [placement and transport](https://www.browserbase.com/blog/stagehand-v4).

## Crawlee TypeScript

The framework is written in **TypeScript, executed as JavaScript in Node.js**. `AutoscaledPool` orchestrates concurrent asynchronous tasks based on resources; `BrowserPool` manages browsers and pages. These are responsibilities distinct from distributing jobs across several machines. The browser backends are **Playwright or Puppeteer**; the possible browsers include Chromium/Chrome, Firefox and WebKit depending on the backend. The HTTP paths do not require a browser and can use a native transport such as Impit. [Introduction](https://crawlee.dev/js/docs/introduction), [async pool](https://crawlee.dev/js/api/core/class/AutoscaledPool), [Impit](https://crawlee.dev/js/docs/guides/impit-http-client).

## Crawlee Python

This implementation has a **Python** core, an async `AutoscaledPool` and a browser pool. The standard navigation backend is **Playwright**; the plugin indicates Chromium by default and allows Firefox, WebKit and Chrome depending on configuration. The Stagehand integration is a distinct backend, with its own context and browser limits. [Architecture](https://crawlee.dev/python/docs/guides/architecture-overview), [async pool](https://crawlee.dev/python/api/class/AutoscaledPool), [Playwright plugin and browser types](https://crawlee.dev/python/api/class/PlaywrightBrowserPlugin).

## Spider-rs — engine, workers and browser

Check of October 7, 2026, on the published version **2.53.9**, SHA **`e4b53c5b582357e46433fa9ba3d91f930fb67cb4`**.

| Layer | Technology actually used |
|---|---|
| Engine | Rust; HTTP acquisition mainly via `reqwest`, parsing and crawl policies in Spider. |
| Normal concurrency | **Tokio** asynchronous tasks, controlled by semaphores. This does not mean one browser process per worker. |
| Optional browser | **Chromey**, a Rust CDP client, drives a local **Chromium/Chrome** or a browser connected via CDP. The browser handles rendering and JavaScript execution. Cargo features `chrome`/`smart` required depending on the chosen path. |
| Optional distribution | `decentralized` feature and OSS binary **`spider_worker`**. Rust server **Warp 0.4**, HTTP transport **Hyper 1 + hyper-util**, on Tokio; optional TLS. |
| Rendering of the provided distributed worker | Code `Page::new_page_streaming()` or `fetch_page_html_raw()`: HTTP fetch/parsing. Its manifest does not enable Chrome; this worker must not be presented as a guaranteed JavaScript rendering worker. |

Frozen sources: [engine dependencies and features](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider/Cargo.toml), [worker manifest](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_worker/Cargo.toml), [server and HTTP handlers](https://github.com/spider-rs/spider/blob/e4b53c5b582357e46433fa9ba3d91f930fb67cb4/spider_worker/src/main.rs).

The local workers, the OSS distributed worker and the private **Spider Cloud** infrastructure are three distinct levels. These sources do not make it possible to describe the commercial service's private fleet or orchestration. No benchmark or local launch was needed for this check of dependencies and calls; no new Exa search.
