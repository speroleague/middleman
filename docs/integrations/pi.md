# Pi integration

Middleman’s Pi extension automatically runs the local lifecycle after each user
task: it refreshes the incremental index, prepares one bounded Markdown packet,
starts a task, and injects the packet. The extension never starts a network
listener, proxies model traffic, stores task prompts in Middleman, or applies a
durable-memory proposal.

## Install

Build or install the `middleman` CLI so it is available on Pi's `PATH`, then
copy both extension files into the repository’s Pi extension directory:

```text
crates/adapters/pi/middleman.ts              -> .pi/extensions/middleman.ts
crates/adapters/pi/middleman-lifecycle.mjs   -> .pi/extensions/middleman-lifecycle.mjs
```

The project still needs ordinary Middleman initialization (`middleman init`). If
the CLI or state directory is unavailable, Pi receives a visible fallback and
continues without an injected packet; it does not attempt network recovery.

## Lifecycle and review

The injected packet is requested with a 1,200-token CLI budget. `middleman_expand`
expands only a selected reference and records that expansion as retrieval
feedback. A write/edit/apply-patch tool call marks work as material; after Pi
settles, the adapter finishes that Middleman task with Git observations.

When the agent has a concrete decision, invariant, or contract with evidence,
it calls `middleman_propose`. The adapter writes its structured payload to a
temporary file under `.middleman/cache`, invokes the existing proposal command,
and removes the file. The resulting proposal is pending review. Use the CLI to
inspect it and make the explicit decision:

```text
middleman review <proposal-id>
middleman apply <proposal-id>
# or: middleman reject <proposal-id> --reason "..."
```

Only the task ID and lifecycle flags are appended to Pi session state. Neither
the submitted prompt nor CLI/tool output is put into that custom state.
