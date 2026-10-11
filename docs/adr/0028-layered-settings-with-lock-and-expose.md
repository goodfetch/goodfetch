# Every parameter is a setting, resolved through layers in which a lower layer can cap, lock or expose a value

At the owner's request, every parameter can be set. That covers retries, timeouts, batching, concurrency and quotas, guarantees, write modes, schema behaviour, failure handling, formats, payloads, Notifications and OAuth. Settings resolve through five layers, lowest first:
1. Built-in defaults: the best-practice values of the catalog decision.
2. An **Operator policy** on the machine or Daemon, which can set, cap or lock values.
3. User and project configuration.
4. A **Recipe** or the developer's code. Every setting the Recipe declares is locked for the Recipe's users unless the developer **exposes** it, optionally with bounds. What the Recipe does not declare, such as the user's own Sinks, Notification channels and credentials, stays the user's to set.
5. The run itself: SDK arguments, CLI flags, environment variables, an Integration's form.

A higher layer wins only where no lower layer locked the value.

This extends ADR 0023, under which every costly control can be switched off and the results say so. It adds who may change a value, which a shared Daemon, a published Recipe and a workflow-tool form all need. Apify's input schema, Terraform's variable validation and Helm's values schema validate what users may pass; the lock across layers is Automan's own addition. Decided in [Export connectors and integrations catalog](https://github.com/Jbmanllr/automan/issues/88).

## Considered Options

- **Settings without locks**, where the last writer wins: rejected. A Recipe's author could not stop its users from removing the delivery guarantee the Recipe depends on, and a team could not cap concurrency on a shared Daemon.
- **Fixed defaults with a few knobs**: rejected by the owner. Every parameter is a setting.

## Consequences

- **One generated schema.** A single settings schema, generated alongside the Operation catalog, produces the SDK types, the CLI flags, the Recipe manifest's exposed inputs, and the n8n, Zapier and Make forms.
- **`automan config explain`** shows each effective value, the layer that set it and the layer that locked it, like `git config --show-origin`. The run report and the Console show the same.
- **Weakening is recorded.** Any override that weakens a guarantee class or switches off a costly control is recorded in the Audit log and marked on the results it touches (ADR 0023).
- **Some honesty guarantees stay locked whatever the layer**: the delivery ID and epoch on every write, the guarantee class reported per delivery, webhook signing, and Secret references. These join ADR 0016's controls that cost nothing at run time.
- **Changes ADRs 0016 and 0023.** Their "switchable per run" controls can now be capped or locked by an Operator policy or a Recipe; where nothing locks them, the run can still switch them.
- **Per product major.** The Operator policy and user and project configuration are kept per product major, and a new major refuses to start on an older major's Operator policy until `automan upgrade-files` has run (ADR 0030).
- **Applies beyond Sinks.** The layering covers every engine setting, not only Sinks. Earlier ADRs that call a control "settable" or "switchable" mean settable through these layers.
