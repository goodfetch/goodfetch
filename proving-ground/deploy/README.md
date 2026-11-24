# The online copy's deployment

The code that deploys the Proving ground's online copy (#168; ADRs 0025, 0032
and 0076) to `challenges.goodfetch.dev`. It is public so anyone can read what
the copy runs and how it is set up.

## What deploys, and when

The `proving ground online` workflow (`.github/workflows/proving-ground-online.yml`)
starts on the push of a release tag (`vX.Y.Z` or `vX.Y.Z-rc.N`). Its one job
runs under the protected `proving-ground-online` environment, so it waits for
the owner's approval and reads the keys from that environment's secrets. It
builds the Proving ground at the tag and runs `deploy.sh`, which:

- ships the copy, `online.toml`, cloudflared and, with an `[anubis]` section,
  Anubis (each pinned and checked by SHA-256), their systemd units and the
  secrets `online.toml` names to the host over SSH, then restarts the services
  and checks the copy answers;
- with a `[botid]` section, deploys `botid/` to Vercel.

On the host:

- `goodfetch-proving-ground.service` serves the copy on `127.0.0.1:8920`.
- `goodfetch-anubis.service` serves Anubis on `127.0.0.1:8923`, in front of
  the copy's `/anubis`.
- `goodfetch-cloudflared.service` runs the Cloudflare Tunnel that brings
  `challenges.goodfetch.dev` to them (`cloudflared.yml`). The host opens no
  inbound port but SSH.

Every secret goes to a file of mode 600 under `/etc/goodfetch-proving-ground`
and never to the log. A secret may hold only `A-Z a-z 0-9 . _ ~ + / = -`.

`online.toml` turns on only the vendors that need no vendor account: ALTCHA,
Anubis and Klaro.

## The owner's one-time setup

1. **The host.** Any Ubuntu 24.04 (or later) machine with systemd, `curl` and
   root SSH by key. Put its `known_hosts` line and a deploy key in the
   secrets below.
2. **The tunnel.** On a machine logged in to the `goodfetch.dev` zone's
   Cloudflare account (free plan), run
   `cloudflared tunnel create proving-ground` and
   `cloudflared tunnel route dns proving-ground challenges.goodfetch.dev`.
   The credentials file that the first command writes is the
   `PROVING_GROUND_TUNNEL_CREDENTIALS` secret.
3. **The environment.** In the repository's settings, create the environment
   `proving-ground-online`. Make the owner its required reviewer, and allow
   only the tags `v*` to deploy. Add the secrets:

   | Secret | Value |
   |---|---|
   | `PROVING_GROUND_DEPLOY_HOST` | `root@<host>` |
   | `PROVING_GROUND_DEPLOY_SSH_KEY` | the deploy key's private half |
   | `PROVING_GROUND_DEPLOY_KNOWN_HOSTS` | the host's `known_hosts` line |
   | `PROVING_GROUND_TUNNEL_CREDENTIALS` | the tunnel's credentials file |
   | `PROVING_GROUND_ALTCHA_KEY` | `openssl rand -hex 32` |
   | `PROVING_GROUND_ANUBIS_HS512_SECRET` | `openssl rand -hex 32`; Anubis refuses a key shorter than 64 characters |

This file is the text that records these settings (`AGENTS.md`, Merging): a
change to them amends it in the same pull request.

## Turning on a vendor that needs an account

Each vendor below needs an account on its free tier. Add its section to
`online.toml` (the sections and their fields are in `src/online.rs`), and its
secret to the environment, under the name its `secret_env` gives:

- **Turnstile** (Cloudflare), **reCAPTCHA** (Google) and **hCaptcha**: a site
  for `challenges.goodfetch.dev`, then its `site_key` and the secret
  (`PROVING_GROUND_TURNSTILE_SECRET`, `PROVING_GROUND_RECAPTCHA_SECRET` or
  `PROVING_GROUND_HCAPTCHA_SECRET`).
- **OneTrust**, **Cookiebot** and **CookieYes**: a banner for the domain, then
  its `domain_script`, `cbid` or `client_id`. They need no secret.
- **Vercel BotID**: a Vercel project on the Hobby plan, on
  `botid.goodfetch.dev`, with BotID's basic mode. Add `[edge]` (secret
  `PROVING_GROUND_EDGE_SECRET`, `openssl rand -hex 32`) and `[botid]` with
  `origin = "https://botid.goodfetch.dev"`, and the secrets `VERCEL_TOKEN`,
  `VERCEL_ORG_ID` and `VERCEL_PROJECT_ID`. `deploy.sh` then deploys `botid/`,
  whose function asks BotID about the request and forwards its verdict to the
  copy's `/botid/verify`.
- **Cap**: the copy can judge it, but this code does not deploy Cap's
  standalone server yet, so it stays off.

## Declined: paid options

Goodfetch spends nothing (ADR 0076), so the vendors that only a paid plan
gives stay off:

- Cloudflare's bot score, which needs Bot Management (Enterprise).
- AWS WAF Bot Control, billed per request.
- BotID's Deep Analysis, which needs Vercel Pro. The basic mode above is free.
