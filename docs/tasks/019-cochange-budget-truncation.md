# Task: Bounded co-change history for repositories with wide merges

## Objective

Let repositories whose main line merges large pull requests be indexed. Today
one wide commit exceeds the co-change budget and fails the whole index. Wide
commits should be left out of co-change evidence, and collection should stop
at the budget, with both reported through `history_truncated`.

## Scope

- Files / modules: `crates/indexer/src/graph.rs` (`Budget`, `cochanges`) and
  `crates/indexer/tests/graph_refresh.rs`.
- Explicitly out of scope:
  - co-change scoring;
  - the 64-commit history window;
  - making the budget configurable in `middleman.toml`.

## Contract and operational impact

- Callers / consumers affected: `index` and `prepare` now succeed where they
  failed with `graph exceeds its cochanges budget`. The packet and index output
  report `history_truncated: true` when evidence was left out.
- Data ownership or schema impact: none. `Graph` and `CoChange` are unchanged.
  `Budget` gains `max_commit_paths`.
- Compatibility / migration path:
  - Repositories that indexed before keep the same co-change evidence, unless
    a recent commit touches more than 50 indexed files.
  - The next `index --full` builds the new evidence.
- Rollback or recovery path: revert the commit. Existing state and event logs
  remain valid.

## Functional design

- Pure rule / transformation / state transition: commits are processed in
  history order, newest first.
  - A commit whose indexed paths exceed `max_commit_paths` (default 50) is
    skipped.
  - When the next commit would push pair observations past `max_cochanges`
    (default 10,000), collection stops.
  - Either case marks the history truncated.
- Effect boundary or adapter: unchanged. Git history is still read by
  `git::observe` and bounded by `max_commits`.
- Intentional mutation or non-determinism, if any: none.

## Decisions

- Truncate explicitly instead of failing. Task 002 rules out silently
  truncating the graph. This truncation is reported through the existing
  `history_truncated` flag, so consumers still see that the evidence is
  partial. Failing the whole index made Middleman unusable on Monarch HUB and
  Portal: a single merge there touches 130 to 252 files, which is up to about
  31,600 pairs, more than three times the budget.
- Skip wide commits rather than sample them. A commit that touches dozens of
  files is a merge or a sweep. Its pairs say little about which files belong
  together, and they would crowd out the narrow commits that do.
- Keep the newest commits when the budget is reached, because recent coupling
  is the most relevant to current work.

## Changes made

- Added `Budget::max_commit_paths` (default 50).
- `cochanges` now skips wide commits, stops at the budget, and returns whether
  it left anything out. `refresh` combines that with the git snapshot's own
  truncation.
- Tests:
  - the budget test now asserts truncation instead of an error;
  - new tests for skipping wide commits, for an untruncated narrow history, and
    for keeping the newest commit at the budget.

## Validation

- Command: `cargo test --offline -p middleman-indexer --test graph_refresh`
  - Result: 16 passed on Windows.
- Command: `cargo test --workspace --offline`
  - Result: 127 passed on Windows.
- Command: `cargo clippy --workspace --all-targets --offline -- -D warnings`
  - Result: passed on Windows.
- Command: `cargo fmt --all --check`
  - Result: passed on Windows.
- Workflow check on a Monarch HUB worktree (Laravel and Vue, wide PR merges):
  - `middleman index --full` reported 1,327 files reparsed and
    `history_truncated: true`. Before the change it failed with
    `graph exceeds its cochanges budget`.
  - `middleman prepare` returned a packet whose ranking reasons include
    `co_change`.

## Next step / handoff

If repositories need different thresholds, expose `max_commit_paths` and
`max_cochanges` through `middleman.toml` limits, each with a hard ceiling so a
configuration can't remove the bound.
