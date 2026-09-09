# Task: Phase 4 Kilo and Cline integrations

## Objective

Make the existing local MCP contract usable from Kilo and Cline without a
re-index or any persisted chat content. Add Cline lifecycle automation where
its documented hooks support it, and a Kilo project custom mode where Kilo
offers MCP and custom modes but no documented lifecycle plugin API.

## Scope

- Files / modules: `crates/adapters/{kilo,cline}/`, `docs/integrations/`,
  `docs/benchmarks/`
- Explicitly out of scope: network transport, model proxying, automatic
  proposal acceptance, any undocumented Kilo extension API, and claiming model
  quality results without a human-run matched task.

## Contract and operational impact

- Callers / consumers affected: users of the Kilo Code and Cline harnesses.
- Data ownership or schema impact: Middleman continues to own the index,
  event log, task, and proposal state. Cline hook state is source-free and
  temporary under the already-ignored `.middleman/cache` directory.
- Compatibility / migration path: both integrations use `middleman-mcp` and
  can be removed without altering the event log; the universal CLI remains the
  fallback.
- Rollback or recovery path: remove the installed mode or hook files and MCP
  configuration. No generated durable-memory proposal is applied automatically.

## Functional design

```mermaid
sequenceDiagram
    participant H as Kilo or Cline
    participant A as mode or hook adapter
    participant M as Middleman CLI/MCP
    H->>A: task submission
    A->>M: index --changed; prepare; task start
    M-->>H: bounded context packet
    H->>A: material edit
    A->>M: task finish --from-git
    H->>M: middleman_propose
    M-->>H: pending reviewable proposal
```

- Pure rule / transformation / state transition: the lifecycle coordinator maps
  command outcomes to packet, unavailable, idle, and proposal-required states.
- Effect boundary or adapter: hook scripts invoke only local `middleman` and
  persist a task ID plus lifecycle flags in the ignored cache directory.
- Intentional mutation or non-determinism, if any: CLI events and ephemeral
  source-free Cline state files are created; state files are removed on task
  completion or cancellation.

## Decisions

- Kilo custom modes are the documented native configuration mechanism. Its
  current docs expose MCP configuration but no lifecycle-hook/plugin API, so the
  mode guides its MCP/CLI lifecycle instead of pretending background automation
  exists.
- Cline's `UserPromptSubmit`, `PreToolUse`, `TaskComplete`, and `TaskCancel`
  hooks provide the documented automatic lifecycle boundary.
- Benchmark evidence records rendered initial input and explicit completion and
  validation scores; cache work counts are not represented as token savings.

## Changes made

- Added a shared, source-free lifecycle coordinator and retained Pi's installed
  module as a compatibility re-export.
- Added Kilo's documented project custom mode and MCP setup instructions.
- Added Cline Node hook scripts for `UserPromptSubmit`, `PreToolUse`,
  `TaskComplete`, and `TaskCancel`, with a Windows MCP/CLI fallback.
- Added installation guidance and a matched-harness benchmark worksheet that
  measures rendered initial input and quality rather than cache work counts.

## Validation

- Command: `node --test crates/adapters/pi/middleman-lifecycle.test.mjs crates/adapters/cline/middleman-hook.test.mjs crates/adapters/kilo/middleman-mode.test.mjs`
- Result: passed (10 tests).
- Command: `cargo test --workspace --offline`
- Result: passed (the Windows environment emitted its existing home-path
  canonicalization warning).
- Command: `cargo clippy --workspace --all-targets --offline -- -D warnings`
- Result: passed (same environment warning only).
- Command: `cargo fmt --all --check`
- Result: passed (same environment warning only).
- Baseline / performance evidence: the reproducible matched-task protocol is
  in `docs/benchmarks/phase-4.md`. No live Kilo/Cline model run is recorded in
  this workspace, so it makes no completed-quality or savings claim.

## Next step / handoff

Validate installation in real Kilo and Cline sessions, then record one matched
task per harness using the benchmark worksheet.
