# Executables beside a remote browser, per-run images and version skew: research of October 15, 2026

Fact-finding for [issue #498](https://github.com/goodfetch/automan/issues/498) (which executables the Browser host and Goodfetch Cloud's engines run). It records what comparable projects ship and how connected components handle version differences. It makes no decision. Repository files are cited at a named tag or commit; documentation pages were read on October 15, 2026. "Unverified" marks what no primary source confirmed.

## A. Where the "host beside the browser" role lives

**1. Selenium Grid 4 (Selenium 4.51.0, released 2026-10-09; docker-selenium `4.51.0-20261010`, released 2026-10-14).**

- *One artifact, roles as subcommands.* The release ships one server artifact, `selenium-server-4.51.0.jar` ([release assets](https://github.com/SeleniumHQ/selenium/releases/tag/selenium-4.51.0)). The [Grid getting-started page](https://www.selenium.dev/documentation/grid/getting_started/) starts every role from that jar: `java -jar selenium-server-<version>.jar standalone`, `hub` or `node`. The fully distributed mode adds `event-bus`, `sessions`, `sessionqueue`, `distributor` and `router`. Verified.
- *Images per role and per browser.* docker-selenium publishes one image per Grid role: `selenium/hub`, `event-bus`, `sessions`, `session-queue`, `distributor` and `router` ([full-grid compose file](https://github.com/SeleniumHQ/docker-selenium/blob/4.51.0-20261010/docker-compose-v3-full-grid.yml)). It also publishes one Node image and one Standalone image per browser:
  - `node-chrome`, `node-chromium`, `node-edge`, `node-firefox` and `node-chrome-for-testing`, with matching `standalone-*` images.
  - `node-all-browsers` and `standalone-all-browsers`, available since 4.35.0.
  - `node-docker` and `standalone-docker`, for the dynamic Grid.
  - `selenium/video`.

  All of these come from the [README at the tag](https://github.com/SeleniumHQ/docker-selenium/blob/4.51.0-20261010/README.md). Every image builds on one `Base` image, which downloads `selenium-server-${VERSION}.jar` to `/opt/selenium/selenium-server.jar` ([Base/Dockerfile](https://github.com/SeleniumHQ/docker-selenium/blob/4.51.0-20261010/Base/Dockerfile), lines 131–132). Each role's start script runs `java -jar /opt/selenium/selenium-server.jar <role>`. The [distributor](https://github.com/SeleniumHQ/docker-selenium/blob/4.51.0-20261010/Distributor/start-selenium-grid-distributor.sh), [router](https://github.com/SeleniumHQ/docker-selenium/blob/4.51.0-20261010/Router/start-selenium-grid-router.sh) and [node](https://github.com/SeleniumHQ/docker-selenium/blob/4.51.0-20261010/NodeBase/start-selenium-node.sh) scripts were checked. So the role images and the browser images all package one jar. Verified.
- *Mixing versions.* No official Selenium statement was found on whether Grid components, or a Hub and its Nodes, may run different versions. The [Components page](https://www.selenium.dev/documentation/grid/components/) and the getting-started page say nothing on this, and a search of SeleniumHQ issues found nothing. **Unverified.** The packaging pins everything to one version:
  - The README says to always use a full tag to pin "a specific browser and Grid version".
  - The [Helm chart values](https://github.com/SeleniumHQ/docker-selenium/blob/4.51.0-20261010/charts/selenium-grid/values.yaml) set one `global.seleniumGrid.imageTag`, described as the image tag "for all selenium components". A separate `nodesImageTag` has the same default, and each component can override the tag.
  - The README and chart examples at this tag still show `4.48.0-20260905`. The documentation lags the release.

**2. Playwright (v1.64.0, 2026-10-07; v1.64.1, 2026-10-14).**

- *Same package.* `run-server` and `launch-server` are hidden commands of the CLI in `playwright-core` ([program.ts at v1.64.1](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/cli/program.ts), lines 178 and 199). `browserType.launchServer()` belongs to the client library's own API. Verified.
- *Version rule.* The [BrowserType reference](https://playwright.dev/docs/api/class-browsertype) says the major and minor versions of the connecting Playwright must match the server's. Its example: "1.2.3 → is compatible with 1.2.x". `launchServer` repeats the rule for `connect()`. Verified.
- *Enforcement in code.* The server reads `Playwright/x.y.z` from the client's User-Agent and compares major.minor. On a mismatch it refuses the WebSocket upgrade with `HTTP 428 Precondition Required` and a "Playwright version mismatch" message that shows both versions. If it cannot parse the User-Agent, it is "lax" and lets the client in ([playwrightServer.ts at v1.64.1](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/remote/playwrightServer.ts), lines 81–84 and 331–353). Verified.
- *Docker.* The [Docker page](https://playwright.dev/docs/docker) runs the remote server in the official image, `mcr.microsoft.com/playwright:v1.64.0-noble`, with `npx -y playwright@1.64.0 run-server --port 3000 --host 0.0.0.0`. Tests then connect with `PW_TEST_CONNECT_WS_ENDPOINT` or `browserType.connect()`. The image holds the browsers and their system dependencies, but "The Playwright package/dependency is not included". The page asks you to make sure the Playwright version in the tests matches the container's. Verified.

**3. Browserless (v2.57.0, 2026-09-30).**

- *Images per browser, one codebase.* GitHub workflows publish `ghcr.io/browserless/chromium`, `chrome`, `edge`, `firefox`, `webkit` and `multi`. Each builds on `ghcr.io/browserless/base` ([workflows](https://github.com/browserless/browserless/tree/v2.57.0/.github/workflows)). Each browser's Dockerfile compiles the same `src/` on that base and runs `./scripts/start.sh` ([chromium](https://github.com/browserless/browserless/blob/v2.57.0/docker/chromium/Dockerfile), [firefox](https://github.com/browserless/browserless/blob/v2.57.0/docker/firefox/Dockerfile)). The Chromium image installs its browser with the bundled `playwright-core` CLI. The [README](https://github.com/browserless/browserless/blob/v2.57.0/README.md) points Firefox and WebKit users to the `firefox` or `multi` image. So there is one server and one image per browser. Verified. The `docker/sdk` folder was not inspected.
- *Client skew.* The server bundles five Playwright server minors, 1.59 to 1.63, as npm aliases, mapped in `playwrightVersions` ([package.json](https://github.com/browserless/browserless/blob/v2.57.0/package.json), lines 71–75 and 116–122). It reads `Playwright/x.y` from the client's User-Agent ([src/browsers/index.ts](https://github.com/browserless/browserless/blob/v2.57.0/src/browsers/index.ts), line 805) and loads the matching version. It falls back to the default when the version is unknown ([src/config.ts](https://github.com/browserless/browserless/blob/v2.57.0/src/config.ts), line 595: `versions[version] || versions['default']`). Verified.

**4. Steel (`steel-dev/steel-browser` v0.5.4-beta, 2026-08-25).** On every push to `main`, the [build-docker workflow](https://github.com/steel-dev/steel-browser/blob/v0.5.4-beta/.github/workflows/build-docker.yml) publishes three multi-arch images tagged `latest`:

- `ghcr.io/steel-dev/steel-browser`, built from the root [Dockerfile](https://github.com/steel-dev/steel-browser/blob/v0.5.4-beta/Dockerfile): the Node API, the built UI, nginx and Chromium in one container, exposing ports 3000 and 9223.
- `ghcr.io/steel-dev/steel-browser-api`, built from [api/Dockerfile](https://github.com/steel-dev/steel-browser/blob/v0.5.4-beta/api/Dockerfile): the API server with Chromium.
- `ghcr.io/steel-dev/steel-browser-ui`, built from [ui/Dockerfile](https://github.com/steel-dev/steel-browser/blob/v0.5.4-beta/ui/Dockerfile): `nginx:alpine` serving the UI build.

The [compose file](https://github.com/steel-dev/steel-browser/blob/v0.5.4-beta/docker-compose.yml) uses the api and ui images, and the README's `docker run` uses the combined image. Verified. No version-tagged images were found in the release workflow. **Unverified** whether any exist.

**5. Kernel (`kernel/kernel-images`, `main` at [`ef93a92`](https://github.com/kernel/kernel-images/tree/ef93a92460ae7031cbdb9df913ff23c4655bc50c), 2026-10-14; no releases).**

- *Images.* The repository has two image directories, `images/chromium-headful` and `images/chromium-headless`. Each builds as a Docker image or as a Unikraft unikernel ([README](https://github.com/kernel/kernel-images/blob/ef93a92460ae7031cbdb9df913ff23c4655bc50c/README.md)). CI pushes `onkernel/chromium-headful:<short_sha>` and `onkernel/chromium-headless:<short_sha>` to Docker Hub ([headful workflow](https://github.com/kernel/kernel-images/blob/ef93a92460ae7031cbdb9df913ff23c4655bc50c/.github/workflows/chromium-headful-image.yaml), line 66).
- *Server beside the browser.* Both Dockerfiles compile the same Go module, `server/`, into `kernel-images-api`, `chromium-launcher`, `wrapper` and `kernel-images-supervisord-shim` ([headful Dockerfile](https://github.com/kernel/kernel-images/blob/ef93a92460ae7031cbdb9df913ff23c4655bc50c/images/chromium-headful/Dockerfile), lines 1–40). The API's [OpenAPI file](https://github.com/kernel/kernel-images/blob/ef93a92460ae7031cbdb9df913ff23c4655bc50c/server/openapi.yaml) covers recording, process exec, filesystem, mouse and keyboard (`/computer/*`), Chromium extensions, policies and flags, `/playwright/execute`, logs and telemetry. CDP is exposed on port 9222 through `ncat`. The headful image adds a WebRTC (neko) or noVNC live view. Verified.

**6. Lightpanda (1.0.0, 2026-10-02).** One `lightpanda` binary. [src/Config.zig at 1.0.0](https://github.com/lightpanda-io/browser/blob/1.0.0/src/Config.zig) (lines 396 onward) defines its commands: `serve`, `fetch`, `mcp`, `agent` and `run`.

- `serve` is the CDP server, on `127.0.0.1:9222` by default. Its `--protocol` option can be repeated, and `webdriver` is one of the protocols.
- `fetch` is the one-shot run.

The [README](https://github.com/lightpanda-io/browser/blob/1.0.0/README.md) runs both from the same binary. The `lightpanda/browser` Docker image starts the CDP server on port 9222. Verified.

**What this suggests (A).** Most projects build the process that runs beside the browser from the product's main codebase:

- Selenium starts it with a role subcommand of one jar.
- Playwright starts it with a subcommand of the client's own package.
- Lightpanda starts it with a subcommand of its one binary.

Per-browser or per-role images are packaging variants of that one artifact (Selenium, Browserless, Kernel's headful and headless images). Kernel and Steel are different: they ship a dedicated server beside the browser, distinct from the SDK the user codes against. On versions:

- Selenium and Playwright expect the same version, or the same major.minor, across the link. Playwright enforces it at connect time.
- Browserless instead ships several server versions and picks the one that matches the connecting client.

## B. Per-run images on hosted platforms

**7. Apify.**

- *Builds and runs.* "A build is a Docker image containing your source code and the required dependencies." A run takes a build and starts it with input ([builds and runs](https://docs.apify.com/platform/actors/development/builds-and-runs)). Builds are numbered MAJOR.MINOR.BUILD, and tags such as `latest` or `beta` select one ([builds](https://docs.apify.com/platform/actors/development/builds-and-runs/builds)).
- *Where the Dockerfile comes from.* The `dockerfile` field of `.actor/actor.json` points to it; otherwise the build uses `.actor/Dockerfile`, then the root `Dockerfile`. Without one, a default Dockerfile starting from `apify/actor-node:24` is used ([Dockerfile](https://docs.apify.com/platform/actors/development/actor-definition/dockerfile)).
- *Official base images.* Node.js: `apify/actor-node`, `actor-node-puppeteer-chrome`, `actor-node-playwright-chrome`, `actor-node-playwright-firefox`, `actor-node-playwright-webkit`, `actor-node-playwright` and `actor-node-playwright-camoufox`. Python: `apify/actor-python`, `actor-python-playwright`, `actor-python-playwright-camoufox` and `actor-python-selenium`. Tags take the form `{node-version}-{library-version}`, with `-slim` variants.

So each run uses its own Actor build's image, typically built on a base image for its stack. Verified. That the run's container starts from exactly that image follows from "a build is a Docker image"; the pages read here do not spell it out.

**8. Browserbase Functions (launched 2026-02-10, [changelog](https://browserbase.com/changelog/browserbase-functions)).** User code is TypeScript that runs "next to a Browserbase browser", with under 5 ms of latency to it, only in `us-west-2` ([runtime overview](https://docs.browserbase.com/platform/runtime/overview.md)). Each invocation receives a session it connects to with `connectOverCDP`, and the session closes when the handler returns. Invocations time out after 60 to 900 s, and the input is at most 64 KB of JSON ([reference](https://docs.browserbase.com/platform/runtime/reference.md)). The underlying runtime is not documented: container, VM or isolate, image, and Node.js version. **Unverified.**

**What this suggests (B).** Apify, the documented case, gives each workload its own image: the Actor's build, built on base images per language and browser stack. It does not run one universal image for every run. Browserbase places user code next to the browser but does not say how.

## C. Version skew for a component that connects out to a service

**9. Kubernetes ([version skew policy](https://kubernetes.io/releases/version-skew-policy/), current docs; maintained releases 1.37, 1.36 and 1.35).**

- `kubelet` "must not be newer than `kube-apiserver`", and may be up to three minor versions older (two before 1.25). With kube-apiserver 1.37, kubelet 1.34 to 1.37 is supported.
- `kubectl` may be one minor version older or newer than the API server.
- The API server is upgraded first. Upgrading the kubelet is optional, but in-place minor kubelet upgrades are not supported.

Verified.

**10. Docker Engine API ([API reference](https://docs.docker.com/reference/api/engine/); Docker 29.9.1 released 2026-10-14).**

- *Negotiation.* Server and client "support API-version negotiation". A client connecting to an older Engine "negotiates the highest version of the API supported by both". In effect it drops to the lower of the two maxima. `DOCKER_API_VERSION` pins a version and disables negotiation. Verified.
- *Daemon window.* The daemon accepts versions from a minimum to a maximum and rejects requests outside that range. The messages read "client version … is too old. Minimum supported API version is …" or "… is too new" ([version.go at docker-v29.9.1](https://github.com/moby/moby/blob/docker-v29.9.1/daemon/server/middleware/version.go), lines 57–61). Verified.
- *The 29.0 change and its rollback.* [29.0.0 (2025-11-10)](https://docs.docker.com/engine/release-notes/29/) made the daemon require API v1.44 or later (Docker 25.0+). The client also dropped negotiation below v1.44 ([moby#51186](https://github.com/moby/moby/pull/51186), merged 2025-10-16). `DOCKER_MIN_API_VERSION` can override that minimum. 29.3.0 (2026-03-05) lowered the default minimum back to v1.40 (Docker 19.03). The reference's matrix gives a 1.44 minimum for 29.0–29.2 and 1.40 for 29.3–29.8. [config.go at docker-v29.9.1](https://github.com/moby/moby/blob/docker-v29.9.1/daemon/config/config.go) (lines 65–71) sets `defaultMinAPIVersion = "1.40"`, with an override floor of `1.24`. Verified.

**11. GitHub Actions self-hosted runners.**

- *Auto-update and the 30-day rule.* Runners update themselves by default, when a job is assigned to them. The `--disableupdate` flag turns this off. With updates off, the runner must still be updated "within 30 days" of any new release, or "the GitHub Actions service will not queue jobs to your runner". A required critical security update stops queuing until it is applied ([self-hosted runners reference](https://docs.github.com/en/actions/reference/runners/self-hosted-runners)).
- *Enforcement.* GitHub announced enforcement for github.com on [2026-06-12](https://github.blog/changelog/2026-06-12-github-actions-minimum-version-enforcement-timeline-for-self-hosted-runners/):
  - Registering or re-registering a runner needs version 2.329.0 or later.
  - Each release must be installed within 30 days of its publication.
  - GitHub Enterprise Server is not affected.
- *Dates.* Full enforcement began July 31, 2026 for GitHub Enterprise Cloud with Data Residency. For GitHub Enterprise Cloud it [moved to September 29, 2026](https://github.blog/changelog/2026-09-28-self-hosted-runner-version-enforcement-date-has-moved/), with the requirements unchanged.

Verified.

**12. Local tunnel binaries.**

- *Sauce Connect.* The [Sauce Connect 4 lifecycle page](https://docs.saucelabs.com/secure-connections/sauce-connect-4/lifecycle/) gives an end of support of January 31, 2026 and an end of life of July 31, 2026. After that date Sauce Labs no longer accepts Sauce Connect 4 traffic. Versions 4.8 and below are no longer supported. An earlier [changelog post (2021-07-31)](https://changelog.saucelabs.com/en/sauce-connect-reminder-end-of-support-dates) shows the same pattern: versions 4.5.4 and below "stopped working". No support window per version and no auto-update were found for Sauce Connect 5: the [Sauce Connect 5 overview](https://docs.saucelabs.com/secure-connections/sauce-connect-5/) and the [`sc run` reference](https://docs.saucelabs.com/dev/cli/sauce-connect-5/run/) say nothing on either. **Unverified.**
- *BrowserStack Local.* The [support FAQ](https://www.browserstack.com/support/faq/local-testing/general/is-my-local-binary-version-supported) supports "the three most recent versions of the binary". The binary checks for the newest version and "self-updates on every restart". The versions the FAQ lists, v7.4 to v7.6 with a 30 November 2019 cutoff, are stale. The [release notes](https://www.browserstack.com/local-testing/releases) list v8.9, released 16 March 2026 (patch 8.9.25). The policy is documented, but whether it is still enforced today is **unverified**.

**13. Negotiating down versus refusing.**

- *Negotiates down.* Only Docker negotiates down to the version both sides support (the lower maximum), within the daemon's accepted window.
- *Serves several versions.* Browserless serves several Playwright server versions at once and picks the client's version.
- *Fixed skew window.* Kubernetes allows only the connecting side to be older, never newer, by a fixed number of minors.
- *Refuses.*
  - Playwright refuses a major.minor mismatch at connect, with a 428 response.
  - The Docker daemon refuses versions outside its window.
  - GitHub stops queuing jobs to runners more than 30 days behind and refuses registration below 2.329.0.
  - Sauce Labs stops accepting traffic from a major version past its end of life.
- *Self-updates.* GitHub runners update when a job is assigned. BrowserStack Local updates on restart.
- *No documented rule.* Selenium Grid documents none.

**What this suggests (C).** The examples combine three mechanisms:

- a compatibility window that bounds how far the outbound side may lag, where the outbound side may be older but not newer (Kubernetes);
- negotiation down to a common version inside that window (Docker);
- forced currency for clients users run themselves: auto-update plus a deadline after which the service refuses work (GitHub's 30 days, Sauce Labs' end of life, BrowserStack's three versions).

Projects whose server and client must match exactly either refuse at connect (Playwright) or carry several server versions to absorb client skew (Browserless).

## Sources

- Selenium: [Grid getting started](https://www.selenium.dev/documentation/grid/getting_started/); [Grid components](https://www.selenium.dev/documentation/grid/components/); [release selenium-4.51.0](https://github.com/SeleniumHQ/selenium/releases/tag/selenium-4.51.0); [docker-selenium README at 4.51.0-20261010](https://github.com/SeleniumHQ/docker-selenium/blob/4.51.0-20261010/README.md); [Base/Dockerfile](https://github.com/SeleniumHQ/docker-selenium/blob/4.51.0-20261010/Base/Dockerfile); [Helm values](https://github.com/SeleniumHQ/docker-selenium/blob/4.51.0-20261010/charts/selenium-grid/values.yaml).
- Playwright: [BrowserType reference](https://playwright.dev/docs/api/class-browsertype); [Docker](https://playwright.dev/docs/docker); [program.ts at v1.64.1](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/cli/program.ts); [playwrightServer.ts at v1.64.1](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/remote/playwrightServer.ts).
- Browserless at [v2.57.0](https://github.com/browserless/browserless/tree/v2.57.0): `docker/*/Dockerfile`, `.github/workflows/docker-*.yml`, `package.json`, `src/browsers/index.ts`, `src/config.ts`.
- Steel at [v0.5.4-beta](https://github.com/steel-dev/steel-browser/tree/v0.5.4-beta): `Dockerfile`, `api/Dockerfile`, `ui/Dockerfile`, `docker-compose.yml`, `.github/workflows/build-docker.yml`.
- Kernel at [`ef93a92`](https://github.com/kernel/kernel-images/tree/ef93a92460ae7031cbdb9df913ff23c4655bc50c): `README.md`, `images/chromium-*/Dockerfile`, `server/openapi.yaml`, `.github/workflows/chromium-*-image.yaml`.
- Lightpanda at [1.0.0](https://github.com/lightpanda-io/browser/tree/1.0.0): `README.md`, `src/Config.zig`.
- Apify: [builds and runs](https://docs.apify.com/platform/actors/development/builds-and-runs); [builds](https://docs.apify.com/platform/actors/development/builds-and-runs/builds); [Dockerfile](https://docs.apify.com/platform/actors/development/actor-definition/dockerfile).
- Browserbase: [Functions changelog](https://browserbase.com/changelog/browserbase-functions); [runtime overview](https://docs.browserbase.com/platform/runtime/overview.md); [runtime reference](https://docs.browserbase.com/platform/runtime/reference.md).
- Kubernetes: [version skew policy](https://kubernetes.io/releases/version-skew-policy/).
- Docker: [Engine API reference](https://docs.docker.com/reference/api/engine/); [Engine 29 release notes](https://docs.docker.com/engine/release-notes/29/); [moby#51186](https://github.com/moby/moby/pull/51186); [version.go](https://github.com/moby/moby/blob/docker-v29.9.1/daemon/server/middleware/version.go) and [config.go](https://github.com/moby/moby/blob/docker-v29.9.1/daemon/config/config.go) at docker-v29.9.1.
- GitHub: [self-hosted runners reference](https://docs.github.com/en/actions/reference/runners/self-hosted-runners); [enforcement timeline, 2026-06-12](https://github.blog/changelog/2026-06-12-github-actions-minimum-version-enforcement-timeline-for-self-hosted-runners/); [date moved, 2026-09-28](https://github.blog/changelog/2026-09-28-self-hosted-runner-version-enforcement-date-has-moved/).
- Sauce Labs: [Sauce Connect 4 lifecycle](https://docs.saucelabs.com/secure-connections/sauce-connect-4/lifecycle/); [2021 end-of-support post](https://changelog.saucelabs.com/en/sauce-connect-reminder-end-of-support-dates); [Sauce Connect 5 overview](https://docs.saucelabs.com/secure-connections/sauce-connect-5/); [`sc run` reference](https://docs.saucelabs.com/dev/cli/sauce-connect-5/run/).
- BrowserStack: [binary support FAQ](https://www.browserstack.com/support/faq/local-testing/general/is-my-local-binary-version-supported); [Local binary release notes](https://www.browserstack.com/local-testing/releases).
