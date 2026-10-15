# Every parameter is a setting, resolved through layers in which a lower layer can cap, lock or expose a value

At the owner's request, every parameter can be set. That covers retries, timeouts, batching, concurrency and quotas, guarantees, write modes, schema behaviour, failure handling, formats, payloads, Notifications and OAuth. Settings resolve through five layers, lowest first:
1. Built-in defaults: the best-practice values of the catalog decision.
2. An **Operator policy** on the machine or Daemon, which can set, cap or lock values.
3. User and project configuration.
4. A **Recipe** or the developer's code. Every setting the Recipe declares is locked for the Recipe's users unless the developer **exposes** it, optionally with bounds. What the Recipe does not declare, such as the user's own Sinks, Notification channels and credentials, stays the user's to set. A Recipe may expose `session.source` but never sets or locks it (ADR 0056).
   > Amended by [ADR 0092](0092-sessions-open-from-their-source-page-pause-waits-for-a-person-and-a-secret-is-a-placeholder-bound-to-domains.md): a Recipe declares the Secret placeholders it needs (`secrets.<name>`) and may expose them, but never binds or locks them, as with `session.source`, and the same holds for `session.http_credentials` and for `proxy.server` and the proxy's credentials, which send a Secret reference's value to a destination they name (a Recipe may still set pool kinds); a Client acting for a model sets none of them.
5. The run itself: SDK arguments, CLI flags, environment variables, an Integration's form.
   > Amended by [ADR 0083](0083-a-program-sets-settings-in-one-typed-option-explicit-values-beat-the-environment-and-no-value-is-silently-ignored.md): environment variables carry no setting, only a closed list of deployment variables that fill connection inputs, which an explicit value beats; layer 4's "developer's code" is a Recipe's code, and a program that is not a Recipe sets the run layer; a project file sent to an Exposed Daemon or Goodfetch Cloud keeps layer 3's rank with the run layer's authority.

A higher layer wins only where no lower layer locked the value. In a Daemon group, settings that only make sense on one machine, such as local addresses, TLS files and browser paths, take a *group member* scope applied only by that Group member (ADR 0055).

This extends ADR 0023, under which every costly control can be switched off and the results say so. It adds who may change a value, which a shared Daemon, a published Recipe and a workflow-tool form all need. Apify's input schema, Terraform's variable validation and Helm's values schema validate what users may pass; the lock across layers is Goodfetch's own addition. Decided in [Export connectors and integrations catalog](https://github.com/Jbmanllr/automan/issues/88).

## Considered Options

- **Settings without locks**, where the last writer wins: rejected. A Recipe's author could not stop its users from removing the delivery guarantee the Recipe depends on, and a team could not cap concurrency on a shared Daemon.
- **Fixed defaults with a few knobs**: rejected by the owner. Every parameter is a setting.

## Consequences

- **One generated schema.** A single settings schema, generated alongside the Operation catalog, produces the SDK types, the CLI flags, the Recipe manifest's exposed inputs, and the n8n, Zapier and Make forms.
- **`goodfetch config explain`** shows each effective value, the layer that set it and the layer that locked it, like `git config --show-origin`. The run report and the Console show the same; which layers the Console may edit is set by ADR 0031.
- **Weakening is recorded.** Any override that weakens a guarantee class or switches off a costly control is recorded in the Audit log and marked on the results it touches (ADR 0023).
- **Some honesty guarantees stay locked whatever the layer**: the delivery ID and epoch on every write, the guarantee class reported per delivery, webhook signing, and Secret references. These join ADR 0016's controls that cost nothing at run time.
- **Changes ADRs 0016 and 0023.** Their "switchable per run" controls can now be capped or locked by an Operator policy or a Recipe; where nothing locks them, the run can still switch them.
- **Per product major.** The Operator policy and the user-level configuration are kept per product major (project configuration lives with the project), and a new major refuses to start on an older major's Operator policy until `goodfetch upgrade-files` has run (ADR 0030).
- **Limits across runs are not settings.** The install, Daemon, person, named token and Client connection Limits live in the Usage ledger, which every major shares; an Operator policy can only cap them for its own major (ADR 0041).
- **Applies beyond Sinks.** The layering covers every engine setting, not only Sinks. Earlier ADRs that call a control "settable" or "switchable" mean settable through these layers.
- Some settings can only be narrowed by the Recipe and run layers. `trusted_cas`, certificate pins and `any` are set only in Operator policy or user and project configuration (ADR 0042). `pay_domains` and `max_price_per_page` are widened only there or, for `pay_domains`, by a person's answer, and the exchange rates and chain RPC endpoints of Paid fetches are set only there (ADR 0045). `consent_domains` is widened only there or by a person's answer (ADR 0048), and so is `wall_domains`, while `wall_refetch_domains` is widened only there, never by an answer, and `wall_denied_domains` is set only in Operator policy, and no lower layer changes it (ADR 0052). Scriptlets of a Filter list from `filter_lists.extra` run only when it is set there (ADR 0047). On a Daemon with accounts, whose user configuration every member's runs share, all of these, scriptlet-carrying Filter lists included, are written only by the operator role, in Operator policy.
