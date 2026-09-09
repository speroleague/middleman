# Task: retrieval CLI

## Objective and scope

Commit: `feat(cli): add context retrieval commands`.
Connect prepare, expand, search and explain to bounded indexing, core ranking and
packet rendering. No source execution, network, automatic claims or raw prompt storage.

## Contract and operational impact

Prepare/expand append one PacketPrepared event with candidate IDs, scores, selected
IDs and rendering metrics. Its event ID is the packet ID, printed on stderr.
The event contains neither prompt nor rendered/source text. Explain resolves this
immutable record across sessions. This additive event needs the new binary for
replay; SQLite schema is unchanged. Search/explain open storage read-only.

## Functional design and decisions

- One bounded fresh scan per command, then graph plus accepted durable facts.
  On-disk incremental caching and the separate index command remain future work.
- Indexer owns source/document hint extraction. CLI owns options, effects and
  orchestration; core owns ranking; packet owns budgets and presentation.
- JSON errors have stable codes. Packet stdout remains the budgeted rendered
  packet; stderr carries packet identity and estimated token count as JSON metadata.
- Search returns bounded ranked summaries, with optional type filter. Expand
  resolves an exact stable ID and its one-hop neighborhood. Node explain returns
  current evidence; packet explain returns historical reasons without rescanning.
- Reject empty/oversized requests, invalid IDs/options and missing initialization.
- Preserve active durable invariants as mandatory packet constraints.

```mermaid
flowchart LR
    Scan[Bounded scan] --> Graph
    Graph --> Hints[Document and task hints]
    Hints --> Ranking
    Ranking --> Packet[Budgeted packet]
    Packet --> Event[Prompt-free PacketPrepared event]
    Event --> Explain
```

## Changes made

- `crates/cli/src/retrieval.rs`: four commands, shared loading, format/budget
  options, JSON errors, packet metadata and event appends. `main.rs` dispatches
  commands and bounds configuration reads to 64 KiB.
- `crates/indexer/src/routing.rs`: pure path/title/heading/tag/routing-table hints,
  with one indexed set for recent Git paths instead of repeated history scans.
- `crates/core`: pure explicit dependency/test expansion; additive PacketPrepared
  events project retrieved/expanded IDs without retaining request text.
- `crates/store`: read-only opening; writable recovery is now under the writer
  lock and transactional, preventing partially rebuilt projections.
- CLI child-process tests plus core expansion and read-only store regressions.
  Only dependency change: existing workspace tempfile used by CLI tests.

## Validation

- `cargo test --workspace --offline --quiet`: all 82 tests passed on Windows.
- `cargo clippy --workspace --all-targets --offline -- -D warnings`: passed.
- `cargo fmt --all --check`: passed.
- Rust fixture request selects frontier module, direct test and contract docs;
  a separate routing-table request resolves worker output to the beta module.
  Laravel-style and Elm fixtures produce relevant packets. No fixture source,
  package manager, build script or validation command was executed.
- Separate processes exercise prepare/search/expand/explain, historical packet
  lookup, deletion, invalid options, empty requests and insufficient budgets.
  Failed prepare and read-only commands append no events; stored events contain
  no raw prompt. CIR stdout parses directly, with no extra empty statement.
- No performance benchmark, Unix execution, or long-lived cache validation.

## Next step / handoff

Generated agent guides and task lifecycle follow after this slice.

## Usage and limits

```text
middleman prepare --task "lease renewal" --format json --budget 2000
middleman search "worker output" --type module --limit 5
middleman expand mod:crates/alpha/src/frontier.rs --format cir
middleman explain evt_<ID printed on stderr>
```

Prepare/expand accept CIR, Markdown and JSON. Search/explain return JSON.
Output format and budget default to the existing configuration (currently CIR and
4000 after init), with flags taking precedence. Requests are at most 16 KiB;
budgets at most 100,000 estimated tokens; retrieval records at most 128 candidates.
Packet stdout is budgeted; stderr is a JSON metadata object. Active durable
invariants are mandatory constraints. Source-derived module/symbol/test/document
facts come from the fresh scan; durable domain claims are overlaid.

Expand prioritizes the requested ID, then direct import/dependency/call/test links;
it does not mislabel documentation or ownership links as dependencies. Node explain
returns current evidence and up to 64 adjacent edges with an explicit truncation
flag. Packet explain needs no source scan and returns original IDs/scores/selection,
format, budget, confidence and ranking truncation. It does not reproduce the raw
prompt or original rendered text. Packet identity uses the existing `evt_` ID type.

Old binaries cannot replay the new event variant; upgrade readers together. Existing
logs remain compatible with this binary. Concurrent intervening appends cause a
stale-append error rather than silently recording an explanation against newer state.
Preparing succeeds only after its explanation event commits.

The CLI currently rebuilds its bounded derived graph on every retrieval; it does
not persist observed graph entities or update `status` index counts. The standalone
index command and persistent incremental snapshot integration remain tracked work.
No whole-repository source dump is returned. The fixture routing criterion is met;
the full phase exit also requires generated guides and remaining index integration.
